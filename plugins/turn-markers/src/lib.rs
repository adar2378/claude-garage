//! turn-markers: one gutter marker per finished agent turn.
//!
//! The daemon anchors each turn: its `session.turn` event carries `anchor.row`, the
//! absolute scrollback row the session's cursor was on when the turn ended
//! (history size plus cursor row, measured by the backend). The plugin keeps
//! the last [`MAX_PER_SESSION`] turns per session and pushes them as
//! `tile.gutter` items `{row}`; the TUI draws each on its row and drops the
//! ones that scrolled out of the kept history. The push is also capped by
//! size ([`BODY_BUDGET`]), newest turns first, so it always fits the widget
//! body limit.

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
/// Turns kept (and drawn) per session.
pub const MAX_PER_SESSION: usize = 32;
/// The widget's limit of scopes.
pub const MAX_SESSIONS: usize = 256;
/// The most bytes a push may take: under the daemon's 16 KiB body limit with
/// room to spare, whatever the number of sessions.
pub const BODY_BUDGET: usize = 12 * 1024 - 512;
/// One column, so the first character is what shows.
const GLYPH: &str = "◆";

/// One finished turn: when (the event's `at`, its identity) and where.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Turn {
    pub at: u64,
    /// Absolute scrollback row of the cursor when the turn ended.
    pub row: u64,
}

#[derive(Debug, Default, Clone, PartialEq, Serialize, Deserialize)]
pub struct Turns {
    /// Session id to its turns, oldest first.
    pub sessions: BTreeMap<String, Vec<Turn>>,
}

fn marker(row: u64) -> WidgetItem {
    WidgetItem::new()
        .row(row)
        .kind("marker")
        .segments(vec![Segment::new(GLYPH).style(Style::new().fg("accent"))])
}

impl Turns {
    pub fn add(&mut self, session: &str, turn: Turn) {
        let list = self.sessions.entry(session.to_owned()).or_default();
        if list.iter().any(|t| t.at == turn.at) {
            return; // the same event twice
        }
        list.push(turn);
        list.sort_unstable_by_key(|t| t.at);
        if list.len() > MAX_PER_SESSION {
            list.drain(..list.len() - MAX_PER_SESSION);
        }
        // Past the widget's scope limit, forget the session whose newest turn is oldest.
        while self.sessions.len() > MAX_SESSIONS {
            let stale = self
                .sessions
                .iter()
                .min_by_key(|(_, v)| v.last().map_or(0, |t| t.at))
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

    /// The widget body, or `None` when there is nothing to draw. Turns are
    /// taken newest first across all sessions until [`BODY_BUDGET`] bytes of
    /// body are used, so a wall full of busy sessions drops the oldest
    /// markers rather than the push.
    pub fn widget(&self) -> Option<WidgetPut> {
        let mut all: Vec<(&str, Turn)> = self
            .sessions
            .iter()
            .flat_map(|(id, v)| v.iter().map(move |t| (id.as_str(), *t)))
            .collect();
        all.sort_unstable_by_key(|(_, t)| std::cmp::Reverse(t.at));
        // `{"scopes":{}}` plus, per scope, `"id":[],` and, per item, its JSON and a comma.
        let mut used = 16;
        let mut scopes: BTreeMap<String, Vec<WidgetItem>> = BTreeMap::new();
        for (id, turn) in all {
            let item = marker(turn.row);
            let item_bytes = serde_json::to_vec(&item).map_or(usize::MAX, |v| v.len()) + 1;
            let scope_bytes = if scopes.contains_key(id) {
                0
            } else {
                id.len() + 6
            };
            if used + item_bytes + scope_bytes > BODY_BUDGET {
                break;
            }
            used += item_bytes + scope_bytes;
            scopes.entry(id.to_owned()).or_default().push(item);
        }
        // Oldest first, like the stored order.
        scopes.values_mut().for_each(|items| items.reverse());
        (!scopes.is_empty()).then(|| WidgetPut::new().scopes(scopes))
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
            // No anchor, or no row in it: the backend could not say where the cursor was, so there
            // is nowhere to draw this turn. Nothing to store, nothing to push.
            let Some(row) = data
                .and_then(|d| d.pointer("/anchor/row"))
                .and_then(Value::as_u64)
            else {
                return Ok(());
            };
            turns.add(session, Turn { at, row });
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

    fn event(id: &str, at: u64, row: u64) -> Value {
        json!({"event": {"name": "session.turn", "data": {"id": id, "workspace": "w", "at": at, "anchor": {"transcript_offset": null, "row": row, "history_limit": 2000}}}})
    }

    fn rig(tag: &str) -> (FakeDaemon, Api, std::path::PathBuf) {
        let d = FakeDaemon::start();
        let dir = std::env::temp_dir().join(format!("turn-markers-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let api = Api::new(&d.base, "turn-markers", "tok");
        (d, api, dir)
    }

    fn t(at: u64, row: u64) -> Turn {
        Turn { at, row }
    }

    #[test]
    fn a_turn_becomes_a_marker_on_the_row_the_daemon_measured() {
        let (d, api, dir) = rig("turn");
        run(
            &api,
            &dir,
            Cmd::Turn,
            &event("garage/w/a", 1_800_000_000_123, 4321),
        )
        .unwrap();
        let body = &d.bodies("PUT", PUT)[0];
        assert_eq!(
            body,
            &json!({"scopes": {"garage/w/a": [{
                "priority": 0, "placement": {"kind": "flow"},
                "segments": [{"text": "◆", "style": {"fg": "accent"}}],
                "row": 4321, "kind": "marker"
            }]}})
        );
    }

    #[test]
    fn a_turn_the_daemon_could_not_anchor_draws_nothing_and_stores_nothing() {
        let (d, api, dir) = rig("norow");
        // no `anchor` at all (nothing measured), an anchor with a null row, and
        // the draft-era flat `row` (no longer part of the event): none draws.
        for data in [
            json!({"id": "garage/w/a", "at": 5}),
            json!({"id": "garage/w/a", "at": 5, "anchor": {"transcript_offset": 9, "row": null, "history_limit": null}}),
            json!({"id": "garage/w/a", "at": 5, "row": 12}),
        ] {
            run(&api, &dir, Cmd::Turn, &json!({"event": {"data": data}})).unwrap();
        }
        assert!(d.seen().is_empty());
        assert!(!dir.join(STORE).exists());
    }

    #[test]
    fn markers_accumulate_per_session_and_the_daemon_accepts_the_body() {
        let (d, api, dir) = rig("accumulate");
        for (id, at, row) in [
            ("garage/w/a", 10, 1),
            ("garage/w/b", 11, 2),
            ("garage/w/a", 12, 3),
        ] {
            run(&api, &dir, Cmd::Turn, &event(id, at, row)).unwrap();
        }
        let last = d.bodies("PUT", PUT).pop().unwrap();
        let put: WidgetPut = serde_json::from_value(last).unwrap();
        let ok = validate_put(WidgetSlot::TileGutter, put).expect("within the gutter limits");
        assert_eq!(ok.scopes["garage/w/a"].len(), 2);
        assert_eq!(ok.scopes["garage/w/b"].len(), 1);
        assert_eq!(
            ok.scopes["garage/w/a"]
                .iter()
                .map(|i| i.row)
                .collect::<Vec<_>>(),
            [Some(1), Some(3)],
            "oldest first"
        );
    }

    #[test]
    fn the_same_event_twice_adds_one_marker_and_only_the_newest_32_stay() {
        let mut turns = Turns::default();
        turns.add("s", t(5, 50));
        turns.add("s", t(5, 50));
        assert_eq!(turns.sessions["s"], [t(5, 50)]);
        for at in 0..100 {
            turns.add("s", t(1000 + at, 10 * at));
        }
        let kept = &turns.sessions["s"];
        assert_eq!(kept.len(), MAX_PER_SESSION);
        assert_eq!(kept.last(), Some(&t(1099, 990)));
        assert_eq!(kept.first(), Some(&t(1099 - 31, 10 * 68)));
    }

    #[test]
    fn closing_a_session_drops_its_markers_and_the_last_one_clears_the_widget() {
        let (d, api, dir) = rig("close");
        run(&api, &dir, Cmd::Turn, &event("garage/w/a", 1, 1)).unwrap();
        run(&api, &dir, Cmd::Turn, &event("garage/w/b", 2, 2)).unwrap();
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
        run(&api, &dir, Cmd::Turn, &event("garage/w/a", 7, 70)).unwrap();
        // The daemon restarted: new daemon, same state dir.
        let (d2, api2, _) = rig("started-2");
        run(&api2, &dir, Cmd::Started, &json!({})).unwrap();
        assert_eq!(d2.bodies("PUT", PUT).len(), 1);
        assert_eq!(
            d2.bodies("PUT", PUT)[0]["scopes"]["garage/w/a"][0]["row"],
            70
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
                &json!({"event": {"data": {"id": "x", "anchor": {"row": 3}}}})
            )
            .is_err()
        );
        assert!(run(&api, &dir, Cmd::Closed, &json!({})).is_err());
        assert!(d.seen().is_empty());
        assert!(!dir.join(STORE).exists());
    }

    #[test]
    fn too_many_sessions_forget_the_quietest_one() {
        let mut turns = Turns::default();
        for i in 0..MAX_SESSIONS as u64 {
            turns.add(&format!("s{i}"), t(100 + i, i));
        }
        turns.add("newest", t(10_000, 1));
        assert_eq!(turns.sessions.len(), MAX_SESSIONS);
        assert!(
            !turns.sessions.contains_key("s0"),
            "the one whose last turn is oldest goes"
        );
        assert!(turns.sessions.contains_key("newest"));
    }

    /// The worst case for size: every session full of turns on long ids and
    /// big row numbers. The push must stay under the budget (and so under the
    /// daemon's body limit), keeping the newest turns.
    #[test]
    fn the_body_is_capped_by_size_and_the_newest_turns_survive() {
        let mut turns = Turns::default();
        let mut at = 1_000_000_000_000;
        for round in 0..MAX_PER_SESSION as u64 {
            for s in 0..40 {
                at += 1;
                turns.add(
                    &format!("garage/a-fairly-long-workspace-name/session-label-{s:02}"),
                    t(at, 4_000_000_000 + round * 7),
                );
            }
        }
        assert_eq!(
            turns.sessions.values().map(Vec::len).sum::<usize>(),
            40 * MAX_PER_SESSION,
            "everything is stored"
        );
        let put = turns.widget().unwrap();
        let bytes = serde_json::to_vec(&put).unwrap().len();
        assert!(bytes <= BODY_BUDGET, "{bytes} bytes");
        assert!(bytes < 12 * 1024);
        assert!(
            bytes > BODY_BUDGET - 400,
            "and it uses the room it has: {bytes}"
        );
        let shown: usize = put.scopes.values().map(Vec::len).sum();
        assert!(shown < 40 * MAX_PER_SESSION, "older turns were left out");
        // The newest turn made it; the very oldest did not.
        let newest_row = 4_000_000_000 + (MAX_PER_SESSION as u64 - 1) * 7;
        assert!(
            put.scopes
                .values()
                .flatten()
                .any(|i| i.row == Some(newest_row))
        );
        assert!(
            !put.scopes
                .values()
                .flatten()
                .any(|i| i.row == Some(4_000_000_000)),
            "the first round, the oldest, is gone"
        );
        validate_put(WidgetSlot::TileGutter, put).expect("the daemon takes it");
    }
}
