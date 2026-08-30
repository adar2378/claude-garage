//! SSE subscription to the daemon's `GET /api/events` (port of
//! `tui/lib/api/sse.dart`).
//!
//! The daemon (daemon/src/events.js) sends exactly two named events —
//! `status` with `{id, status, since}` and `sessions` with `{}` — plus
//! `: connected` / `: keepalive` comment lines every 15s.
//!
//! Three pieces, separated so the logic is unit-testable without sockets:
//!  - [`SseParser`]: pure line-based framing (partial lines across chunks,
//!    comments ignored, multi-`data:` joining).
//!  - [`SseReconnectMachine`]: pure reconnect/backoff/poll-fallback state
//!    (500ms → 8s exponential; the 5s sessions-poll fallback is active only
//!    while disconnected — design: "Poll fallback").
//!  - [`SseClient`]: the thin blocking shell that wires the two to a socket
//!    (run it on a `spawn_blocking` task feeding the app event channel).

use std::io::Read;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

/// One dispatched server-sent event.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SseEvent {
    /// Event name (`status` / `sessions` here); `message` when the server
    /// sent no `event:` field, per the SSE spec.
    pub event: String,
    /// Raw data payload (JSON text for the daemon's events).
    pub data: String,
}

/// Incremental line-based SSE parser. Feed decoded chunks in arrival order;
/// each call returns the events completed by that chunk.
#[derive(Default)]
pub struct SseParser {
    partial_line: String,
    event_name: String,
    data_lines: Vec<String>,
}

impl SseParser {
    pub fn new() -> SseParser {
        SseParser::default()
    }

    pub fn add_chunk(&mut self, chunk: &str) -> Vec<SseEvent> {
        let mut events = Vec::new();
        let mut start = 0;
        for (i, byte) in chunk.bytes().enumerate() {
            if byte != b'\n' {
                continue;
            }
            self.partial_line.push_str(&chunk[start..i]);
            start = i + 1;
            let mut line = std::mem::take(&mut self.partial_line);
            // The daemon terminates lines with '\n'; tolerate CRLF too.
            if line.ends_with('\r') {
                line.pop();
            }
            if let Some(event) = self.process_line(&line) {
                events.push(event);
            }
        }
        self.partial_line.push_str(&chunk[start..]);
        events
    }

    fn process_line(&mut self, line: &str) -> Option<SseEvent> {
        if line.is_empty() {
            // Blank line = dispatch. Per spec: no accumulated data → no event
            // (this is how keepalive comments stay invisible).
            if self.data_lines.is_empty() {
                self.event_name.clear();
                return None;
            }
            let event = SseEvent {
                event: if self.event_name.is_empty() {
                    "message".to_owned()
                } else {
                    self.event_name.clone()
                },
                data: self.data_lines.join("\n"),
            };
            self.event_name.clear();
            self.data_lines.clear();
            return Some(event);
        }
        if line.starts_with(':') {
            return None; // comment (keepalive)
        }

        let (field, value) = match line.find(':') {
            None => (line, ""),
            Some(colon) => (&line[..colon], &line[colon + 1..]),
        };
        let value = value.strip_prefix(' ').unwrap_or(value);

        match field {
            "event" => self.event_name = value.to_owned(),
            "data" => self.data_lines.push(value.to_owned()),
            // id/retry/unknown fields — ignored, the daemon sends neither.
            _ => {}
        }
        None
    }
}

/// Pure reconnect state: exponential backoff 500ms → 8s (doubling, capped),
/// reset on a successful connect; the sessions-poll fallback is active
/// exactly while started-but-not-connected.
pub struct SseReconnectMachine {
    pub initial_delay: Duration,
    pub max_delay: Duration,
    started: bool,
    connected: bool,
    attempt: u32,
}

impl Default for SseReconnectMachine {
    fn default() -> SseReconnectMachine {
        SseReconnectMachine::new(Duration::from_millis(500), Duration::from_secs(8))
    }
}

impl SseReconnectMachine {
    pub fn new(initial_delay: Duration, max_delay: Duration) -> SseReconnectMachine {
        SseReconnectMachine {
            initial_delay,
            max_delay,
            started: false,
            connected: false,
            attempt: 0,
        }
    }

    pub fn connected(&self) -> bool {
        self.connected
    }

    /// True exactly while the client should poll `/api/sessions` every 5s
    /// instead of trusting SSE (spec tui-wall: state comes from the daemon
    /// even when the stream is down).
    pub fn poll_fallback_active(&self) -> bool {
        self.started && !self.connected
    }

    pub fn start(&mut self) {
        self.started = true;
    }

    pub fn on_connected(&mut self) {
        self.connected = true;
        self.attempt = 0;
    }

    /// Records a drop and returns how long to wait before the next attempt:
    /// 500ms, 1s, 2s, 4s, 8s, 8s, ...
    pub fn on_disconnected(&mut self) -> Duration {
        self.connected = false;
        // Cap the shift so attempt can't overflow on very long outages.
        let shift = self.attempt.min(30);
        self.attempt += 1;
        let delay_ms = (self.initial_delay.as_millis() as u64) << shift;
        if delay_ms >= self.max_delay.as_millis() as u64 {
            self.max_delay
        } else {
            Duration::from_millis(delay_ms)
        }
    }
}

/// Interval of the sessions-poll fallback while the stream is down.
pub const SSE_POLL_FALLBACK_INTERVAL: Duration = Duration::from_secs(5);

/// Long-lived blocking subscription to `/api/events`. Owns its own agent so
/// the request/response client is never involved; a wedged stream is bounded
/// by the read timeout (the daemon keepalives every 15s, so a 30s read
/// timeout only ever fires on a genuinely dead stream).
pub struct SseClient {
    pub base_url: String,
    pub machine: SseReconnectMachine,
}

impl SseClient {
    pub fn new(base_url: String) -> SseClient {
        SseClient {
            base_url,
            machine: SseReconnectMachine::default(),
        }
    }

    /// Run until `stop` flips. `on_event` receives each dispatched event;
    /// `on_poll_fallback` fires every [`SSE_POLL_FALLBACK_INTERVAL`] while
    /// disconnected — wire it to a sessions refetch.
    pub fn run(
        &mut self,
        stop: Arc<AtomicBool>,
        mut on_event: impl FnMut(SseEvent),
        mut on_poll_fallback: impl FnMut(),
    ) {
        self.machine.start();
        let agent = ureq::AgentBuilder::new()
            .timeout_connect(Duration::from_secs(5))
            .timeout_read(Duration::from_secs(30))
            .build();
        let mut last_poll = Instant::now();

        while !stop.load(Ordering::Relaxed) {
            let connected = agent
                .get(&format!("{}/api/events", self.base_url))
                .set("Accept", "text/event-stream")
                .call();
            if let Ok(response) = connected {
                self.machine.on_connected();
                let mut reader = response.into_reader();
                let mut parser = SseParser::new();
                let mut buf = [0u8; 8192];
                loop {
                    if stop.load(Ordering::Relaxed) {
                        return;
                    }
                    match reader.read(&mut buf) {
                        Ok(0) | Err(_) => break,
                        Ok(n) => {
                            let chunk = String::from_utf8_lossy(&buf[..n]).into_owned();
                            for event in parser.add_chunk(&chunk) {
                                if stop.load(Ordering::Relaxed) {
                                    return;
                                }
                                on_event(event);
                            }
                        }
                    }
                }
            }
            if stop.load(Ordering::Relaxed) {
                return;
            }
            // Connect failure or mid-stream drop — back off, polling while
            // disconnected, in short slices so `stop` stays responsive.
            let delay = self.machine.on_disconnected();
            let deadline = Instant::now() + delay;
            while Instant::now() < deadline && !stop.load(Ordering::Relaxed) {
                if self.machine.poll_fallback_active()
                    && last_poll.elapsed() >= SSE_POLL_FALLBACK_INTERVAL
                {
                    last_poll = Instant::now();
                    on_poll_fallback();
                }
                std::thread::sleep(Duration::from_millis(100));
            }
        }
    }
}

#[cfg(test)]
mod parser_tests {
    //! Port of `tui/test/sse_parser_test.dart` — the daemon's exact wire
    //! shape plus the framing edge cases a chunked socket produces.
    use super::*;

    #[test]
    fn parses_a_named_event_with_json_data_daemon_status_shape() {
        let mut parser = SseParser::new();
        let events = parser.add_chunk(
            "event: status\ndata: {\"id\":\"garage/a/x\",\"status\":\"needs-input\",\"since\":123}\n\n",
        );
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].event, "status");
        assert_eq!(
            events[0].data,
            "{\"id\":\"garage/a/x\",\"status\":\"needs-input\",\"since\":123}"
        );
    }

    #[test]
    fn keepalive_and_connected_comments_produce_no_events() {
        let mut parser = SseParser::new();
        assert!(parser.add_chunk(": connected\n\n").is_empty());
        assert!(parser.add_chunk(": keepalive\n\n").is_empty());
        // A comment between fields of a real event does not disturb it.
        let events = parser.add_chunk("event: sessions\n: keepalive\ndata: {}\n\n");
        assert_eq!(events[0].event, "sessions");
    }

    #[test]
    fn partial_lines_across_chunks_are_reassembled() {
        let mut parser = SseParser::new();
        assert!(parser.add_chunk("event: sta").is_empty());
        assert!(parser.add_chunk("tus\ndata: {\"id\"").is_empty());
        assert!(parser.add_chunk(":1}\n").is_empty());
        let events = parser.add_chunk("\n");
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].event, "status");
        assert_eq!(events[0].data, "{\"id\":1}");
    }

    #[test]
    fn one_chunk_carrying_two_events_yields_both_in_order() {
        let mut parser = SseParser::new();
        let events =
            parser.add_chunk("event: status\ndata: {\"n\":1}\n\nevent: sessions\ndata: {}\n\n");
        let names: Vec<&str> = events.iter().map(|e| e.event.as_str()).collect();
        assert_eq!(names, ["status", "sessions"]);
        assert_eq!(events[0].data, "{\"n\":1}");
    }

    #[test]
    fn an_event_with_no_event_field_dispatches_as_message() {
        let mut parser = SseParser::new();
        let events = parser.add_chunk("data: hello\n\n");
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].event, "message");
        assert_eq!(events[0].data, "hello");
    }

    #[test]
    fn multiple_data_lines_join_with_newline_only_one_leading_space_stripped() {
        let mut parser = SseParser::new();
        let events = parser.add_chunk("data: line1\ndata:  spaced\ndata:\n\n");
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].data, "line1\n spaced\n");
    }

    #[test]
    fn a_blank_line_with_no_data_dispatches_nothing_and_resets_the_event_name() {
        let mut parser = SseParser::new();
        assert!(parser.add_chunk("event: status\n\n").is_empty());
        // The dangling name must not leak into the next event.
        let events = parser.add_chunk("data: x\n\n");
        assert_eq!(events[0].event, "message");
    }

    #[test]
    fn crlf_line_endings_are_tolerated() {
        let mut parser = SseParser::new();
        let events = parser.add_chunk("event: sessions\r\ndata: {}\r\n\r\n");
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].event, "sessions");
        assert_eq!(events[0].data, "{}");
    }
}

#[cfg(test)]
mod reconnect_tests {
    //! Port of `tui/test/sse_reconnect_test.dart`.
    use super::*;

    #[test]
    fn backoff_doubles_from_500ms_and_caps_at_8s() {
        let mut machine = SseReconnectMachine::default();
        machine.start();
        let delays: Vec<Duration> = (0..7).map(|_| machine.on_disconnected()).collect();
        assert_eq!(
            delays,
            [
                Duration::from_millis(500),
                Duration::from_secs(1),
                Duration::from_secs(2),
                Duration::from_secs(4),
                Duration::from_secs(8),
                Duration::from_secs(8),
                Duration::from_secs(8),
            ]
        );
    }

    #[test]
    fn a_successful_connect_resets_the_backoff_to_500ms() {
        let mut machine = SseReconnectMachine::default();
        machine.start();
        machine.on_disconnected();
        machine.on_disconnected();
        assert_eq!(machine.on_disconnected(), Duration::from_secs(2));
        machine.on_connected();
        assert_eq!(machine.on_disconnected(), Duration::from_millis(500));
    }

    #[test]
    fn poll_fallback_is_active_exactly_while_started_but_disconnected() {
        let mut machine = SseReconnectMachine::default();
        assert!(!machine.poll_fallback_active()); // not started yet

        machine.start();
        assert!(machine.poll_fallback_active()); // connecting

        machine.on_connected();
        assert!(!machine.poll_fallback_active()); // stream healthy
        assert!(machine.connected());

        machine.on_disconnected();
        assert!(machine.poll_fallback_active()); // dropped — poll again
        assert!(!machine.connected());

        machine.on_connected();
        assert!(!machine.poll_fallback_active());
    }

    #[test]
    fn backoff_stays_capped_over_a_very_long_outage_no_overflow() {
        let mut machine = SseReconnectMachine::default();
        machine.start();
        let mut last = Duration::ZERO;
        for _ in 0..100 {
            last = machine.on_disconnected();
        }
        assert_eq!(last, Duration::from_secs(8));
    }
}
