//! closed-chats: the daemon keeps a record of every closed session; this
//! plugin shows them as grey rows under their workspace in the rail
//! (`rail.extra_rows`) and offers a recent list on `H`.

use std::collections::BTreeMap;
use std::time::Duration;

use garage_plugin_sdk::tui::{self, Picker};
use garage_plugin_sdk::{Api, Error, Segment, Style, Value, WidgetItem, WidgetPut, json};

/// The rail allows this many extra rows per workspace.
pub const ROWS_PER_WORKSPACE: usize = 8;
/// The widget label budget; the whole item is held to 64 columns.
const LABEL_COLS: usize = 38;

/// One closed-session record (`GET /api/workspaces/{name}/closed`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Closed {
    pub id: String,
    pub workspace: String,
    pub reason: String,
    pub closed_at: i64,
}

impl Closed {
    fn from_json(v: &Value) -> Option<Self> {
        Some(Self {
            id: v.get("id")?.as_str()?.to_owned(),
            workspace: v.get("workspace")?.as_str()?.to_owned(),
            reason: v
                .get("reason")
                .and_then(Value::as_str)
                .unwrap_or("user")
                .to_owned(),
            closed_at: v.get("closedAt").and_then(Value::as_i64).unwrap_or(0),
        })
    }

    /// `label` of `garage/<workspace>/<label>`.
    pub fn label(&self) -> &str {
        self.id.rsplit('/').next().unwrap_or(&self.id)
    }
}

/// How a close is described next to its name.
pub fn reason_hint(reason: &str) -> &'static str {
    match reason {
        "crash" => "crashed",
        "daemon-restart" => "restart",
        _ => "closed",
    }
}

/// "just now", "5m ago", "2h ago", "3d ago".
pub fn age(now_ms: i64, closed_ms: i64) -> String {
    let secs = ((now_ms - closed_ms) / 1000).max(0);
    match secs {
        0..=59 => "just now".to_owned(),
        60..=3599 => format!("{}m ago", secs / 60),
        3600..=86_399 => format!("{}h ago", secs / 3600),
        _ => format!("{}d ago", secs / 86_400),
    }
}

fn truncate(s: &str, cols: usize) -> String {
    if s.chars().count() <= cols {
        return s.to_owned();
    }
    let mut t: String = s.chars().take(cols.saturating_sub(1)).collect();
    t.push('~');
    t
}

fn dim() -> Option<Style> {
    Some(Style {
        fg: Some("dim".into()),
        ..Default::default()
    })
}

fn click(action: &str) -> Option<garage_plugin_sdk::Click> {
    Some(garage_plugin_sdk::Click {
        action: action.to_owned(),
        args: None,
    })
}

/// One grey rail row for a closed chat: the name restores it (the row is
/// about that session, so `session.restore` takes it as its target), the
/// trailing `x` forgets it (a dangerous action: the wall asks twice).
pub fn row(rec: &Closed) -> WidgetItem {
    let name = truncate(rec.label(), LABEL_COLS);
    WidgetItem {
        segments: Some(vec![
            Segment {
                text: format!("  o {name} - {}", reason_hint(&rec.reason)),
                style: dim(),
                click: click("session.restore"),
            },
            Segment {
                text: " x ".to_owned(),
                style: dim(),
                click: click("session.delete"),
            },
        ]),
        session: Some(rec.id.clone()),
        ..Default::default()
    }
}

/// The widget content: per workspace its newest records, newest first.
pub fn put(records: &BTreeMap<String, Vec<Closed>>) -> WidgetPut {
    let scopes = records
        .iter()
        .filter(|(_, r)| !r.is_empty())
        .map(|(ws, recs)| {
            let mut recs: Vec<&Closed> = recs.iter().collect();
            recs.sort_by_key(|a| std::cmp::Reverse(a.closed_at));
            (
                ws.clone(),
                recs.into_iter().take(ROWS_PER_WORKSPACE).map(row).collect(),
            )
        })
        .collect();
    WidgetPut {
        scopes,
        ttl_secs: None,
    }
}

/// Every workspace's closed records.
pub fn fetch(api: &Api) -> Result<BTreeMap<String, Vec<Closed>>, Error> {
    let workspaces = api.get("/api/workspaces")?;
    let mut out = BTreeMap::new();
    for ws in workspaces.as_array().into_iter().flatten() {
        let Some(name) = ws.get("name").and_then(Value::as_str) else {
            continue;
        };
        let recs = match api.get(&format!("/api/workspaces/{name}/closed")) {
            Ok(v) => v,
            // removed between the two calls
            Err(Error::Http { status: 404, .. }) => continue,
            Err(e) => return Err(e),
        };
        out.insert(
            name.to_owned(),
            recs.as_array()
                .into_iter()
                .flatten()
                .filter_map(Closed::from_json)
                .collect(),
        );
    }
    Ok(out)
}

/// Rebuild the rows. With `expect` (the id a `session.closed` event named)
/// the fetch is repeated up to five times until that record exists: the
/// registry writes it concurrently with the event.
pub fn sync(api: &Api, expect: Option<&str>, pause: Duration) -> Result<(), Error> {
    let mut records = fetch(api)?;
    if let Some(id) = expect {
        for _ in 0..5 {
            if records.values().flatten().any(|r| r.id == id) {
                break;
            }
            std::thread::sleep(pause);
            records = fetch(api)?;
        }
    }
    api.put_widget("rows", &put(&records)).map(|_| ())
}

/// The picker lines, newest first, with the matching records.
pub fn recent_items(
    records: &BTreeMap<String, Vec<Closed>>,
    now_ms: i64,
) -> (Vec<String>, Vec<Closed>) {
    let mut all: Vec<Closed> = records.values().flatten().cloned().collect();
    all.sort_by_key(|a| std::cmp::Reverse(a.closed_at));
    let lines = all
        .iter()
        .map(|r| {
            format!(
                "{}/{}  {}  {}",
                r.workspace,
                r.label(),
                reason_hint(&r.reason),
                age(now_ms, r.closed_at)
            )
        })
        .collect();
    (lines, all)
}

/// The `H` pane: choose a closed chat, restore it, exit.
pub fn recent(api: &Api) -> Result<(), Error> {
    let records = fetch(api)?;
    let now = now_ms();
    let (lines, all) = recent_items(&records, now);
    if all.is_empty() {
        let _ = tui::show(
            &[
                "No closed chats".to_owned(),
                String::new(),
                "Press any key".to_owned(),
            ],
            None,
        );
        return Ok(());
    }
    let chosen = tui::run_picker(Picker::new("Restore a closed chat", lines))
        .map_err(|e| Error::Transport(e.to_string()))?;
    if let Some(i) = chosen
        && let Err(e) = restore(api, &all[i].id)
    {
        let _ = tui::show(
            &[
                format!("Could not restore: {e}"),
                "Press any key".to_owned(),
            ],
            None,
        );
        return Err(e);
    }
    Ok(())
}

/// `POST /api/sessions/restore` (a core route).
pub fn restore(api: &Api, id: &str) -> Result<(), Error> {
    api.post("/api/sessions/restore", &json!({"id": id}))
        .map(|_| ())
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use garage_plugin_sdk::testing::FakeDaemon;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn rec(id: &str, reason: &str, at: i64) -> Closed {
        Closed {
            id: id.to_owned(),
            workspace: id.split('/').nth(1).unwrap().to_owned(),
            reason: reason.to_owned(),
            closed_at: at,
        }
    }

    #[test]
    fn a_row_restores_on_the_name_and_forgets_on_the_x_and_is_about_its_session() {
        let item = row(&rec("garage/app/fix-auth", "crash", 5));
        assert_eq!(item.session.as_deref(), Some("garage/app/fix-auth"));
        let segs = item.segments.unwrap();
        assert_eq!(segs[0].text, "  o fix-auth - crashed");
        assert_eq!(segs[0].click.as_ref().unwrap().action, "session.restore");
        assert_eq!(segs[1].click.as_ref().unwrap().action, "session.delete");
        assert_eq!(segs[0].style.as_ref().unwrap().fg.as_deref(), Some("dim"));
    }

    #[test]
    fn rows_keep_the_newest_eight_per_workspace_and_skip_empty_workspaces() {
        let mut map = BTreeMap::new();
        map.insert(
            "app".to_owned(),
            (0..12)
                .map(|i| rec(&format!("garage/app/c{i}"), "user", i))
                .collect(),
        );
        map.insert("empty".to_owned(), vec![]);
        let p = put(&map);
        assert_eq!(p.scopes.keys().collect::<Vec<_>>(), ["app"]);
        let rows = &p.scopes["app"];
        assert_eq!(rows.len(), ROWS_PER_WORKSPACE);
        assert_eq!(
            rows[0].session.as_deref(),
            Some("garage/app/c11"),
            "newest first"
        );
        assert_eq!(rows[7].session.as_deref(), Some("garage/app/c4"));
    }

    #[test]
    fn a_long_name_is_cut_so_the_item_fits_the_column_limit() {
        let long = format!("garage/app/{}", "x".repeat(80));
        let item = row(&rec(&long, "daemon-restart", 1));
        let total: usize = item
            .segments
            .unwrap()
            .iter()
            .map(|s| s.text.chars().count())
            .sum();
        assert!(total <= 64, "{total}");
    }

    #[test]
    fn reasons_and_ages_read_naturally() {
        assert_eq!(reason_hint("user"), "closed");
        assert_eq!(reason_hint("crash"), "crashed");
        assert_eq!(reason_hint("daemon-restart"), "restart");
        assert_eq!(reason_hint("anything-new"), "closed");
        let now = 10_000_000_000;
        assert_eq!(age(now, now - 5_000), "just now");
        assert_eq!(
            age(now, now + 5_000),
            "just now",
            "clock skew never goes negative"
        );
        assert_eq!(age(now, now - 5 * 60_000), "5m ago");
        assert_eq!(age(now, now - 3 * 3_600_000), "3h ago");
        assert_eq!(age(now, now - 2 * 86_400_000), "2d ago");
    }

    #[test]
    fn the_recent_list_is_newest_first_across_workspaces() {
        let mut map = BTreeMap::new();
        map.insert("a".to_owned(), vec![rec("garage/a/old", "user", 1_000)]);
        map.insert("b".to_owned(), vec![rec("garage/b/new", "crash", 900_000)]);
        let (lines, all) = recent_items(&map, 1_000_000);
        assert_eq!(all[0].id, "garage/b/new");
        assert_eq!(lines[0], "b/new  crashed  1m ago");
        assert!(lines[1].starts_with("a/old  closed  16m ago"), "{lines:?}");
    }

    fn daemon_with(closed_after: usize) -> (FakeDaemon, Arc<AtomicUsize>) {
        // the record appears on the Nth fetch of /closed, as a registry write racing the event
        let fetches = Arc::new(AtomicUsize::new(0));
        let f = fetches.clone();
        let d = FakeDaemon::start(move |req| match req.path.as_str() {
            "/api/workspaces" => (200, r#"[{"name":"app","dir":"/a"}]"#.into()),
            "/api/workspaces/app/closed" => {
                let n = f.fetch_add(1, Ordering::SeqCst);
                if n >= closed_after {
                    (200, r#"[{"id":"garage/app/main","workspace":"app","closedAt":5,"reason":"user"}]"#.into())
                } else {
                    (200, "[]".into())
                }
            }
            "/api/plugins/closed-chats/widgets/rows" => (200, r#"{"ok":true,"rev":1}"#.into()),
            _ => (404, "{}".into()),
        });
        (d, fetches)
    }

    #[test]
    fn sync_waits_for_the_record_the_close_event_named_then_pushes_the_rows() {
        let (d, fetches) = daemon_with(2);
        sync(
            &d.api("closed-chats"),
            Some("garage/app/main"),
            Duration::from_millis(1),
        )
        .unwrap();
        assert_eq!(
            fetches.load(Ordering::SeqCst),
            3,
            "two empty fetches, then the record"
        );
        let put = d
            .requests()
            .into_iter()
            .find(|r| r.method == "PUT")
            .unwrap();
        assert_eq!(put.path, "/api/plugins/closed-chats/widgets/rows");
        assert_eq!(put.authorization.as_deref(), Some("Bearer tok"));
        assert_eq!(put.body["scopes"]["app"][0]["session"], "garage/app/main");
        assert_eq!(
            put.body["scopes"]["app"][0]["segments"][0]["click"]["action"],
            "session.restore"
        );
    }

    #[test]
    fn sync_gives_up_waiting_after_five_tries_and_still_pushes() {
        let (d, fetches) = daemon_with(usize::MAX);
        sync(
            &d.api("closed-chats"),
            Some("garage/app/main"),
            Duration::from_millis(1),
        )
        .unwrap();
        assert_eq!(fetches.load(Ordering::SeqCst), 6);
        let put = d
            .requests()
            .into_iter()
            .find(|r| r.method == "PUT")
            .unwrap();
        assert_eq!(
            put.body["scopes"],
            json!({}),
            "no rows, and the widget is emptied"
        );
    }

    #[test]
    fn a_workspace_removed_mid_sync_is_skipped_not_fatal() {
        let d = FakeDaemon::start(|req| match req.path.as_str() {
            "/api/workspaces" => (200, r#"[{"name":"gone"},{"name":"app"}]"#.into()),
            "/api/workspaces/gone/closed" => (404, r#"{"error":"unknown workspace"}"#.into()),
            "/api/workspaces/app/closed" => (200, "[]".into()),
            _ => (200, "{}".into()),
        });
        let got = fetch(&d.api("closed-chats")).unwrap();
        assert_eq!(got.keys().collect::<Vec<_>>(), ["app"]);
    }

    #[test]
    fn restore_posts_the_session_id_to_the_core_route() {
        let d = FakeDaemon::start(|_| (200, "{}".into()));
        restore(&d.api("closed-chats"), "garage/app/main").unwrap();
        let r = &d.requests()[0];
        assert_eq!(
            (r.method.as_str(), r.path.as_str()),
            ("POST", "/api/sessions/restore")
        );
        assert_eq!(r.body, json!({"id": "garage/app/main"}));
    }
}
