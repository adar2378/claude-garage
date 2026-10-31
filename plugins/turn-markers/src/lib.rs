//! turn-markers: one gutter marker per finished agent turn.
//!
//! The plugin never says on which scrollback row a turn ended; it does not
//! know. It pushes `{turn: <at>}` per turn (the `at` of the daemon's
//! `session.turn` event) and the TUI places the marker (see the wall's
//! `ui::turn_rows`). The plugin only keeps the list of turns, so the markers
//! can be drawn again after a daemon restart.

use std::collections::BTreeMap;
use std::path::Path;
use std::time::Duration;

use garage_plugin_sdk::{
    Api, Error, Segment, StateLock, Style, Value, WidgetItem, WidgetPut, read_state_in,
    write_state_in,
};
use serde::{Deserialize, Serialize};

pub const WIDGET: &str = "markers";
const STORE: &str = "turns.json";
/// The gutter slot's limit per session.
pub const MAX_PER_SESSION: usize = 64;
/// The widget's limit of scopes.
pub const MAX_SESSIONS: usize = 256;
/// One column, so the first character is what shows.
const GLYPH: &str = "◆";

#[derive(Debug, Default, Clone, PartialEq, Serialize, Deserialize)]
pub struct Turns {
    /// Session id to the `at` of its turns, oldest first.
    pub sessions: BTreeMap<String, Vec<u64>>,
}

impl Turns {
    pub fn add(&mut self, session: &str, at: u64) {
        let list = self.sessions.entry(session.to_owned()).or_default();
        if list.contains(&at) {
            return; // the same event twice
        }
        list.push(at);
        list.sort_unstable();
        list.dedup();
        if list.len() > MAX_PER_SESSION {
            list.drain(..list.len() - MAX_PER_SESSION);
        }
        // Past the widget's scope limit, forget the session whose newest turn is oldest.
        while self.sessions.len() > MAX_SESSIONS {
            let stale = self
                .sessions
                .iter()
                .min_by_key(|(_, v)| v.last().copied().unwrap_or(0))
                .map(|(k, _)| k.clone());
            match stale {
                Some(k) => self.sessions.remove(&k),
                None => break,
            };
        }
    }

    pub fn close(&mut self, session: &str) {
        self.sessions.remove(session);
    }

    /// The widget body, or `None` when there is nothing to draw.
    pub fn widget(&self) -> Option<WidgetPut> {
        let scopes: BTreeMap<String, Vec<WidgetItem>> = self
            .sessions
            .iter()
            .filter(|(_, v)| !v.is_empty())
            .map(|(id, turns)| {
                let items = turns
                    .iter()
                    .map(|at| WidgetItem {
                        turn: Some(*at),
                        kind: Some("marker".into()),
                        segments: Some(vec![Segment {
                            text: GLYPH.into(),
                            style: Some(Style {
                                fg: Some("accent".into()),
                                ..Style::default()
                            }),
                            click: None,
                        }]),
                        ..WidgetItem::default()
                    })
                    .collect();
                (id.clone(), items)
            })
            .collect();
        (!scopes.is_empty()).then_some(WidgetPut {
            scopes,
            ttl_secs: None,
        })
    }
}

/// What a run was asked to do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Cmd {
    Turn,
    Closed,
    Started,
}

impl Cmd {
    pub fn parse(s: &str) -> Option<Cmd> {
        match s {
            "turn" => Some(Cmd::Turn),
            "closed" => Some(Cmd::Closed),
            "started" => Some(Cmd::Started),
            _ => None,
        }
    }
}

fn push(api: &Api, turns: &Turns) -> Result<(), Error> {
    match turns.widget() {
        Some(put) => api.put_widget(WIDGET, &put).map(|_| ()),
        None => api.clear_widget(WIDGET),
    }
}

/// One run. `context` is the run context (the event rides in `event.data`).
pub fn run(api: &Api, dir: &Path, cmd: Cmd, context: &Value) -> Result<(), Error> {
    let _lock = StateLock::acquire(
        dir,
        "turns",
        Duration::from_secs(20),
        Duration::from_secs(8),
    )?;
    let mut turns: Turns = read_state_in(dir, STORE).unwrap_or_default();
    let data = context.pointer("/event/data");
    let session = data.and_then(|d| d.get("id")).and_then(Value::as_str);
    match cmd {
        Cmd::Turn => {
            let at = data.and_then(|d| d.get("at")).and_then(Value::as_u64);
            let (Some(session), Some(at)) = (session, at) else {
                return Err(Error::Context("session.turn without id and at".into()));
            };
            turns.add(session, at);
        }
        Cmd::Closed => {
            let Some(session) = session else {
                return Err(Error::Context("session.closed without id".into()));
            };
            turns.close(session);
        }
        Cmd::Started => {}
    }
    write_state_in(dir, STORE, &turns)?;
    push(api, &turns)
}

#[cfg(test)]
mod tests {
    use super::*;
    use garage_plugin_sdk::{WidgetSlot, fake::FakeDaemon, json, validate_put};

    const PUT: &str = "/api/plugins/turn-markers/widgets/markers";

    fn event(id: &str, at: u64) -> Value {
        json!({"event": {"name": "session.turn", "data": {"id": id, "workspace": "w", "at": at, "transcript_offset": null}}})
    }

    fn rig(tag: &str) -> (FakeDaemon, Api, std::path::PathBuf) {
        let d = FakeDaemon::start();
        let dir = std::env::temp_dir().join(format!("turn-markers-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let api = Api::new(&d.base, "turn-markers", "tok");
        (d, api, dir)
    }

    #[test]
    fn a_turn_becomes_an_unplaced_marker_addressed_by_its_event_time() {
        let (d, api, dir) = rig("turn");
        run(
            &api,
            &dir,
            Cmd::Turn,
            &event("garage/w/a", 1_800_000_000_123),
        )
        .unwrap();
        let body = &d.bodies("PUT", PUT)[0];
        assert_eq!(
            body,
            &json!({"scopes": {"garage/w/a": [{
                "priority": 0, "placement": {"kind": "flow"},
                "segments": [{"text": "◆", "style": {"fg": "accent"}}],
                "turn": 1_800_000_000_123_u64, "kind": "marker"
            }]}})
        );
        assert!(
            body["scopes"]["garage/w/a"][0].get("row").is_none(),
            "the TUI places it"
        );
    }

    #[test]
    fn markers_accumulate_per_session_and_the_daemon_accepts_the_body() {
        let (d, api, dir) = rig("accumulate");
        for (id, at) in [("garage/w/a", 10), ("garage/w/b", 11), ("garage/w/a", 12)] {
            run(&api, &dir, Cmd::Turn, &event(id, at)).unwrap();
        }
        let last = d.bodies("PUT", PUT).pop().unwrap();
        let put: WidgetPut = serde_json::from_value(last).unwrap();
        let ok = validate_put(WidgetSlot::TileGutter, put).expect("within the gutter limits");
        assert_eq!(ok.scopes["garage/w/a"].len(), 2);
        assert_eq!(ok.scopes["garage/w/b"].len(), 1);
    }

    #[test]
    fn the_same_event_twice_adds_one_marker_and_only_the_newest_64_stay() {
        let mut t = Turns::default();
        t.add("s", 5);
        t.add("s", 5);
        assert_eq!(t.sessions["s"], [5]);
        for at in 0..100 {
            t.add("s", 1000 + at);
        }
        assert_eq!(t.sessions["s"].len(), MAX_PER_SESSION);
        assert_eq!(t.sessions["s"].last(), Some(&1099));
        assert_eq!(t.sessions["s"].first(), Some(&(1099 - 63)));
    }

    #[test]
    fn closing_a_session_drops_its_markers_and_the_last_one_clears_the_widget() {
        let (d, api, dir) = rig("close");
        run(&api, &dir, Cmd::Turn, &event("garage/w/a", 1)).unwrap();
        run(&api, &dir, Cmd::Turn, &event("garage/w/b", 2)).unwrap();
        let closed = json!({"event": {"name": "session.closed", "data": {"id": "garage/w/a"}}});
        run(&api, &dir, Cmd::Closed, &closed).unwrap();
        let body = d.bodies("PUT", PUT).pop().unwrap();
        assert!(body["scopes"].get("garage/w/a").is_none());
        assert!(body["scopes"].get("garage/w/b").is_some());
        let closed_b = json!({"event": {"data": {"id": "garage/w/b"}}});
        run(&api, &dir, Cmd::Closed, &closed_b).unwrap();
        assert!(
            d.seen()
                .iter()
                .any(|s| s.method == "DELETE" && s.path == PUT)
        );
    }

    #[test]
    fn a_daemon_restart_draws_the_stored_markers_again() {
        let (d, api, dir) = rig("started");
        run(&api, &dir, Cmd::Turn, &event("garage/w/a", 7)).unwrap();
        // The daemon restarted: new daemon, same state dir.
        let (d2, api2, _) = rig("started-2");
        run(&api2, &dir, Cmd::Started, &json!({})).unwrap();
        assert_eq!(d2.bodies("PUT", PUT).len(), 1);
        assert_eq!(
            d2.bodies("PUT", PUT)[0]["scopes"]["garage/w/a"][0]["turn"],
            7
        );
        drop(d);
    }

    #[test]
    fn an_event_without_what_it_needs_fails_the_run_and_stores_nothing() {
        let (d, api, dir) = rig("bad");
        assert!(
            run(
                &api,
                &dir,
                Cmd::Turn,
                &json!({"event": {"data": {"id": "x"}}})
            )
            .is_err()
        );
        assert!(run(&api, &dir, Cmd::Closed, &json!({})).is_err());
        assert!(d.seen().is_empty());
        assert!(!dir.join(STORE).exists());
    }

    #[test]
    fn too_many_sessions_forget_the_quietest_one() {
        let mut t = Turns::default();
        for i in 0..MAX_SESSIONS as u64 {
            t.add(&format!("s{i}"), 100 + i);
        }
        t.add("newest", 10_000);
        assert_eq!(t.sessions.len(), MAX_SESSIONS);
        assert!(
            !t.sessions.contains_key("s0"),
            "the one whose last turn is oldest goes"
        );
        assert!(t.sessions.contains_key("newest"));
    }
}
