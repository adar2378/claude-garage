//! HTTP contract of the garage daemon, limited to what the TUI and Claude
//! Code's hooks depend on. Each test runs its own isolated daemon.
//!
//! Deliberately not covered: /api/pick-directory and /api/open-editor (spawn
//! GUI programs), /api/daemon/restart (kills the daemon under test),
//! /api/hooks/install and /api/statusline/install (rewrite Claude settings;
//! covered by daemon unit tests), /api/diff (git plumbing, unit tested),
//! /api/ui/visibility (trivial; notifications are stubbed out instead),
//! PATCH /api/workspaces (rename; unit tested), and plain 400/404 validation.

mod common;

use common::Daemon;
use serde_json::json;
use std::time::Duration;

#[test]
fn health_reports_the_package_version() {
    let d = Daemon::start();
    let (s, b) = d.get("/api/health");
    assert_eq!(s, 200);
    assert_eq!(b["status"], "ok");
    let pkg: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string("../../package.json").unwrap()).unwrap();
    // the launcher uses this to spot a stale daemon after an upgrade
    assert_eq!(b["version"], pkg["version"]);
    assert!(b["pid"].as_u64().unwrap() > 0);
}

#[test]
fn workspace_add_list_delete() {
    let d = Daemon::start();
    let dir = d.mkdir("proj");

    let (s, _) = d.put("/api/workspaces", json!({"name": "Bad Name", "dir": dir}));
    assert_eq!(s, 400, "names must match [a-z0-9-]+");

    d.add_workspace("proj", &dir);
    let (_, list) = d.get("/api/workspaces");
    let ws = list
        .as_array()
        .unwrap()
        .iter()
        .find(|w| w["name"] == "proj")
        .expect("registered workspace is listed");
    assert_eq!(ws["dir"], dir.to_str().unwrap());
    assert!(ws.get("branch").is_some(), "entries carry a branch field");

    let (s, _) = d.delete("/api/workspaces/proj");
    assert_eq!(s, 204);
    let (_, list) = d.get("/api/workspaces");
    assert!(list.as_array().unwrap().iter().all(|w| w["name"] != "proj"));
}

#[test]
fn session_create_list_delete() {
    let d = Daemon::start();
    let dir = d.mkdir("proj");
    d.add_workspace("proj", &dir);

    let created = d.create_session("proj", "one", false);
    let id = created["id"].as_str().unwrap().to_string();
    assert_eq!(id, "garage/proj/one");

    let s = d.session(&id).expect("live session is listed");
    assert_eq!(s["workspace"], "proj");
    assert_eq!(s["label"], "one");
    assert_eq!(s["dir"], dir.to_str().unwrap());
    assert!(s["status"].is_string());
    assert!(s.get("restorable").is_none() || s["restorable"].is_null());

    let (st, _) = d.post("/api/sessions", json!({"workspace": "proj", "label": "one"}));
    assert_eq!(st, 409, "duplicate session");

    let (st, b) = d.delete(&format!("/api/sessions/{id}"));
    assert_eq!(st, 200);
    assert_eq!(b["deleted"], true);
    assert!(d.session(&id).is_none(), "deleted session is gone, not restorable");
    let (st, _) = d.delete(&format!("/api/sessions/{id}"));
    assert_eq!(st, 404);
}

#[test]
fn a_dead_worktree_session_is_restorable_and_restores() {
    let d = Daemon::start();
    let repo = d.git_repo("repo");
    d.add_workspace("repo", &repo);

    // worktree sessions get resume metadata at creation
    let created = d.create_session("repo", "wt", true);
    let id = created["id"].as_str().unwrap().to_string();
    assert!(created["worktree"]["path"].is_string());

    // the session dies behind the daemon's back (crash / reboot)
    assert!(d.tmux(&["kill-session", "-t", &format!("={id}")]));
    let entry = d
        .wait_for(10, || d.session(&id).filter(|s| s["restorable"] == true))
        .expect("dead session shows up as restorable");
    assert_eq!(entry["status"], "restorable");

    let (st, b) = d.post("/api/sessions/restore", json!({"id": id}));
    assert_eq!(st, 201, "{b}");
    assert_eq!(b["restored"][0]["id"], id.as_str());
    assert_eq!(b["restored"][0]["resumed"], false, "no claude session id to resume");
    let live = d.session(&id).expect("restored session is live");
    assert!(live["restorable"].is_null());
}

#[test]
fn restart_keeps_the_session_alive() {
    let d = Daemon::start();
    let dir = d.mkdir("proj");
    d.add_workspace("proj", &dir);
    let id = d.create_session("proj", "r", false)["id"]
        .as_str()
        .unwrap()
        .to_string();

    let (st, b) = d.post("/api/sessions/restart", json!({"id": id, "force": true}));
    assert_eq!(st, 200, "{b}");
    assert_eq!(b["restarted"][0]["id"], id.as_str());
    assert!(b["failed"].as_array().unwrap().is_empty());
    assert!(d.session(&id).is_some(), "same session, new process");

    let (st, _) = d.post("/api/sessions/restart", json!({"id": "garage/proj/nope"}));
    assert_eq!(st, 404);
}

fn collect_until(
    rx: &std::sync::mpsc::Receiver<String>,
    secs: u64,
    pred: impl Fn(&[String]) -> bool,
) -> Vec<String> {
    let mut lines = vec![];
    let deadline = std::time::Instant::now() + Duration::from_secs(secs);
    while std::time::Instant::now() < deadline && !pred(&lines) {
        if let Ok(l) = rx.recv_timeout(Duration::from_millis(200)) {
            lines.push(l);
        }
    }
    lines
}

#[test]
fn hooks_change_status_and_events_announce_it() {
    let d = Daemon::start();
    let dir = d.mkdir("proj");
    d.add_workspace("proj", &dir);
    let rx = d.events();

    let id = d.create_session("proj", "h", false)["id"]
        .as_str()
        .unwrap()
        .to_string();
    let lines = collect_until(&rx, 10, |l| l.iter().any(|x| x == "event: sessions"));
    assert!(
        lines.iter().any(|x| x == "event: sessions"),
        "a session create must arrive on /api/events; got {lines:?}"
    );

    let token = d.hook_token();
    let hook = format!("/api/hooks/claude?token={token}");
    let (st, _) = d.post("/api/hooks/claude?token=wrong", json!({"hook_event_name": "Stop"}));
    assert_eq!(st, 401);

    // cwd fallback resolves the payload to the session in that directory
    let (st, b) = d.post(&hook, json!({"hook_event_name": "Stop", "cwd": dir}));
    assert_eq!(st, 200, "{b}");
    assert_eq!(b["applied"][0], id.as_str());
    assert_eq!(d.session(&id).unwrap()["status"], "done");
    let lines = collect_until(&rx, 10, |l| {
        l.iter().any(|x| x.starts_with("data: ") && x.contains("\"status\":\"done\""))
    });
    assert!(
        lines.iter().any(|x| x == "event: status"),
        "a status change must arrive as a status event; got {lines:?}"
    );
    let data = lines.iter().find(|x| x.contains("\"status\":\"done\"")).unwrap();
    assert!(data.contains(&id) && data.contains("\"since\""));

    // a real blocker carries its message; the idle reminder is ignored
    let (_, _) = d.post(
        &hook,
        json!({"hook_event_name": "Notification", "cwd": dir, "message": "Claude needs your permission to use Bash"}),
    );
    let s = d.session(&id).unwrap();
    assert_eq!(s["status"], "needs-input");
    assert_eq!(s["message"], "Claude needs your permission to use Bash");

    let (_, b) = d.post(
        &hook,
        json!({"hook_event_name": "Notification", "cwd": dir, "message": "Claude is waiting for your input"}),
    );
    assert_eq!(b["ignored"], "idle-reminder");
    assert_eq!(d.session(&id).unwrap()["status"], "needs-input");
}

#[test]
fn statusline_posts_feed_session_context_and_usage() {
    let d = Daemon::start();
    let dir = d.mkdir("proj");
    d.add_workspace("proj", &dir);
    let id = d.create_session("proj", "s", false)["id"]
        .as_str()
        .unwrap()
        .to_string();
    let (_, usage) = d.get("/api/usage");
    assert!(usage["fiveHour"].is_null() && usage["sevenDay"].is_null());

    let token = d.hook_token();
    let url = format!("/api/statusline/claude?token={token}");
    let (st, _) = d.post("/api/statusline/claude?token=wrong", json!({}));
    assert_eq!(st, 401);

    let (st, b) = d.post(
        &url,
        json!({
            "cwd": dir,
            "context_window": {"used_percentage": 42},
            "rate_limits": {
                "five_hour": {"used_percentage": 12.5, "resets_at": 1700000000},
                "seven_day": {"used_percentage": 3}
            }
        }),
    );
    assert_eq!(st, 200, "{b}");
    assert_eq!(b["applied"][0], id.as_str());

    let ctx = &d.session(&id).unwrap()["context"];
    assert_eq!(ctx["usedPercentage"], 42);
    assert_eq!(ctx["source"], "statusline");

    let (_, usage) = d.get("/api/usage");
    assert_eq!(usage["fiveHour"]["usedPercentage"], 12.5);
    assert_eq!(usage["fiveHour"]["resetsAt"], 1700000000);
    assert_eq!(usage["sevenDay"]["usedPercentage"], 3);

    // an unattributable post records nothing
    let (_, b) = d.post(&url, json!({"cwd": "/nonexistent", "rate_limits": {"five_hour": {"used_percentage": 99}}}));
    assert_eq!(b["ignored"], true);
    assert_eq!(d.get("/api/usage").1["fiveHour"]["usedPercentage"], 12.5);
}

#[test]
fn deleting_a_worktree_session_returns_a_record_that_finish_can_discard() {
    let d = Daemon::start();
    let repo = d.git_repo("repo");
    d.add_workspace("repo", &repo);
    let created = d.create_session("repo", "wt", true);
    let id = created["id"].as_str().unwrap();
    let wt_path = created["worktree"]["path"].as_str().unwrap().to_string();
    assert!(std::path::Path::new(&wt_path).is_dir());

    let (st, b) = d.delete(&format!("/api/sessions/{id}"));
    assert_eq!(st, 200);
    let record = b["worktree"].clone();
    assert_eq!(record["path"], wt_path.as_str());
    assert!(record["branch"].as_str().unwrap().starts_with("garage/"));

    // the worktree outlives the session until the user decides
    assert!(std::path::Path::new(&wt_path).is_dir());
    let (st, _) = d.post("/api/worktrees/finish", json!({"worktree": record, "action": "keep"}));
    assert_eq!(st, 200);
    assert!(std::path::Path::new(&wt_path).is_dir());

    let (st, b) = d.post("/api/worktrees/finish", json!({"worktree": record, "action": "discard"}));
    assert_eq!(st, 200, "{b}");
    assert_eq!(b["discarded"], true);
    assert!(!std::path::Path::new(&wt_path).exists());

    let mut bad = record.clone();
    bad["branch"] = json!("main");
    let (st, _) = d.post("/api/worktrees/finish", json!({"worktree": bad, "action": "discard"}));
    assert_eq!(st, 400, "never touches non-garage branches");
}
