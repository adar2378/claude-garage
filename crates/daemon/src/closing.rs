//! Why a session left tmux (p24 `session.closed` reasons).
//!
//! The poller sees a session vanish but not why. Paths that kill a session on
//! purpose leave a short-lived marker here first; the poller takes it when the
//! id disappears. No marker means the session died on its own: `crash`.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// How long a marker waits for the poller before it is forgotten (a kill that
/// never shows up in a poll must not tag a much later, unrelated vanish).
pub const INTENT_TTL: Duration = Duration::from_secs(60);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CloseReason {
    /// Closed through garage (API delete, workspace remove with kill).
    User,
    /// The backend session vanished with no close request.
    Crash,
    /// Closed as part of a restart or stop sequence.
    DaemonRestart,
}

impl CloseReason {
    pub fn as_str(self) -> &'static str {
        match self {
            CloseReason::User => "user",
            CloseReason::Crash => "crash",
            CloseReason::DaemonRestart => "daemon-restart",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "user" => Some(CloseReason::User),
            "crash" => Some(CloseReason::Crash),
            "daemon-restart" => Some(CloseReason::DaemonRestart),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Intent {
    Reason(CloseReason),
    /// The id changed (workspace rename): neither a close nor a create.
    Renamed,
}

#[derive(Clone, Default)]
pub struct ClosingIntents(Arc<Mutex<HashMap<String, (Intent, Instant)>>>);

impl ClosingIntents {
    pub fn mark(&self, id: &str, reason: CloseReason) {
        self.put(id, Intent::Reason(reason));
    }

    /// The id is about to stop existing because it is being renamed.
    pub fn mark_renamed(&self, id: &str) {
        self.put(id, Intent::Renamed);
    }

    fn put(&self, id: &str, intent: Intent) {
        self.0
            .lock()
            .unwrap()
            .insert(id.to_owned(), (intent, Instant::now()));
    }

    /// Forget a marker whose close did not happen (the kill failed).
    pub fn clear(&self, id: &str) {
        self.0.lock().unwrap().remove(id);
    }

    /// Consume the marker for `id`, if one is still fresh.
    pub fn take(&self, id: &str) -> Option<Intent> {
        let mut map = self.0.lock().unwrap();
        map.retain(|_, (_, at)| at.elapsed() < INTENT_TTL);
        map.remove(id).map(|(i, _)| i)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_marker_is_taken_once_and_names_its_reason() {
        let c = ClosingIntents::default();
        assert_eq!(c.take("garage/w/a"), None);
        c.mark("garage/w/a", CloseReason::User);
        assert_eq!(
            c.take("garage/w/a"),
            Some(Intent::Reason(CloseReason::User))
        );
        assert_eq!(c.take("garage/w/a"), None);
    }

    #[test]
    fn reasons_use_the_documented_wire_names() {
        for r in [
            CloseReason::User,
            CloseReason::Crash,
            CloseReason::DaemonRestart,
        ] {
            assert_eq!(CloseReason::parse(r.as_str()), Some(r));
        }
        assert_eq!(CloseReason::DaemonRestart.as_str(), "daemon-restart");
        assert_eq!(CloseReason::parse("other"), None);
    }
}
