//! The first-party plugins against the real host: the shipped manifests are
//! on the compiled allowlist, seed with the documented defaults and load
//! together without conflicts; and the plugins that react to events run as
//! real binaries behind the real HTTP API (fake backend, temp `GARAGE_DIR`).
//! Nothing here touches tmux or the real `~/.garage`.

#![cfg(unix)]

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

use garage_plugin::manifest::sha256_hex;
use serde_json::json;

use super::bundled::{DEFAULT_ON, is_trusted, seed};
use super::layout::MANIFEST_FILE;
use super::rig::{Rig, wait_for};
use crate::bus::DaemonEvent;
use crate::closing::CloseReason;
use crate::state::registry::ClosedRecord;

/// The six plugins this change ships (usage-bar, turn-markers and pit-pet
/// have their own tests).
const ACTION_PLUGINS: [(&str, &str); 6] = [
    ("closed-chats", "garage-plugin-closed-chats"),
    ("jump-palette", "garage-plugin-jump-palette"),
    ("quick-send", "garage-plugin-quick-send"),
    ("tile-buttons", ""),
    ("prune-empty-projects", "garage-plugin-prune-empty-projects"),
    ("cli-extras", "garage-plugin-cli-extras"),
];

fn repo_plugins() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../plugins")
}

/// Builds a plugin crate once per test run and returns its executable.
pub fn plugin_binary(package: &str) -> PathBuf {
    static BUILT: OnceLock<Mutex<HashMap<String, PathBuf>>> = OnceLock::new();
    let mut built = BUILT.get_or_init(Default::default).lock().unwrap();
    if let Some(p) = built.get(package) {
        return p.clone();
    }
    let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".into());
    // Same profile and target dir as this test run (deps/ sits two levels
    // under the profile dir), so cargo reuses the compiled dependencies.
    let target = std::env::current_exe()
        .unwrap()
        .ancestors()
        .nth(3)
        .unwrap()
        .to_path_buf();
    let mut args = vec![
        "build",
        "-p",
        package,
        "--message-format=json",
        "--target-dir",
        target.to_str().unwrap(),
    ];
    if !cfg!(debug_assertions) {
        args.push("--release");
    }
    let out = std::process::Command::new(cargo)
        .args(args)
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .output()
        .expect("cargo runs");
    assert!(
        out.status.success(),
        "building {package} failed:\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let exe = String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter_map(|l| serde_json::from_str::<serde_json::Value>(l).ok())
        .find_map(|v| {
            v.get("executable")
                .and_then(|e| e.as_str())
                .map(PathBuf::from)
        })
        .unwrap_or_else(|| panic!("cargo reported no executable for {package}"));
    built.insert(package.to_owned(), exe.clone());
    exe
}

/// `<root>/<id>/{garage-plugin.toml[,bin/<id>]}` for the given plugins, as packaging stages them.
fn staged(plugins: &[(&str, &str)]) -> tempfile::TempDir {
    // build before anything else runs on the async path's clock
    let bins: Vec<Option<PathBuf>> = plugins
        .iter()
        .map(|(_, pkg)| (!pkg.is_empty()).then(|| plugin_binary(pkg)))
        .collect();
    let root = tempfile::tempdir().unwrap();
    for ((id, _), bin) in plugins.iter().zip(bins) {
        let dir = root.path().join(id);
        std::fs::create_dir_all(dir.join("bin")).unwrap();
        std::fs::copy(
            repo_plugins().join(id).join(MANIFEST_FILE),
            dir.join(MANIFEST_FILE),
        )
        .unwrap();
        if let Some(bin) = bin {
            std::fs::copy(bin, dir.join("bin").join(id)).unwrap();
        }
    }
    root
}

#[test]
fn every_shipped_action_plugin_is_on_the_compiled_allowlist_and_a_changed_manifest_is_not() {
    for (id, _) in ACTION_PLUGINS {
        let raw = std::fs::read(repo_plugins().join(id).join(MANIFEST_FILE)).unwrap();
        assert!(is_trusted(id, &sha256_hex(&raw)), "{id} is not on the allowlist");
        let mut edited = raw.clone();
        edited.extend_from_slice(b"\n# edited\n");
        assert!(!is_trusted(id, &sha256_hex(&edited)), "{id}: an edit stays trusted");
    }
}

#[tokio::test]
async fn the_six_seed_with_the_documented_defaults_load_together_and_none_is_refused() {
    let r = Rig::new();
    let root = staged(&ACTION_PLUGINS);
    let seeded = seed(&r.paths(), root.path()).unwrap();
    assert_eq!(seeded.added.len(), ACTION_PLUGINS.len());
    r.st.plugins.reload().unwrap();
    // turn every one on: conflicts (cli names, the single fallback, keys) show only when all are active
    for (id, _) in ACTION_PLUGINS {
        r.st.plugins.set_enabled(id, true).unwrap();
    }
    for (id, _) in ACTION_PLUGINS {
        let p = r.st.plugins.plugin(id).unwrap();
        assert!(p.entry.is_first_party(), "{id}");
        assert!(p.problems.is_empty(), "{id}: {:?}", p.problems);
        assert!(p.refused.is_none(), "{id}: {:?}", p.refused);
        let m = p.manifest().unwrap_or_else(|| panic!("{id} did not load"));
        assert_eq!(
            p.entry.granted_permissions.as_deref(),
            Some(m.permission_names().as_slice()),
            "{id}: first-party grant"
        );
    }
}

#[tokio::test]
async fn a_fresh_install_turns_on_exactly_the_default_plugins() {
    let r = Rig::new();
    let root = staged(&ACTION_PLUGINS);
    seed(&r.paths(), root.path()).unwrap();
    r.st.plugins.reload().unwrap();
    for (id, _) in ACTION_PLUGINS {
        let on = r.st.plugins.plugin(id).unwrap().active();
        assert_eq!(on, DEFAULT_ON.contains(&id), "{id}");
    }
}

#[tokio::test]
async fn closed_chats_shows_a_closed_session_as_a_restorable_row() {
    let r = Rig::serving().await;
    let root = staged(&[("closed-chats", "garage-plugin-closed-chats")]);
    seed(&r.paths(), root.path()).unwrap();
    let _events = r.st.plugins.start();
    assert!(r.st.plugins.active("closed-chats").is_some(), "on by default");

    r.st.registry.upsert_workspace("app", "/tmp").await.unwrap();
    r.st.registry
        .record_closed(ClosedRecord {
            id: "garage/app/fix-auth".into(),
            workspace: "app".into(),
            dir: "/tmp".into(),
            branch: None,
            closed_at: 1,
            reason: "user".into(),
            last_status: None,
            claude_session_id: Some("c".into()),
            worktree: None,
        })
        .await
        .unwrap();
    r.st.bus.publish(DaemonEvent::SessionClosed {
        id: "garage/app/fix-auth".into(),
        dir: "/tmp".into(),
        workspace: "app".into(),
        reason: CloseReason::User,
        last_status: None,
    });

    let shown = || {
        r.st.plugins
            .widgets()
            .widgets("closed-chats")
            .get("rows")
            .map(|w| serde_json::to_string(&w.scopes).unwrap())
            .unwrap_or_default()
    };
    wait_for(|| shown().contains("fix-auth")).await;
    let rows = shown();
    assert!(rows.contains("session.restore") && rows.contains("session.delete"), "{rows}");
    assert!(rows.contains(r#""session":"garage/app/fix-auth""#), "{rows}");

    // forgetting it (what the row's x ends in) empties the rows again, through session.forgotten
    let (status, _, _) = r
        .call("DELETE", "/api/sessions/garage/app/fix-auth?meta=1", None, None)
        .await;
    assert_eq!(status, axum::http::StatusCode::OK);
    wait_for(|| !shown().contains("fix-auth")).await;
}

fn closed(ws: &str, reason: CloseReason) -> DaemonEvent {
    DaemonEvent::SessionClosed {
        id: format!("garage/{ws}/claude-1"),
        dir: "/tmp".into(),
        workspace: ws.into(),
        reason,
        last_status: None,
    }
}

#[tokio::test]
async fn prune_removes_a_project_after_a_user_close_but_never_after_a_crash() {
    let r = Rig::serving().await;
    let root = staged(&[("prune-empty-projects", "garage-plugin-prune-empty-projects")]);
    seed(&r.paths(), root.path()).unwrap();
    r.st.plugins.reload().unwrap();
    assert!(
        r.st.plugins.active("prune-empty-projects").is_none(),
        "off by default"
    );
    r.st.plugins.set_enabled("prune-empty-projects", true).unwrap();
    let _events = r.st.plugins.start();
    for ws in ["crashed", "closed"] {
        r.st.registry.upsert_workspace(ws, "/tmp").await.unwrap();
    }

    r.st.bus.publish(closed("crashed", CloseReason::Crash));
    r.st.bus.publish(closed("closed", CloseReason::User));
    let gone = |ws: &'static str| {
        let reg = r.st.registry.clone();
        async move { reg.get_workspace(ws).await.is_none() }
    };
    // the user close is acted on after the plugin's settle delay ...
    let end = std::time::Instant::now() + std::time::Duration::from_secs(8);
    while !gone("closed").await {
        assert!(std::time::Instant::now() < end, "the project was never pruned");
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    // ... and by then the crash event (handled at once) has had its chance
    assert!(!gone("crashed").await, "a crash must not remove the project");
    let _ = json!(null);
}
