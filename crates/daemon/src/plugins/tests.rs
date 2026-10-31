//! Plugin host tests: loading, the run queue, event hooks and the plugin
//! routes. Plugin processes are a recording fake or tiny scripts written into
//! temp dirs; the backend is a fake or an isolated `TestTmux`.

use std::collections::HashMap;
use std::time::Duration;

use axum::http::StatusCode;
#[cfg_attr(not(unix), allow(unused_imports))]
use garage_protocol::widgets::{WidgetPut, WidgetSlot};
use serde_json::{json, Value};

use super::layout::PluginsFile;
use super::rig::{manifest, wait_for, Rig};
use super::runner::RunOutcome;
use super::PluginInfo;
use crate::bus::DaemonEvent;
use crate::closing::CloseReason;

const WIDGETS: &str = "[[widgets]]\nid = \"bar\"\nslot = \"strip.right\"\n";

fn action(id: &str, cmd: &str) -> String {
    format!("[[actions]]\nid = \"{id}\"\ntitle = \"{id}\"\ncommand = [{cmd}]\n")
}

fn hook(on: &str) -> String {
    format!("[[events]]\non = \"{on}\"\ncommand = [\"./hook.sh\"]\n")
}

fn created(id: &str) -> DaemonEvent {
    DaemonEvent::SessionCreated { id: id.into(), dir: "/d".into(), workspace: "w".into() }
}

// ---------------------------------------------------------------------
// Loading
// ---------------------------------------------------------------------

#[tokio::test]
async fn a_first_party_plugin_is_approved_at_first_load_gets_a_private_token_and_announces_once() {
    let mut r = Rig::new();
    r.install_bundled("usage-bar", &manifest("usage-bar", &["ui.widgets"], WIDGETS));
    assert_eq!(
        r.st.plugins.infos(),
        [PluginInfo {
            id: "usage-bar".into(),
            enabled: true,
            active: true,
            version: Some("1.0.0".into()),
            permissions: vec!["ui.widgets".into()],
            problems: vec![]
        }]
    );
    assert_eq!(r.token("usage-bar").len(), 64);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(r.paths().token("usage-bar")).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
    }
    let file = PluginsFile::read(&r.paths()).unwrap();
    assert_eq!(file.plugins[0].granted_permissions.as_deref(), Some(&["ui.widgets".to_string()][..]), "bundled code is approved by shipping");
    assert_eq!(file.plugins[0].manifest_sha256.as_ref().map(String::len), Some(64));
    assert_eq!(r.events().iter().filter(|e| **e == DaemonEvent::PluginsChanged).count(), 1);
    r.st.plugins.reload().unwrap();
    assert!(!r.events().contains(&DaemonEvent::PluginsChanged), "an unchanged reload is silent");
}

#[tokio::test]
async fn a_linked_plugin_nobody_approved_stays_off_until_the_grant_is_written() {
    let mut r = Rig::new();
    let extra = format!("{WIDGETS}{}{}", action("go", "\"./x\""), hook("session.created"));
    r.install_unapproved("dev", &manifest("dev", &["ui.widgets", "sessions.input"], &extra));
    let before = std::fs::read_to_string(r.paths().registry_file()).unwrap();

    let info = r.st.plugins.infos().remove(0);
    assert!(!info.active && info.enabled, "{info:?}");
    assert!(info.problems.iter().any(|p| p.contains("not approved") && p.contains("plugin link")), "{:?}", info.problems);
    assert!(r.st.plugins.active("dev").is_none());
    assert!(!r.paths().token("dev").exists(), "no token before approval");
    let file = PluginsFile::read(&r.paths()).unwrap();
    assert_eq!(file.plugins[0].granted_permissions, None, "loading never grants");
    assert_eq!(file.plugins[0].manifest_sha256, None);
    // however often it reloads, and even when the manifest is rewritten
    r.st.plugins.reload().unwrap();
    r.set_manifest("dev", &manifest("dev", &[], ""));
    r.st.plugins.reload().unwrap();
    assert_eq!(std::fs::read_to_string(r.paths().registry_file()).unwrap(), before, "the registry is untouched");
    assert!(r.st.plugins.active("dev").is_none());

    // nothing it declares does anything: no run, no hook
    r.set_manifest("dev", &manifest("dev", &["ui.widgets"], &extra));
    r.st.plugins.reload().unwrap();
    let _loop = r.st.plugins.start();
    r.events();
    let (s, _, _) = r.call_ui("POST", "/api/plugins/dev/actions/go/run", None).await;
    assert_eq!(s, StatusCode::CONFLICT);
    r.st.bus.publish(created("garage/w/a"));
    tokio::time::sleep(Duration::from_millis(150)).await;
    assert_eq!(r.runner.count(), 0);
    assert!(r.st.plugins.authenticate("").is_none(), "an empty bearer must not match a plugin without a token");

    // `plugin link` records the user's approval; the plugin then loads
    let paths = r.paths();
    PluginsFile::update(&paths, |f| {
        f.plugins[0].granted_permissions = Some(vec!["ui.widgets".into()]);
        (true, ())
    })
    .unwrap();
    r.st.plugins.reload().unwrap();
    let info = r.st.plugins.infos().remove(0);
    assert!(info.active && info.problems.is_empty(), "{info:?}");
    assert_eq!(r.token("dev").len(), 64);
    assert!(r.events().contains(&DaemonEvent::PluginsChanged));
    assert_eq!(r.call_ui("POST", "/api/plugins/dev/actions/go/run", None).await.0, StatusCode::ACCEPTED);
}

#[tokio::test]
async fn a_github_plugin_without_a_grant_is_just_as_inactive_and_bundled_ones_are_the_only_auto_approved() {
    let r = Rig::new();
    let paths = r.paths();
    let dir = paths.installed("fromgh");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("garage-plugin.toml"), manifest("fromgh", &[], "")).unwrap();
    PluginsFile::update(&paths, |f| {
        f.plugins.push(super::layout::Entry::new(
            "fromgh",
            super::layout::Source::Github { repo: "o/r".into(), subdir: None, git_ref: None, commit: None },
        ));
        (true, ())
    })
    .unwrap();
    r.bundle("shipped", &manifest("shipped", &["core.actions"], ""));
    r.link_unapproved("devdir", &manifest("devdir", &[], ""));
    r.st.plugins.reload().unwrap();
    let active: HashMap<String, bool> = r.st.plugins.infos().into_iter().map(|i| (i.id, i.active)).collect();
    assert_eq!(active, HashMap::from([("fromgh".to_string(), false), ("shipped".to_string(), true), ("devdir".to_string(), false)]));
    let file = PluginsFile::read(&paths).unwrap();
    let granted: HashMap<_, _> = file.plugins.iter().map(|e| (e.id.clone(), e.granted_permissions.is_some())).collect();
    assert_eq!(granted, HashMap::from([("fromgh".to_string(), false), ("shipped".to_string(), true), ("devdir".to_string(), false)]));
}

#[tokio::test]
async fn nothing_is_created_on_disk_when_there_is_no_registry() {
    let r = Rig::new();
    r.st.plugins.reload().unwrap();
    assert!(r.st.plugins.infos().is_empty());
    assert!(!r.garage.path().join("plugins").exists());
}

#[tokio::test]
async fn a_corrupt_registry_is_an_error_that_keeps_what_was_loaded_and_is_never_overwritten() {
    let r = Rig::new();
    r.install("pp", &manifest("pp", &[], ""));
    std::fs::write(r.paths().registry_file(), "{broken").unwrap();
    let err = r.st.plugins.reload().unwrap_err();
    assert!(err.contains("plugins.json") && err.contains("corrupt"), "{err}");
    assert_eq!(r.st.plugins.load_error().as_deref(), Some(err.as_str()));
    assert!(r.st.plugins.active("pp").is_some(), "the loaded set survives");
    assert_eq!(std::fs::read_to_string(r.paths().registry_file()).unwrap(), "{broken");
    let (s, _, body) = r.call_ui("POST", "/api/plugins/reload", None).await;
    assert_eq!(s, StatusCode::CONFLICT);
    assert!(body["error"].as_str().unwrap().contains("corrupt"));
}

#[tokio::test]
async fn refused_manifests_stay_inactive_and_say_why_with_the_field_path() {
    let r = Rig::new();
    let bad_action = manifest("aa", &[], "[[actions]]\nid = \"x\"\ntitle = \"X\"\n");
    let new_api = manifest("bb", &[], "").replace("api_version = 1", "api_version = 2");
    let needs = manifest("cc", &[], "requires = [\"warp.drive\"]");
    for (id, toml) in [("aa", bad_action), ("bb", new_api), ("cc", needs)] {
        r.link(id, &toml);
    }
    r.link("dd", &manifest("not-dd", &[], ""));
    r.st.plugins.reload().unwrap();
    let infos: HashMap<String, PluginInfo> = r.st.plugins.infos().into_iter().map(|i| (i.id.clone(), i)).collect();
    for (id, fragment) in [
        ("aa", "actions[0].command: missing"),
        ("bb", "needs a newer garage"),
        ("cc", "needs garage feature \"warp.drive\""),
        ("dd", "manifest says \"not-dd\""),
    ] {
        let i = &infos[id];
        assert!(!i.active && i.version.is_none(), "{id} must not load");
        assert!(i.problems.iter().any(|p| p.contains(fragment)), "{id}: {:?}", i.problems);
        assert!(!r.paths().token(id).exists(), "{id}: no token for a plugin that never loaded");
    }
    assert!(r.st.plugins.active("aa").is_none());
}

#[tokio::test]
async fn a_linked_manifest_that_asks_for_more_keeps_the_last_good_one_and_toasts_once() {
    let mut r = Rig::new();
    let v1 = manifest("lp", &["ui.widgets"], WIDGETS);
    r.install("lp", &v1);
    r.events();

    // 1. a permission the user never approved
    let v2 = manifest("lp", &["ui.widgets", "sessions.input"], WIDGETS);
    r.set_manifest("lp", &v2);
    assert!(r.st.plugins.poll_links(), "the changed file is noticed");
    let want = "plugin lp wants new permissions - run `claude-garage plugin link` to approve";
    let info = r.st.plugins.infos().remove(0);
    assert!(info.active, "the last good manifest keeps running");
    assert_eq!(info.permissions, ["ui.widgets"]);
    assert!(info.problems[0].starts_with(want), "{:?}", info.problems);
    let notices: Vec<_> = r.events().into_iter().filter(|e| matches!(e, DaemonEvent::PluginNotice { .. })).collect();
    assert_eq!(notices.len(), 1);
    assert!(matches!(&notices[0], DaemonEvent::PluginNotice { plugin, message } if plugin == "lp" && message.contains(want)));

    // 2. touched again with the same ask: no second toast
    r.set_manifest("lp", &format!("{v2}# touched\n"));
    assert!(r.st.plugins.poll_links());
    assert!(r.events().iter().all(|e| !matches!(e, DaemonEvent::PluginNotice { .. })));
    assert!(!r.st.plugins.poll_links(), "no change, no reload");

    // 3. a manifest that does not even parse also keeps the old one
    r.set_manifest("lp", "id = ");
    assert!(r.st.plugins.poll_links());
    let info = r.st.plugins.infos().remove(0);
    assert!(info.active && info.problems[0].starts_with("manifest:"), "{info:?}");
    assert!(r.events().iter().any(|e| matches!(e, DaemonEvent::PluginNotice { .. })), "a different problem is worth a toast");

    // 4. the user approves (what `plugin link` will record) and the new
    //    manifest takes effect
    let paths = r.paths();
    let mut file = PluginsFile::read(&paths).unwrap();
    file.plugins[0].granted_permissions = Some(vec!["ui.widgets".into(), "sessions.input".into()]);
    file.write(&paths).unwrap();
    r.set_manifest("lp", &format!("{v2}# approved\n"));
    assert!(r.st.plugins.poll_links());
    let info = r.st.plugins.infos().remove(0);
    assert_eq!((info.active, info.permissions.len(), info.problems.len()), (true, 2, 0), "{info:?}");
}

// ---------------------------------------------------------------------
// Running
// ---------------------------------------------------------------------

#[tokio::test]
async fn an_action_run_gets_substituted_argv_the_env_table_and_the_context() {
    let r = Rig::new();
    r.backend.set("garage/main", "/dev/garage", Some(1), "");
    let dir = r.install(
        "usage-bar",
        &manifest(
            "usage-bar",
            &[],
            &format!("{}timeout_secs = 7\n", action("refresh", "\"{plugin_dir}/bin/usage\", \"refresh\", \"{state_dir}\", \"$(x)\"")),
        ),
    );
    let (s, _, _) = r.call_ui("PUT", "/api/plugins/ui/slots", Some(r#"{"strip.right":{"width":38}}"#)).await;
    assert_eq!(s, StatusCode::OK);
    let body = json!({
        "trigger": {"kind": "key", "key": "u", "args": {"window": "5h"}},
        "focus": {"workspace": "garage", "session": {"id": "garage/main", "dir": "/spoofed"}},
        "selection": {"session": "garage/main"}
    });
    let (s, _, res) = r.call_ui("POST", "/api/plugins/usage-bar/actions/refresh/run", Some(&body.to_string())).await;
    assert_eq!(s, StatusCode::ACCEPTED);
    assert!(res["run_id"].as_str().is_some_and(|id| id.parse::<u64>().is_ok()));
    wait_for(|| r.runner.count() == 1).await;
    let call = r.runner.call(0);

    let data = r.paths().data("usage-bar");
    assert_eq!(
        call.spec.argv,
        [format!("{}/bin/usage", dir.display()), "refresh".into(), data.display().to_string(), "$(x)".into()],
        "no shell: substitution only, `$(x)` stays literal"
    );
    assert_eq!(call.spec.cwd, dir);
    assert_eq!(call.spec.timeout, Some(Duration::from_secs(7)));
    assert!(data.is_dir(), "the state dir exists before the first run");
    assert_eq!(call.env("GARAGE_API"), Some("http://127.0.0.1:4747"));
    assert_eq!(call.env("GARAGE_API_VERSION"), Some("1"));
    assert_eq!(call.env("GARAGE_PLUGIN_ID"), Some("usage-bar"));
    assert_eq!(call.env("GARAGE_PLUGIN_VERSION"), Some("1.0.0"));
    assert_eq!(call.env("GARAGE_PLUGIN_DIR"), Some(dir.to_str().unwrap()));
    assert_eq!(call.env("GARAGE_PLUGIN_STATE_DIR"), Some(data.to_str().unwrap()));
    assert_eq!(call.env("GARAGE_PLUGIN_TOKEN"), Some(r.token("usage-bar").as_str()));
    assert_eq!(call.env("GARAGE_TRIGGER"), Some("key"));
    assert!(call.env("GARAGE_CONTEXT_FILE").is_none());

    let ctx = call.ctx();
    assert_eq!(serde_json::from_str::<Value>(call.env("GARAGE_CONTEXT").unwrap()).unwrap(), ctx, "env and stdin agree");
    assert_eq!(ctx["focus"]["session"]["id"], "garage/main");
    assert_eq!(ctx["focus"]["session"]["dir"], "/dev/garage", "the daemon recomputes dir from its own state");
    assert_eq!(ctx["focus"]["session"]["status"], "idle");
    assert_eq!(ctx["focus"]["workspace"], "garage");
    assert_eq!(ctx["trigger"]["action"], "usage-bar:refresh");
    assert_eq!(ctx["trigger"]["args"], json!({"window": "5h"}));
    assert_eq!(ctx["ui"]["slots"]["strip.right"]["width"], 38);
    assert_eq!(ctx["plugin"]["version"], "1.0.0");
}

#[tokio::test]
async fn a_context_over_32_kib_travels_in_a_file_that_is_removed_afterwards() {
    let r = Rig::new();
    r.install("big", &manifest("big", &[], &action("go", "\"./x\"")));
    let body = json!({"selection": {"view": "v".repeat(40_000)}});
    r.call_ui("POST", "/api/plugins/big/actions/go/run", Some(&body.to_string())).await;
    wait_for(|| r.runner.count() == 1).await;
    let call = r.runner.call(0);
    assert!(call.env("GARAGE_CONTEXT").is_none(), "the env var is unset when the file is used");
    let path = call.env("GARAGE_CONTEXT_FILE").expect("file path").to_owned();
    let from_file: Value = serde_json::from_str(call.ctx_file.as_deref().expect("file existed during the run")).unwrap();
    assert_eq!(from_file, call.ctx());
    assert_eq!(from_file["selection"]["view"].as_str().unwrap().len(), 40_000);
    #[cfg(unix)]
    assert_eq!(call.ctx_file_mode, Some(0o600), "the context names sessions and paths: owner only");
    wait_for(|| !std::path::Path::new(&path).exists()).await;
}

#[tokio::test]
async fn failures_and_timeouts_publish_run_failed_and_unusable_targets_are_refused() {
    let mut r = Rig::new();
    let extra = format!("{}{}", action("go", "\"./x\""), "[[panes]]\nid=\"p\"\ntitle=\"P\"\nplacement=\"overlay\"\ncommand=[\"./ui\"]\n[[actions]]\nid=\"open\"\ntitle=\"Open\"\npane=\"p\"\n");
    r.install("pl", &manifest("pl", &["ui.panes"], &extra));
    r.runner.outcomes.lock().unwrap().extend([RunOutcome::Failed(Some(3)), RunOutcome::TimedOut(Duration::from_secs(30))]);
    for _ in 0..2 {
        assert_eq!(r.call_ui("POST", "/api/plugins/pl/actions/go/run", None).await.0, StatusCode::ACCEPTED);
    }
    let mut reasons = vec![];
    for _ in 0..2 {
        let ev = r.next_event(|e| matches!(e, DaemonEvent::PluginRunFailed { .. })).await;
        if let DaemonEvent::PluginRunFailed { plugin, action, reason } = ev {
            assert_eq!((plugin.as_str(), action.as_str()), ("pl", "pl:go"));
            reasons.push(reason);
        }
    }
    reasons.sort();
    assert_eq!(reasons, ["exited with code 3", "timed out after 30s"]);

    assert_eq!(r.call_ui("POST", "/api/plugins/pl/actions/nope/run", None).await.0, StatusCode::NOT_FOUND);
    assert_eq!(r.call_ui("POST", "/api/plugins/zz/actions/go/run", None).await.0, StatusCode::NOT_FOUND);
    assert_eq!(r.call_ui("POST", "/api/plugins/pl/actions/open/run", None).await.0, StatusCode::NOT_IMPLEMENTED);
    r.st.plugins.set_enabled("pl", false).unwrap();
    let (s, _, body) = r.call_ui("POST", "/api/plugins/pl/actions/go/run", None).await;
    assert_eq!((s, body["error"].as_str().unwrap()), (StatusCode::CONFLICT, "plugin pl is disabled"));
}

#[tokio::test]
async fn at_most_four_runs_execute_at_once_and_a_full_queue_drops_the_oldest() {
    let r = Rig::new();
    r.install("busy", &manifest("busy", &[], &action("go", "\"./x\"")));
    let gate = r.runner.hold();
    let mut ids = vec![];
    for _ in 0..71 {
        ids.push(r.st.plugins.run_action("busy", "go", &json!({})).await.unwrap());
    }
    wait_for(|| r.runner.count() == 4).await;
    assert_eq!(r.st.plugins.queue_len("busy"), 64, "67 waiting, 3 oldest dropped");
    gate.add_permits(100);
    wait_for(|| r.runner.count() == 68).await;
    assert_eq!(r.runner.max_active.load(std::sync::atomic::Ordering::SeqCst), 4);
    let ran: Vec<u64> = (0..68).map(|i| r.runner.call(i).spec.run_id).collect();
    for dropped in &ids[4..7] {
        assert!(!ran.contains(dropped), "run {dropped} was the oldest queued and must be dropped");
    }
    assert!(ids[..4].iter().chain(&ids[7..]).all(|id| ran.contains(id)));
    let log = r.paths().log("busy");
    wait_for(|| std::fs::read_to_string(&log).is_ok_and(|t| t.matches("dropped this run").count() == 3)).await;
}

#[tokio::test]
async fn five_failed_hook_runs_in_a_row_switch_hooks_off_until_reload_but_not_actions() {
    let mut r = Rig::new();
    r.install("flaky", &manifest("flaky", &[], &format!("{}{}", hook("session.created"), action("go", "\"./x\""))));
    let _loop = r.st.plugins.start();
    r.events();
    r.runner.outcomes.lock().unwrap().extend(std::iter::repeat_n(RunOutcome::Failed(Some(1)), 5));
    for i in 0..5 {
        r.st.bus.publish(created(&format!("garage/w/s{i}")));
    }
    let notice = r.next_event(|e| matches!(e, DaemonEvent::PluginNotice { .. })).await;
    assert!(matches!(&notice, DaemonEvent::PluginNotice { plugin, message } if plugin == "flaky" && message.contains("failed 5 times")), "{notice:?}");
    assert!(r.st.plugins.events_disabled("flaky"));
    let before = r.runner.count();
    r.st.bus.publish(created("garage/w/later"));
    tokio::time::sleep(Duration::from_millis(150)).await;
    assert_eq!(r.runner.count(), before, "hooks are off");
    r.st.plugins.run_action("flaky", "go", &json!({})).await.unwrap();
    wait_for(|| r.runner.count() == before + 1).await;

    r.st.plugins.reload().unwrap();
    assert!(!r.st.plugins.events_disabled("flaky"));
    r.st.bus.publish(created("garage/w/again"));
    wait_for(|| r.runner.count() == before + 2).await;
}

#[tokio::test]
async fn a_success_resets_the_failure_streak() {
    let r = Rig::new();
    r.install("mostly", &manifest("mostly", &[], &hook("session.created")));
    let _loop = r.st.plugins.start();
    let outcomes = [vec![RunOutcome::Failed(Some(1)); 4], vec![RunOutcome::Success], vec![RunOutcome::Failed(Some(1)); 4]].concat();
    r.runner.outcomes.lock().unwrap().extend(outcomes);
    for i in 0..9 {
        r.st.bus.publish(created(&format!("garage/w/s{i}")));
        wait_for(|| r.runner.count() == i + 1).await;
    }
    assert!(!r.st.plugins.events_disabled("mostly"), "4 + ok + 4 never reaches 5 in a row");
}

#[tokio::test]
async fn a_timer_hook_runs_on_its_interval_and_skips_ticks_while_the_previous_run_is_going() {
    let mut r = Rig::new();
    r.fast_timers(Duration::from_millis(20)); // every_secs = 5 -> 100 ms
    r.install("ticker", &manifest("ticker", &[], "[[events]]\non = \"timer\"\nevery_secs = 5\ncommand = [\"./tick.sh\"]\n"));
    let gate = r.runner.hold();
    wait_for(|| r.runner.count() == 1).await;
    tokio::time::sleep(Duration::from_millis(450)).await;
    assert_eq!(r.runner.count(), 1, "ticks during a running run are skipped, not queued");
    assert_eq!(r.st.plugins.queue_len("ticker"), 0);
    gate.add_permits(1);
    wait_for(|| r.runner.count() == 2).await;
    let ctx = r.runner.call(0).ctx();
    assert_eq!((ctx["trigger"]["kind"].as_str(), ctx["event"]["name"].as_str()), (Some("timer"), Some("timer")));
    assert_eq!(r.runner.call(0).env("GARAGE_TRIGGER"), Some("timer"));
    // disabling stops the timer for good
    gate.add_permits(100);
    r.st.plugins.set_enabled("ticker", false).unwrap();
    let n = r.runner.count();
    tokio::time::sleep(Duration::from_millis(350)).await;
    assert_eq!(r.runner.count(), n);
}

#[tokio::test]
async fn session_closed_reaches_hooks_once_with_the_real_reason() {
    let r = Rig::new();
    r.install("closer", &manifest("closer", &[], &hook("session.closed")));
    let _loop = r.st.plugins.start();
    let poller = r.st.shared_poller();
    r.backend.set("garage/ws/user", "/proj", Some(1), "");
    r.backend.set("garage/ws/crash", "/proj2", Some(2), "");
    r.backend.set("garage/ws/restart", "/proj3", Some(3), "");
    poller.poll_once().await;
    r.st.closing.mark("garage/ws/user", CloseReason::User);
    r.st.closing.mark("garage/ws/restart", CloseReason::DaemonRestart);
    for id in ["garage/ws/user", "garage/ws/crash", "garage/ws/restart"] {
        r.backend.remove(id);
    }
    r.backend.set("garage/ws/alive", "/alive", Some(4), "");
    poller.poll_once().await;
    wait_for(|| r.runner.count() == 3).await;
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(r.runner.count(), 3, "once per closed session, none for the survivor");
    let mut got: Vec<(String, String, String, String)> = (0..3)
        .map(|i| {
            let c = r.runner.call(i);
            let ctx = c.ctx();
            assert_eq!((ctx["trigger"]["kind"].as_str(), c.env("GARAGE_TRIGGER")), (Some("event"), Some("event")));
            assert_eq!(ctx["event"]["name"], "session.closed");
            let d = &ctx["event"]["data"];
            let s = |v: &Value| v.as_str().unwrap().to_owned();
            assert_eq!(ctx["focus"]["session"]["id"], d["id"], "the closed session is the focus");
            (s(&d["id"]), s(&d["reason"]), s(&d["workspace"]), s(&ctx["focus"]["session"]["dir"]))
        })
        .collect();
    got.sort();
    assert_eq!(
        got,
        [
            ("garage/ws/crash".into(), "crash".into(), "ws".into(), "/proj2".into()),
            ("garage/ws/restart".into(), "daemon-restart".into(), "ws".into(), "/proj3".into()),
            ("garage/ws/user".into(), "user".into(), "ws".into(), "/proj".into()),
        ]
    );
}

#[tokio::test]
async fn the_stop_hook_becomes_a_session_turn_with_the_transcript_offset() {
    let r = Rig::new();
    r.backend.set("garage/ws/a", "/proj", Some(1), "");
    r.install("turns", &manifest("turns", &[], &hook("session.turn")));
    let _loop = r.st.plugins.start();
    let transcript = r.st.telemetry.claude_home.join("projects").join("t.jsonl");
    std::fs::create_dir_all(transcript.parent().unwrap()).unwrap();
    std::fs::write(&transcript, vec![b'x'; 123]).unwrap();
    // a file outside Claude's home must never be stat'ed, nor reached through `..` or a symlink
    let outside = r.src.path().join("secret.txt");
    std::fs::write(&outside, vec![b'y'; 77]).unwrap();
    let dotdot = r.st.telemetry.claude_home.join("projects").join("..").join("..").join(r.src.path().file_name().unwrap()).join("secret.txt");
    #[cfg(unix)]
    let link = {
        let l = r.st.telemetry.claude_home.join("projects").join("link.jsonl");
        std::os::unix::fs::symlink(&outside, &l).unwrap();
        l
    };
    let token = r.st.registry.get_hook_token().await.unwrap();
    let url = format!("/api/hooks/claude?token={token}");

    let stop = json!({"hook_event_name": "Stop", "cwd": "/proj", "transcript_path": transcript});
    assert_eq!(r.call("POST", &url, None, Some(&stop.to_string())).await.0, StatusCode::OK);
    wait_for(|| r.runner.count() == 1).await;
    let ctx = r.runner.call(0).ctx();
    assert_eq!(ctx["event"]["name"], "session.turn");
    let d = &ctx["event"]["data"];
    assert_eq!((d["id"].as_str(), d["workspace"].as_str(), d["transcript_offset"].as_u64()), (Some("garage/ws/a"), Some("ws"), Some(123)));
    assert!(d["at"].as_i64().is_some_and(|at| at > 1_600_000_000_000));

    let bare = json!({"hook_event_name": "Stop", "cwd": "/proj"});
    r.call("POST", &url, None, Some(&bare.to_string())).await;
    wait_for(|| r.runner.count() == 2).await;
    assert_eq!(r.runner.call(1).ctx()["event"]["data"]["transcript_offset"], Value::Null);

    #[cfg_attr(not(unix), allow(unused_mut))]
    let mut outside_paths = vec![outside.to_string_lossy().into_owned(), dotdot.to_string_lossy().into_owned(), "/etc/hosts".to_owned()];
    #[cfg(unix)]
    outside_paths.push(link.to_string_lossy().into_owned());
    for (i, p) in outside_paths.iter().enumerate() {
        let stop = json!({"hook_event_name": "Stop", "cwd": "/proj", "transcript_path": p});
        r.call("POST", &url, None, Some(&stop.to_string())).await;
        wait_for(|| r.runner.count() == 3 + i).await;
        assert_eq!(r.runner.call(2 + i).ctx()["event"]["data"]["transcript_offset"], Value::Null, "{p} is outside Claude's home");
    }
    let total = 2 + outside_paths.len();

    let note = json!({"hook_event_name": "Notification", "cwd": "/proj", "message": "needs you"});
    r.call("POST", &url, None, Some(&note.to_string())).await;
    tokio::time::sleep(Duration::from_millis(150)).await;
    assert_eq!(r.runner.count(), total, "only Stop ends a turn");
}

#[tokio::test]
async fn workspace_added_and_removed_events_fire_once_per_change() {
    let mut r = Rig::new();
    let dir = r.src.path().join("repo");
    std::fs::create_dir_all(&dir).unwrap();
    let put = json!({"name": "repo", "dir": dir}).to_string();
    r.call("PUT", "/api/workspaces", None, Some(&put)).await;
    r.call("PUT", "/api/workspaces", None, Some(&put)).await; // re-point: not an add
    assert_eq!(r.call("DELETE", "/api/workspaces/repo", None, None).await.0, StatusCode::NO_CONTENT);
    let names: Vec<_> = r
        .events()
        .into_iter()
        .filter_map(|e| match e {
            DaemonEvent::WorkspaceAdded { name } => Some(format!("added {name}")),
            DaemonEvent::WorkspaceRemoved { name } => Some(format!("removed {name}")),
            _ => None,
        })
        .collect();
    assert_eq!(names, ["added repo", "removed repo"]);
}

#[tokio::test]
async fn a_status_hook_gets_the_workspace_derived_from_the_session_id() {
    let r = Rig::new();
    r.install("watch", &manifest("watch", &[], &hook("session.status_changed")));
    let _loop = r.st.plugins.start();
    r.st.status.set("garage/ws/a", garage_protocol::SessionStatus::Working, None);
    wait_for(|| r.runner.count() == 1).await;
    let ctx = r.runner.call(0).ctx();
    assert_eq!((ctx["focus"]["workspace"].as_str(), ctx["event"]["data"]["to"].as_str()), (Some("ws"), Some("working")));
}

// ---------------------------------------------------------------------
// Real processes
// ---------------------------------------------------------------------

#[cfg(unix)]
fn write_script(dir: &std::path::Path, name: &str, body: &str) {
    use std::os::unix::fs::PermissionsExt;
    let p = dir.join(name);
    std::fs::write(&p, format!("#!/bin/sh\n{body}\n")).unwrap();
    std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
}

#[cfg(unix)]
#[tokio::test]
async fn a_real_script_runs_through_the_route_with_context_env_and_token() {
    let r = Rig::real();
    let dir = r.link("real", &manifest("real", &[], &action("go", "\"sh\", \"./go.sh\", \"arg one\"")));
    write_script(
        &dir,
        "go.sh",
        "cat > \"$GARAGE_PLUGIN_STATE_DIR/stdin.json\"\nprintf '%s|%s|%s|%s' \"$GARAGE_PLUGIN_ID\" \"$GARAGE_TRIGGER\" \"$GARAGE_API\" \"$1\" > \"$GARAGE_PLUGIN_STATE_DIR/env.txt\"",
    );
    r.st.plugins.reload().unwrap();
    let body = json!({"trigger": {"kind": "palette"}, "focus": {"workspace": "w"}});
    assert_eq!(r.call_ui("POST", "/api/plugins/real/actions/go/run", Some(&body.to_string())).await.0, StatusCode::ACCEPTED);
    let data = r.paths().data("real");
    wait_for(|| data.join("env.txt").exists()).await;
    assert_eq!(std::fs::read_to_string(data.join("env.txt")).unwrap(), "real|palette|http://127.0.0.1:4747|arg one");
    let stdin: Value = serde_json::from_str(&std::fs::read_to_string(data.join("stdin.json")).unwrap()).unwrap();
    assert_eq!(stdin["focus"]["workspace"], "w");
    let log = r.paths().log("real");
    assert!(!log.exists() || !std::fs::read_to_string(log).unwrap().contains("run failed"));
}

#[cfg(unix)]
#[tokio::test]
async fn disabling_kills_running_trees_clears_widgets_and_refuses_new_triggers() {
    let mut r = Rig::real();
    let dir = r.link("sleepy", &manifest("sleepy", &["ui.widgets"], &format!("{WIDGETS}{}", action("long", "\"sh\", \"./long.sh\""))));
    write_script(&dir, "long.sh", "sleep 60 &\necho $! > \"$GARAGE_PLUGIN_STATE_DIR/child.pid\"\nwait");
    r.st.plugins.reload().unwrap();
    let item = garage_protocol::widgets::WidgetItem { segments: Some(vec![]), ..Default::default() };
    let put = WidgetPut { scopes: [("*".to_owned(), vec![item])].into(), ttl_secs: None };
    r.st.plugins.widgets().put("sleepy", "bar", WidgetSlot::StripRight, put);
    r.events();

    r.call_ui("POST", "/api/plugins/sleepy/actions/long/run", None).await;
    let pidfile = r.paths().data("sleepy").join("child.pid");
    wait_for(|| r.st.plugins.running_count("sleepy") == 1 && pidfile.exists()).await;
    let child: i32 = std::fs::read_to_string(&pidfile).unwrap().trim().parse().unwrap();

    r.st.plugins.set_enabled("sleepy", false).unwrap();
    wait_for(|| r.st.plugins.running_count("sleepy") == 0).await;
    wait_for(|| unsafe { libc::kill(child, 0) } != 0).await; // the grandchild died with the group
    assert!(r.st.plugins.widgets().widgets("sleepy").is_empty());
    let seen = r.events();
    assert!(seen.contains(&DaemonEvent::PluginsChanged));
    // the clear is announced too (coalesced, so it may trail by up to 200 ms)
    if !seen.iter().any(|e| matches!(e, DaemonEvent::WidgetsChanged { plugin, .. } if plugin == "sleepy")) {
        r.next_event(|e| matches!(e, DaemonEvent::WidgetsChanged { plugin, .. } if plugin == "sleepy")).await;
    }
    assert_eq!(r.call_ui("POST", "/api/plugins/sleepy/actions/long/run", None).await.0, StatusCode::CONFLICT);
    assert!(!PluginsFile::read(&r.paths()).unwrap().plugins[0].enabled, "persisted");

    r.st.plugins.set_enabled("sleepy", true).unwrap();
    assert!(r.st.plugins.active("sleepy").is_some());
}

// ---------------------------------------------------------------------
// Routes
// ---------------------------------------------------------------------

#[tokio::test]
async fn features_are_public_and_match_the_single_feature_table() {
    let r = Rig::new();
    let (s, _, body) = r.call("GET", "/api/plugin/features", None, None).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(body, garage_plugin::features::features_response(env!("CARGO_PKG_VERSION")));
    assert_eq!(body["api_versions"], json!([1]));
}

async fn pw(r: &Rig, plugin: &str, token: Option<&str>, body: &str) -> (StatusCode, axum::http::HeaderMap, Value) {
    r.call("PUT", &format!("/api/plugins/{plugin}/widgets/bar"), token, Some(body)).await
}

fn put_body(text: &str) -> String {
    json!({"scopes": {"*": [{"segments": [{"text": text}]}]}}).to_string()
}

#[tokio::test]
async fn widget_puts_check_the_token_the_permission_the_declaration_and_the_limits() {
    let mut r = Rig::new();
    let extra = format!("{WIDGETS}{}", action("go", "\"./x\""));
    r.install("wid", &manifest("wid", &["ui.widgets"], &extra));
    r.install("wcore", &manifest("wcore", &["ui.widgets", "core.actions"], &extra));
    r.install("reader", &manifest("reader", &["sessions.read"], ""));
    let (tw, twc, tr) = (r.token("wid"), r.token("wcore"), r.token("reader"));
    let url = |p: &str| format!("/api/plugins/{p}/widgets/bar");
    r.events();

    // who may push
    assert_eq!(pw(&r, "wid", None, &put_body("a")).await.0, StatusCode::UNAUTHORIZED);
    assert_eq!(pw(&r, "wid", Some("nope"), &put_body("a")).await.0, StatusCode::UNAUTHORIZED);
    let (s, _, b) = pw(&r, "wid", Some(&twc), &put_body("a")).await;
    assert_eq!(s, StatusCode::FORBIDDEN);
    assert!(b["error"].as_str().unwrap().contains("belongs to plugin wcore"), "{b}");
    let (s, _, b) = pw(&r, "reader", Some(&tr), &put_body("a")).await;
    assert_eq!(s, StatusCode::FORBIDDEN);
    assert_eq!(b["error"], "plugin reader lacks permission ui.widgets");

    // a push replaces content and reports the rev; equal content is a no-op
    let (s, _, b) = pw(&r, "wid", Some(&tw), &put_body("a")).await;
    assert_eq!((s, b["rev"].as_u64()), (StatusCode::OK, Some(1)));
    assert_eq!(pw(&r, "wid", Some(&tw), &put_body("a")).await.2["rev"], 1, "equal content does not bump the rev");
    assert_eq!(pw(&r, "wid", Some(&tw), &put_body("b")).await.2["rev"], 2);
    assert_eq!(r.st.plugins.widgets().rev("wid"), 2);
    let (s, _, _) = r.call("PUT", "/api/plugins/wid/widgets/nope", Some(&tw), Some(&put_body("a"))).await;
    assert_eq!(s, StatusCode::NOT_FOUND, "undeclared widget");

    // limits name themselves
    let fat = json!({"scopes": {"*": [{"segments": [{"text": "x".repeat(17_000)}]}]}}).to_string();
    let (s, _, b) = pw(&r, "wid", Some(&tw), &fat).await;
    assert_eq!(s, StatusCode::PAYLOAD_TOO_LARGE);
    assert!(b["error"].as_str().unwrap().contains("16384"), "{b}");
    let many = json!({"scopes": {"*": vec![json!({"segments": []}); 17]}}).to_string();
    let (s, _, b) = pw(&r, "wid", Some(&tw), &many).await;
    assert_eq!(s, StatusCode::PAYLOAD_TOO_LARGE);
    assert!(b["error"].as_str().unwrap().contains("16 items per scope"), "{b}");
    let wide = json!({"scopes": {"*": [{"segments": [{"text": "x".repeat(65)}]}]}}).to_string();
    assert_eq!(pw(&r, "wid", Some(&tw), &wide).await.0, StatusCode::PAYLOAD_TOO_LARGE);
    let color = json!({"scopes": {"*": [{"segments": [{"text": "x", "style": {"fg": "puce"}}]}]}}).to_string();
    assert_eq!(pw(&r, "wid", Some(&tw), &color).await.0, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(pw(&r, "wid", Some(&tw), "not json").await.0, StatusCode::UNPROCESSABLE_ENTITY);
    let scoped = json!({"scopes": {"garage/a": [{"segments": []}]}}).to_string();
    assert_eq!(pw(&r, "wid", Some(&tw), &scoped).await.0, StatusCode::UNPROCESSABLE_ENTITY, "strip.right takes only \"*\"");

    // clear
    let (s, _, b) = r.call("DELETE", &url("wid"), Some(&tw), None).await;
    assert_eq!((s, b["cleared"].clone()), (StatusCode::OK, json!(true)));
    assert_eq!(r.call("DELETE", &url("wid"), Some(&tw), None).await.2["cleared"], json!(false));
    assert_eq!(r.call("DELETE", &url("wid"), None, None).await.0, StatusCode::UNAUTHORIZED);

    // a disabled plugin's token stops working
    r.st.plugins.set_enabled("wid", false).unwrap();
    let (s, _, b) = pw(&r, "wid", Some(&tw), &put_body("a")).await;
    assert_eq!((s, b["error"].as_str().unwrap()), (StatusCode::FORBIDDEN, "plugin wid is disabled"));
}

#[tokio::test]
async fn a_widget_click_on_a_core_action_needs_core_actions_and_a_plugin_invocable_id() {
    let r = Rig::new();
    let extra = format!("{WIDGETS}{}", action("go", "\"./x\""));
    r.install("plain", &manifest("plain", &["ui.widgets"], &extra));
    r.install("corep", &manifest("corep", &["ui.widgets", "core.actions"], &extra));
    let click = |action: &str| {
        json!({"scopes": {"*": [{"segments": [{"text": "x", "click": {"action": action, "args": {"k": 1}}}]}]}}).to_string()
    };
    let (tp, tc) = (r.token("plain"), r.token("corep"));
    let (s, _, b) = pw(&r, "plain", Some(&tp), &click("session.close")).await;
    assert_eq!(s, StatusCode::FORBIDDEN, "the Widget click permission scenario");
    assert!(b["error"].as_str().unwrap().contains("core.actions"), "{b}");
    assert_eq!(pw(&r, "plain", Some(&tp), &click("plain:go")).await.0, StatusCode::OK, "a plugin action needs no permission");
    assert_eq!(pw(&r, "plain", Some(&tp), &click("go")).await.0, StatusCode::OK, "nor does a bare local id");
    assert_eq!(pw(&r, "plain", Some(&tp), &click("other:thing")).await.0, StatusCode::OK, "another plugin's action resolves at click time");
    assert_eq!(pw(&r, "plain", Some(&tp), &click("plain:nope")).await.0, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(pw(&r, "plain", Some(&tp), &click("nope")).await.0, StatusCode::UNPROCESSABLE_ENTITY);

    assert_eq!(pw(&r, "corep", Some(&tc), &click("session.close")).await.0, StatusCode::OK);
    assert_eq!(pw(&r, "corep", Some(&tc), &click("app.quit")).await.0, StatusCode::UNPROCESSABLE_ENTITY, "never plugin-invocable");
    assert_eq!(pw(&r, "corep", Some(&tc), &click("overlay.close")).await.0, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(pw(&r, "corep", Some(&tc), &click("session.teleport")).await.0, StatusCode::UNPROCESSABLE_ENTITY, "not a core id");
}

#[tokio::test]
async fn widget_pushes_are_rate_limited_with_retry_after() {
    let r = Rig::new();
    r.install("fast", &manifest("fast", &["ui.widgets"], WIDGETS));
    let t = r.token("fast");
    let mut ok = 0;
    let mut limited = None;
    for i in 0..40 {
        let (s, h, _) = r.call("PUT", "/api/plugins/fast/widgets/bar", Some(&t), Some(&put_body(&format!("v{i}")))).await;
        match s {
            StatusCode::OK => ok += 1,
            StatusCode::TOO_MANY_REQUESTS => {
                limited.get_or_insert(h);
            }
            other => panic!("unexpected {other}"),
        }
    }
    assert!((20..=22).contains(&ok), "burst of 20 plus a little refill, got {ok}");
    let retry: u64 = limited.expect("a 429 happened").get("retry-after").unwrap().to_str().unwrap().parse().unwrap();
    assert!(retry >= 1);
}

#[tokio::test]
async fn a_foreign_origin_cannot_use_a_plugin_token_but_a_header_less_caller_can() {
    let r = Rig::new();
    r.install("orig", &manifest("orig", &["ui.widgets"], WIDGETS));
    let t = r.token("orig");
    let (s, _, b) = r.call_with("PUT", "/api/plugins/orig/widgets/bar", Some(&t), Some(&put_body("a")), &[("origin", "https://evil.example")]).await;
    assert_eq!((s, b["error"].as_str().unwrap()), (StatusCode::FORBIDDEN, "origin not allowed"));
    assert_eq!(r.st.plugins.widgets().rev("orig"), 0);
    assert_eq!(r.call("PUT", "/api/plugins/orig/widgets/bar", Some(&t), Some(&put_body("a"))).await.0, StatusCode::OK);
}

#[tokio::test]
async fn session_input_needs_a_token_the_permission_and_a_live_garage_session() {
    let tmux = crate::test_support::TestTmux::new();
    let r = Rig::with_tmux(&tmux);
    let args = vec!["-c".to_string(), "read line; printf '%s' \"$line\" > out.txt; sleep 60".to_string()];
    r.st.backend.create("garage/ws/s", tmux.dir(), "sh", &args, &[]).await.unwrap();
    r.install("qs", &manifest("qs", &["sessions.input"], ""));
    r.install("ro", &manifest("ro", &["sessions.read"], ""));
    let out = tmux.dir().join("out.txt");
    let body = |id: &str| json!({"id": id, "text": "hello there"}).to_string();

    let (s, _, b) = r.call("POST", "/api/sessions/input", None, Some(&body("garage/ws/s"))).await;
    assert_eq!((s, b["error"].as_str().unwrap().contains("missing plugin token")), (StatusCode::UNAUTHORIZED, true));
    assert_eq!(r.call("POST", "/api/sessions/input", Some("forged"), Some(&body("garage/ws/s"))).await.0, StatusCode::UNAUTHORIZED);
    let (s, _, b) = r.call("POST", "/api/sessions/input", Some(&r.token("ro")), Some(&body("garage/ws/s"))).await;
    assert_eq!((s, b["error"].as_str().unwrap()), (StatusCode::FORBIDDEN, "plugin ro lacks permission sessions.input"));
    let qs = r.token("qs");
    assert_eq!(r.call("POST", "/api/sessions/input", Some(&qs), Some(&body("other/x"))).await.0, StatusCode::FORBIDDEN);
    assert_eq!(r.call("POST", "/api/sessions/input", Some(&qs), Some(&body("garage/ws/missing"))).await.0, StatusCode::NOT_FOUND);
    assert_eq!(r.call("POST", "/api/sessions/input", Some(&qs), Some(r#"{"id":"garage/ws/s"}"#)).await.0, StatusCode::BAD_REQUEST);
    assert!(!out.exists(), "every refused call typed nothing");

    let (s, _, _) = r.call("POST", "/api/sessions/input", Some(&qs), Some(&body("garage/ws/s"))).await;
    assert_eq!(s, StatusCode::OK);
    tmux.wait_for(|| std::fs::read_to_string(&out).is_ok_and(|t| !t.is_empty())).await;
    assert_eq!(std::fs::read_to_string(&out).unwrap(), "hello there");
}

#[tokio::test]
async fn closed_sessions_are_listed_per_workspace_until_restored_or_forgotten() {
    let r = Rig::new();
    let _recorder = crate::closed::spawn(&r.st.bus, r.st.registry.clone(), r.st.backend.clone());
    r.st.registry.upsert_workspace("ws", "/proj").await.unwrap();
    r.install("reader", &manifest("reader", &["sessions.read"], ""));
    r.install("other", &manifest("other", &[], ""));
    let poller = r.st.shared_poller();
    for id in ["garage/ws/a", "garage/ws/b", "garage/other/z"] {
        r.backend.set(id, "/proj", Some(1), "");
    }
    poller.poll_once().await;
    r.st.closing.mark("garage/ws/a", CloseReason::User);
    for id in ["garage/ws/a", "garage/ws/b", "garage/other/z"] {
        r.backend.remove(id);
    }
    r.backend.set("garage/ws/alive", "/proj", Some(9), "");
    poller.poll_once().await;

    let list = || async { r.call("GET", "/api/workspaces/ws/closed", None, None).await };
    let end = std::time::Instant::now() + Duration::from_secs(5);
    let body = loop {
        let (s, _, b) = list().await;
        assert_eq!(s, StatusCode::OK);
        if b.as_array().unwrap().len() == 2 {
            break b;
        }
        assert!(std::time::Instant::now() < end, "records never appeared: {b}");
        tokio::time::sleep(Duration::from_millis(20)).await;
    };
    let by_id: HashMap<&str, &Value> = body.as_array().unwrap().iter().map(|r| (r["id"].as_str().unwrap(), r)).collect();
    let a = by_id["garage/ws/a"];
    assert_eq!((a["workspace"].as_str(), a["dir"].as_str(), a["reason"].as_str()), (Some("ws"), Some("/proj"), Some("user")));
    assert_eq!(by_id["garage/ws/b"]["reason"], "crash");
    assert!(a["closedAt"].as_i64().is_some_and(|t| t > 1_600_000_000_000));
    assert!(a.get("branch").is_some() && a.get("lastStatus").is_some());

    // a token is optional, but one that is presented must be valid and allowed
    let tok = r.token("reader");
    assert_eq!(r.call("GET", "/api/workspaces/ws/closed", Some(&tok), None).await.0, StatusCode::OK);
    assert_eq!(r.call("GET", "/api/workspaces/ws/closed", Some("bad"), None).await.0, StatusCode::UNAUTHORIZED);
    assert_eq!(r.call("GET", "/api/workspaces/ws/closed", Some(&r.token("other")), None).await.0, StatusCode::FORBIDDEN);
    assert_eq!(r.call("GET", "/api/workspaces/nope/closed", None, None).await.0, StatusCode::NOT_FOUND);

    // the session comes back: its record goes
    r.backend.set("garage/ws/b", "/proj", Some(2), "");
    poller.poll_once().await;
    let end = std::time::Instant::now() + Duration::from_secs(5);
    while list().await.2.as_array().unwrap().len() != 1 {
        assert!(std::time::Instant::now() < end, "restored session kept its record");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    // forgetting the other one (resume metadata delete) removes its record too
    r.st.registry
        .upsert_session_meta("garage/ws/a", crate::state::registry::MetaUpdate { workspace: Some("ws".into()), ..Default::default() })
        .await
        .unwrap();
    let (s, _, _) = r.call("DELETE", "/api/sessions/garage/ws/a?meta=1", None, None).await;
    assert_eq!(s, StatusCode::OK);
    assert!(list().await.2.as_array().unwrap().is_empty());
}

#[tokio::test]
async fn a_removed_plugin_loses_its_widgets_and_tokens_stop_working() {
    let r = Rig::new();
    r.install("gone", &manifest("gone", &["ui.widgets"], WIDGETS));
    let t = r.token("gone");
    r.call("PUT", "/api/plugins/gone/widgets/bar", Some(&t), Some(&put_body("a"))).await;
    assert!(!r.st.plugins.widgets().widgets("gone").is_empty());
    // `plugin remove` deletes the entry, then asks the daemon to reload
    let paths = r.paths();
    let mut file = PluginsFile::read(&paths).unwrap();
    file.plugins.clear();
    file.write(&paths).unwrap();
    r.call_ui("POST", "/api/plugins/reload", None).await;
    assert!(r.st.plugins.widgets().widgets("gone").is_empty());
    assert_eq!(r.call("PUT", "/api/plugins/gone/widgets/bar", Some(&t), Some(&put_body("a"))).await.0, StatusCode::UNAUTHORIZED);
}

// ---------------------------------------------------------------------
// Credentials: who may trigger what (security review)
// ---------------------------------------------------------------------

const GO: &str = "[[actions]]\nid = \"go\"\ntitle = \"go\"\ncommand = [\"./x\"]\n";

#[tokio::test]
async fn running_an_action_needs_the_ui_token_or_the_plugins_own_token_for_its_own_actions() {
    let r = Rig::new();
    r.install("one", &manifest("one", &[], GO));
    r.install("two", &manifest("two", &[], GO));
    let (t1, t2, ui) = (r.token("one"), r.token("two"), r.ui_token());
    let run = |plugin: &'static str, tok: Option<String>| {
        let r = &r;
        async move { r.call("POST", &format!("/api/plugins/{plugin}/actions/go/run"), tok.as_deref(), None).await }
    };
    // who is nobody
    assert_eq!(run("one", None).await.0, StatusCode::UNAUTHORIZED);
    assert_eq!(run("one", Some("garbage".into())).await.0, StatusCode::UNAUTHORIZED);
    assert_eq!(run("one", Some(String::new())).await.0, StatusCode::UNAUTHORIZED, "an empty bearer is no credential");
    assert_eq!(r.runner.count(), 0);
    // a plugin runs its own action, not another's
    assert_eq!(run("one", Some(t1.clone())).await.0, StatusCode::ACCEPTED);
    let (s, _, b) = run("two", Some(t1.clone())).await;
    assert_eq!(s, StatusCode::FORBIDDEN);
    assert!(b["error"].as_str().unwrap().contains("needs the UI token"), "{b}");
    // the TUI runs anyone's
    assert_eq!(run("two", Some(ui.clone())).await.0, StatusCode::ACCEPTED);
    assert_eq!(run("one", Some(ui)).await.0, StatusCode::ACCEPTED);
    wait_for(|| r.runner.count() == 3).await;
    // a disabled plugin's own token is dead
    r.st.plugins.set_enabled("two", false).unwrap();
    assert_eq!(run("two", Some(t2)).await.0, StatusCode::FORBIDDEN);
    assert_eq!(r.runner.count(), 3);
}

#[tokio::test]
async fn reload_and_ui_slots_are_closed_to_plugins_and_to_anonymous_callers() {
    let mut r = Rig::new();
    r.install("hooked", &manifest("hooked", &["ui.widgets"], &format!("{WIDGETS}{}", hook("session.created"))));
    let tok = r.token("hooked");
    let _loop = r.st.plugins.start();
    // wear the hooks out: five failures switch them off
    r.runner.outcomes.lock().unwrap().extend(std::iter::repeat_n(RunOutcome::Failed(Some(1)), 5));
    for i in 0..5 {
        r.st.bus.publish(created(&format!("garage/w/s{i}")));
    }
    r.next_event(|e| matches!(e, DaemonEvent::PluginNotice { .. })).await;
    assert!(r.st.plugins.events_disabled("hooked"));
    r.events();

    let geo = r#"{"strip.right":{"width":38}}"#;
    for (m, u, body) in [("POST", "/api/plugins/reload", None), ("PUT", "/api/plugins/ui/slots", Some(geo))] {
        assert_eq!(r.call(m, u, None, body).await.0, StatusCode::UNAUTHORIZED, "{u} anonymous");
        assert_eq!(r.call(m, u, Some(""), body).await.0, StatusCode::UNAUTHORIZED, "{u} empty bearer");
        assert_eq!(r.call(m, u, Some("nope"), body).await.0, StatusCode::FORBIDDEN, "{u} unknown token");
        assert_eq!(r.call(m, u, Some(&tok), body).await.0, StatusCode::FORBIDDEN, "{u} a plugin's own token");
    }
    assert!(r.st.plugins.events_disabled("hooked"), "a plugin cannot reset its own auto-disable");
    assert_eq!(r.st.plugins.ui_slots(), json!({}), "and cannot write geometry");
    assert!(r.events().is_empty());

    assert_eq!(r.call_ui("POST", "/api/plugins/reload", None).await.0, StatusCode::OK);
    assert!(!r.st.plugins.events_disabled("hooked"), "the UI token can");
    assert_eq!(r.call_ui("PUT", "/api/plugins/ui/slots", Some(geo)).await.0, StatusCode::OK);
}

#[tokio::test]
async fn the_ui_token_is_a_private_per_install_file_distinct_from_plugin_tokens() {
    let r = Rig::new();
    r.install("pp", &manifest("pp", &[], ""));
    let ui = r.ui_token();
    assert_eq!(ui.len(), 64);
    assert_ne!(ui, r.token("pp"));
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(super::layout::ui_token_path(r.garage.path())).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
    }
    // stable across daemon restarts (a new host over the same directory)
    let again = crate::AppState::new(
        crate::Config { port: 1, garage_dir: r.garage.path().to_path_buf(), tmux_socket: None, tmux_control: false, ui_origins: vec![] },
        r.backend.clone(),
        std::sync::Arc::new(crate::os::UnsupportedOs),
    );
    assert!(again.plugins.is_ui_token(&ui));
    assert!(!again.plugins.is_ui_token(""));
    assert!(!again.plugins.is_ui_token(&r.token("pp")));
}

// ---------------------------------------------------------------------
// ui/slots: shape, size, rate, change detection
// ---------------------------------------------------------------------

#[tokio::test]
async fn ui_slots_are_validated_size_capped_rate_limited_and_announced_only_when_they_change() {
    let r = Rig::new();
    let mut rx = r.st.bus.subscribe();
    let put = |body: String| {
        let r = &r;
        async move { r.call_ui("PUT", "/api/plugins/ui/slots", Some(&body)).await }
    };
    let mut changed = |_: &Rig| {
        let mut n = 0;
        while let Ok(e) = rx.try_recv() {
            n += usize::from(e == DaemonEvent::PluginsChanged);
        }
        n
    };

    // shape: unknown slot, unknown key, wrong type, inverted range
    for bad in [
        json!({"nope": {"width": 1}}),
        json!({"strip.right": {"junk": 1}}),
        json!({"strip.right": {"width": "wide"}}),
        json!({"strip.right": {"filler": {"x0": 9, "x1": 2}}}),
        json!([1, 2]),
    ] {
        assert_eq!(put(bad.to_string()).await.0, StatusCode::UNPROCESSABLE_ENTITY, "{bad}");
    }
    // size: an object that is well-formed but huge
    let big = json!({"strip.right": {"junk": "x".repeat(20_000)}}).to_string();
    assert_eq!(put(big).await.0, StatusCode::PAYLOAD_TOO_LARGE);
    assert_eq!(r.st.plugins.ui_slots(), json!({}), "nothing stored");
    assert_eq!(changed(&r), 0);

    // a good value is stored and announced once; the same value again is silent
    let good = json!({"strip.right": {"width": 38, "filler": {"x0": 4, "x1": 30}}, "rail.row": {"width": 24}}).to_string();
    assert_eq!(put(good.clone()).await.0, StatusCode::OK);
    assert_eq!(changed(&r), 1);
    assert_eq!(put(good).await.0, StatusCode::OK);
    assert_eq!(changed(&r), 0, "an unchanged value publishes nothing");
    assert_eq!(r.st.plugins.ui_slots()["strip.right"]["width"], 38);
    assert_eq!(put(json!({"strip.right": {"width": 40}}).to_string()).await.0, StatusCode::OK);
    assert_eq!(changed(&r), 1);

    // rate: the 20-call burst is spent (9 calls above), then 429 with Retry-After
    let mut limited = None;
    for i in 0..30 {
        let (s, h, _) = put(json!({"strip.right": {"width": 100 + i}}).to_string()).await;
        if s == StatusCode::TOO_MANY_REQUESTS {
            limited = Some(h);
            break;
        }
    }
    let h = limited.expect("a resize storm is limited");
    assert!(h.get("retry-after").is_some());
}

// ---------------------------------------------------------------------
// Widget ttl cap
// ---------------------------------------------------------------------

#[tokio::test]
async fn a_widget_push_cannot_ask_for_more_than_an_hour_of_ttl() {
    let r = Rig::new();
    r.install("ttl", &manifest("ttl", &["ui.widgets"], WIDGETS));
    let tok = r.token("ttl");
    let body = |t: u32| json!({"scopes": {"*": [{"segments": [{"text": "x"}]}]}, "ttl_secs": t}).to_string();
    assert_eq!(pw(&r, "ttl", Some(&tok), &body(3600)).await.0, StatusCode::OK);
    let (s, _, b) = pw(&r, "ttl", Some(&tok), &body(3601)).await;
    assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY);
    assert!(b["error"].as_str().unwrap().contains("3600"), "{b}");
    assert_eq!(r.st.plugins.widgets().widgets("ttl")["bar"].scopes["*"].len(), 1, "the earlier push stands");
    assert_eq!(r.st.plugins.widgets().sweepers_started(), 1);
}

// ---------------------------------------------------------------------
// Hooks stay off the bus loop; a lagged host says so
// ---------------------------------------------------------------------

struct BlockingLookup {
    entered: std::sync::atomic::AtomicUsize,
    gate: tokio::sync::Semaphore,
}

#[async_trait::async_trait]
impl super::context::SessionLookup for BlockingLookup {
    async fn session(&self, _id: &str) -> Option<super::context::SessionInfo> {
        self.entered.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        self.gate.acquire().await.unwrap().forget();
        None
    }
}

#[tokio::test]
async fn slow_context_building_never_stalls_the_bus_loop() {
    let r = Rig::new();
    let lookup = std::sync::Arc::new(BlockingLookup { entered: Default::default(), gate: tokio::sync::Semaphore::new(0) });
    let host = super::PluginHost::with_runner(&r.st.config, r.st.bus.clone(), lookup.clone(), r.runner.clone());
    r.link("slow", &manifest("slow", &[], &hook("session.closed")));
    let _loop = host.start();
    for i in 0..3 {
        r.st.bus.publish(DaemonEvent::SessionClosed {
            id: format!("garage/w/s{i}"),
            dir: "/d".into(),
            workspace: "w".into(),
            reason: CloseReason::Crash,
            last_status: None,
        });
    }
    // all three lookups are in flight at once although none has finished: the
    // loop moved on to the next event instead of waiting for the first
    wait_for(|| lookup.entered.load(std::sync::atomic::Ordering::SeqCst) == 3).await;
    assert_eq!(r.runner.count(), 0);
    lookup.gate.add_permits(3);
    wait_for(|| r.runner.count() == 3).await;
}

#[tokio::test]
async fn a_host_that_falls_behind_the_bus_logs_the_gap_for_every_active_plugin() {
    let r = Rig::new();
    let bus = crate::bus::EventBus::new(2);
    let lookup = std::sync::Arc::new(super::context::StateLookup {
        backend: r.st.backend.clone(),
        status: r.st.status.clone(),
        registry: r.st.registry.clone(),
    });
    let host = super::PluginHost::with_runner(&r.st.config, bus.clone(), lookup, r.runner.clone());
    r.link("watcher", &manifest("watcher", &[], &hook("session.created")));
    let _loop = host.start();
    // the loop has not run yet: ten events into a two-slot channel
    for i in 0..10 {
        bus.publish(created(&format!("garage/w/s{i}")));
    }
    let log = r.paths().log("watcher");
    wait_for(|| std::fs::read_to_string(&log).is_ok_and(|t| t.contains("missed") && t.contains("bus events"))).await;
}

// ---------------------------------------------------------------------
// The registry has one writer at a time
// ---------------------------------------------------------------------

#[tokio::test]
async fn the_hosts_registry_edits_survive_a_cli_writing_at_the_same_time() {
    let r = Rig::new();
    r.install("keep", &manifest("keep", &[], ""));
    let paths = r.paths();
    let cli = {
        let paths = paths.clone();
        std::thread::spawn(move || {
            for i in 0..25 {
                PluginsFile::update(&paths, |f| {
                    f.plugins.push(super::layout::Entry::new(&format!("cli{i}"), super::layout::Source::Link { path: "/x".into() }));
                    (true, ())
                })
                .unwrap();
            }
        })
    };
    for i in 0..25 {
        r.st.plugins.set_enabled("keep", i % 2 == 0).unwrap();
    }
    cli.join().unwrap();
    let file = PluginsFile::read(&paths).unwrap();
    assert_eq!(file.plugins.len(), 26, "no CLI edit was lost to a host write");
    assert!(file.plugins[0].enabled, "and the host's last edit is the last word");
}
