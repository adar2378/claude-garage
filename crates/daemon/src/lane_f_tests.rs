//! Lane F route tests: workspaces, worktree finish, editor, picker, visibility,
//! daemon restart. Fake `Os` everywhere (no GUI, notification or folder picker
//! is ever started), temp `GARAGE_DIR`s, real tmux only through `TestTmux`.

use std::cell::Cell;
use std::path::Path;
use std::sync::Arc;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use serde_json::{json, Value};
use tower::ServiceExt;

use crate::api::daemon_restart::{default_successor_args, wait_for_predecessor, Lifecycle};
use crate::api::editor::{editor_cmd, resolve};
use crate::backend::SessionBackend;
use crate::os::fake::{FakeOs, Mode};
use crate::poller::fakes::FakeBackend;
use crate::test_support::TestTmux;
use crate::{app, AppState, Config};

fn state_with(os: Arc<FakeOs>, backend: Arc<dyn SessionBackend>, garage: &Path) -> AppState {
    let cfg = Config {
        port: 4999,
        garage_dir: garage.to_path_buf(),
        tmux_socket: None,
        tmux_control: false,
        ui_origins: vec![],
    };
    AppState::new(cfg, backend, os)
}

fn fake_state(garage: &Path) -> (AppState, Arc<FakeOs>) {
    let os = Arc::new(FakeOs::default());
    (
        state_with(os.clone(), Arc::new(FakeBackend::new()), garage),
        os,
    )
}

async fn call(
    state: &AppState,
    method: &str,
    uri: &str,
    body: Option<Value>,
) -> (StatusCode, Value) {
    let mut req = Request::builder().method(method).uri(uri);
    let body = match body {
        Some(b) => {
            req = req.header("content-type", "application/json");
            Body::from(b.to_string())
        }
        None => Body::empty(),
    };
    let res = app(state.clone())
        .oneshot(req.body(body).unwrap())
        .await
        .unwrap();
    let status = res.status();
    let bytes = res.into_body().collect().await.unwrap().to_bytes();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}

fn git(dir: &Path, args: &[&str]) {
    let ok = std::process::Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_SYSTEM", "/dev/null")
        .env("GIT_AUTHOR_NAME", "t")
        .env("GIT_AUTHOR_EMAIL", "t@t")
        .env("GIT_COMMITTER_NAME", "t")
        .env("GIT_COMMITTER_EMAIL", "t@t")
        .output()
        .unwrap()
        .status
        .success();
    assert!(ok, "git {args:?}");
}

// ---------------------------------------------------------------- workspaces

#[tokio::test]
async fn put_workspace_validates_name_and_dir_then_registers() {
    let garage = tempfile::tempdir().unwrap();
    let proj = tempfile::tempdir().unwrap();
    let file = proj.path().join("f.txt");
    std::fs::write(&file, "x").unwrap();
    let (st, _) = fake_state(garage.path());
    let dir = proj.path().to_str().unwrap();

    for body in [
        json!({"dir": dir}),
        json!({"name": "Bad_Name", "dir": dir}),
        json!({"name": "", "dir": dir}),
    ] {
        let (s, b) = call(&st, "PUT", "/api/workspaces", Some(body)).await;
        assert_eq!(
            (s, b),
            (
                StatusCode::BAD_REQUEST,
                json!({"error": "name must match [a-z0-9-]+"})
            )
        );
    }
    // Missing dir: JS prints the undefined value into the message.
    let (s, b) = call(&st, "PUT", "/api/workspaces", Some(json!({"name": "ws"}))).await;
    assert_eq!(
        (s, b),
        (
            StatusCode::BAD_REQUEST,
            json!({"error": "dir is not a directory: undefined"})
        )
    );
    let (s, b) = call(
        &st,
        "PUT",
        "/api/workspaces",
        Some(json!({"name": "ws", "dir": file.to_str().unwrap()})),
    )
    .await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    assert_eq!(
        b["error"],
        format!("dir is not a directory: {}", file.display())
    );
    // No body at all is `{}` -> the name error, not a framework 4xx.
    assert_eq!(
        call(&st, "PUT", "/api/workspaces", None).await.0,
        StatusCode::BAD_REQUEST
    );

    let (s, b) = call(
        &st,
        "PUT",
        "/api/workspaces",
        Some(json!({"name": "ws", "dir": dir})),
    )
    .await;
    assert_eq!((s, b), (StatusCode::OK, json!({"name": "ws", "dir": dir})));
    assert_eq!(
        st.registry
            .get_workspace("ws")
            .await
            .unwrap()
            .dir
            .as_deref(),
        Some(dir)
    );
}

#[tokio::test]
async fn get_workspaces_lists_name_dir_and_resolved_branch() {
    let garage = tempfile::tempdir().unwrap();
    let repo = tempfile::tempdir().unwrap();
    let plain = tempfile::tempdir().unwrap();
    git(repo.path(), &["init", "-q", "-b", "feat-x"]);
    git(
        repo.path(),
        &["commit", "-q", "--allow-empty", "-m", "init"],
    );
    let (st, _) = fake_state(garage.path());
    assert_eq!(call(&st, "GET", "/api/workspaces", None).await.1, json!([]));
    st.registry
        .upsert_workspace("a-repo", repo.path().to_str().unwrap())
        .await
        .unwrap();
    st.registry
        .upsert_workspace("b-plain", plain.path().to_str().unwrap())
        .await
        .unwrap();
    let (s, b) = call(&st, "GET", "/api/workspaces", None).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(
        b,
        json!([
            {"name": "a-repo", "dir": repo.path().to_str().unwrap(), "branch": "feat-x"},
            {"name": "b-plain", "dir": plain.path().to_str().unwrap(), "branch": null},
        ])
    );
}

#[tokio::test]
async fn patch_workspace_errors_unknown_bad_name_and_conflict() {
    let garage = tempfile::tempdir().unwrap();
    let (st, _) = fake_state(garage.path());
    st.registry.upsert_workspace("one", "/x").await.unwrap();
    st.registry.upsert_workspace("two", "/y").await.unwrap();
    let (s, b) = call(
        &st,
        "PATCH",
        "/api/workspaces/nope",
        Some(json!({"name": "z"})),
    )
    .await;
    assert_eq!(
        (s, b),
        (
            StatusCode::NOT_FOUND,
            json!({"error": "unknown workspace: nope"})
        )
    );
    let (s, b) = call(
        &st,
        "PATCH",
        "/api/workspaces/one",
        Some(json!({"name": "No Good"})),
    )
    .await;
    assert_eq!(
        (s, b),
        (
            StatusCode::BAD_REQUEST,
            json!({"error": "name must match [a-z0-9-]+"})
        )
    );
    let (s, b) = call(
        &st,
        "PATCH",
        "/api/workspaces/one",
        Some(json!({"name": "two"})),
    )
    .await;
    assert_eq!(
        (s, b),
        (
            StatusCode::CONFLICT,
            json!({"error": "workspace already registered: two"})
        )
    );
    // Unknown wins over a bad name (check order).
    assert_eq!(
        call(
            &st,
            "PATCH",
            "/api/workspaces/nope",
            Some(json!({"name": "B!"}))
        )
        .await
        .0,
        StatusCode::NOT_FOUND
    );
}

fn live(tmux: &TestTmux) -> Vec<String> {
    let mut v: Vec<String> = tmux
        .tmux(&["ls", "-F", "#{session_name}"])
        .lines()
        .map(str::to_owned)
        .collect();
    v.sort();
    v
}

async fn spawn_sessions(tmux: &TestTmux, ids: &[&str]) -> Arc<crate::backend::tmux::TmuxBackend> {
    let backend = Arc::new(tmux.backend());
    backend.init().await.unwrap();
    for id in ids {
        backend
            .create(id, tmux.dir(), "sleep", &["60".to_string()], &[])
            .await
            .unwrap();
    }
    backend
}

#[tokio::test]
async fn patch_workspace_renames_live_sessions_registry_and_resume_metadata() {
    let tmux = TestTmux::new();
    let garage = tempfile::tempdir().unwrap();
    // `old2` shares the prefix text `old`: it must not be touched.
    let backend = spawn_sessions(&tmux, &["garage/old/a", "garage/old/b", "garage/old2/c"]).await;
    let st = state_with(Arc::new(FakeOs::default()), backend, garage.path());
    st.registry.upsert_workspace("old", "/proj").await.unwrap();
    st.registry
        .upsert_session_meta(
            "garage/old/a",
            crate::state::registry::MetaUpdate {
                claude_session_id: Some(Some("sid".into())),
                workspace: Some("old".into()),
                label: Some("a".into()),
                worktree: None,
            },
        )
        .await
        .unwrap();

    let (s, b) = call(
        &st,
        "PATCH",
        "/api/workspaces/old",
        Some(json!({"name": "new"})),
    )
    .await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(b["name"], "new");
    assert_eq!(b["dir"], "/proj");
    let mut renamed: Vec<_> = b["renamedSessions"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| (r["from"].clone(), r["to"].clone()))
        .collect();
    renamed.sort_by_key(|r| r.0.to_string());
    assert_eq!(
        renamed,
        [
            (json!("garage/old/a"), json!("garage/new/a")),
            (json!("garage/old/b"), json!("garage/new/b"))
        ]
    );
    assert!(
        b.get("failedSessions").is_none(),
        "only present when non-empty"
    );
    assert_eq!(
        live(&tmux),
        ["garage/new/a", "garage/new/b", "garage/old2/c"]
    );
    assert!(st.registry.get_workspace("old").await.is_none());
    assert_eq!(
        st.registry
            .get_workspace("new")
            .await
            .unwrap()
            .dir
            .as_deref(),
        Some("/proj")
    );
    let meta = st.registry.get_session_meta("garage/new/a").await.unwrap();
    assert_eq!(meta.workspace.as_deref(), Some("new"));
    assert!(st.registry.get_session_meta("garage/old/a").await.is_none());
}

#[tokio::test]
async fn patch_workspace_reports_failed_session_renames_and_still_renames_the_registry() {
    let tmux = TestTmux::new();
    let garage = tempfile::tempdir().unwrap();
    // `garage/new/a` already lives in tmux (workspace `new` is not registered,
    // so no 409): renaming old/a onto it fails, old/b succeeds.
    let backend = spawn_sessions(&tmux, &["garage/old/a", "garage/old/b", "garage/new/a"]).await;
    let st = state_with(Arc::new(FakeOs::default()), backend, garage.path());
    st.registry.upsert_workspace("old", "/proj").await.unwrap();

    let (s, b) = call(
        &st,
        "PATCH",
        "/api/workspaces/old",
        Some(json!({"name": "new"})),
    )
    .await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(
        b["renamedSessions"],
        json!([{"from": "garage/old/b", "to": "garage/new/b"}])
    );
    let failed = b["failedSessions"].as_array().unwrap();
    assert_eq!(failed.len(), 1);
    assert_eq!(failed[0]["id"], "garage/old/a");
    assert!(!failed[0]["error"].as_str().unwrap().is_empty());
    assert_eq!(
        live(&tmux),
        ["garage/new/a", "garage/new/b", "garage/old/a"]
    );
    assert!(
        st.registry.get_workspace("new").await.is_some(),
        "no rollback: the registry still moves"
    );
}

#[tokio::test]
async fn delete_workspace_plain_is_204_drops_metadata_and_never_touches_tmux() {
    let tmux = TestTmux::new();
    let garage = tempfile::tempdir().unwrap();
    let backend = spawn_sessions(&tmux, &["garage/ws/a"]).await;
    let st = state_with(Arc::new(FakeOs::default()), backend, garage.path());
    st.registry.upsert_workspace("ws", "/proj").await.unwrap();
    st.registry
        .upsert_session_meta(
            "garage/ws/a",
            crate::state::registry::MetaUpdate {
                workspace: Some("ws".into()),
                ..Default::default()
            },
        )
        .await
        .unwrap();

    assert_eq!(
        call(&st, "DELETE", "/api/workspaces/missing", None).await.0,
        StatusCode::NOT_FOUND
    );
    let (s, b) = call(&st, "DELETE", "/api/workspaces/ws", None).await;
    assert_eq!((s, b), (StatusCode::NO_CONTENT, Value::Null));
    assert!(st.registry.get_workspace("ws").await.is_none());
    assert!(st.registry.get_session_meta("garage/ws/a").await.is_none());
    assert_eq!(
        live(&tmux),
        ["garage/ws/a"],
        "the live session keeps running"
    );
}

#[tokio::test]
async fn delete_workspace_with_sessions_kill_kills_only_that_workspaces_sessions() {
    let tmux = TestTmux::new();
    let garage = tempfile::tempdir().unwrap();
    let backend = spawn_sessions(&tmux, &["garage/ws/a", "garage/ws/b", "garage/ws2/c"]).await;
    let st = state_with(Arc::new(FakeOs::default()), backend, garage.path());
    st.registry.upsert_workspace("ws", "/proj").await.unwrap();

    let (s, b) = call(&st, "DELETE", "/api/workspaces/ws?sessions=kill", None).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(b["removed"], "ws");
    let mut killed: Vec<String> = serde_json::from_value(b["killedSessions"].clone()).unwrap();
    killed.sort();
    assert_eq!(killed, ["garage/ws/a", "garage/ws/b"]);
    assert!(b.get("failedSessions").is_none());
    assert!(st.registry.get_workspace("ws").await.is_none());
    assert_eq!(live(&tmux), ["garage/ws2/c"]);
    // Any other value of the query is the plain, tmux-safe delete.
    st.registry.upsert_workspace("ws2", "/p").await.unwrap();
    assert_eq!(
        call(&st, "DELETE", "/api/workspaces/ws2?sessions=keep", None)
            .await
            .0,
        StatusCode::NO_CONTENT
    );
    assert_eq!(live(&tmux), ["garage/ws2/c"]);
}

// ----------------------------------------------------------------- worktrees

#[tokio::test]
async fn finish_validates_record_then_fields_then_action_then_branch() {
    let garage = tempfile::tempdir().unwrap();
    let (st, _) = fake_state(garage.path());
    let wt = |extra: Value| {
        let mut w = json!({"path": "/p", "branch": "garage/x", "repoDir": "/r"});
        w.as_object_mut()
            .unwrap()
            .extend(extra.as_object().unwrap().clone());
        w
    };
    let post = |body: Value| call(&st, "POST", "/api/worktrees/finish", Some(body));

    for body in [
        json!({"action": "keep"}),
        json!({"worktree": "x", "action": "keep"}),
        json!({"worktree": null, "action": "keep"}),
    ] {
        let (s, b) = post(body).await;
        assert_eq!(
            (s, b),
            (
                StatusCode::BAD_REQUEST,
                json!({"error": "worktree record required"})
            )
        );
    }
    for missing in ["path", "branch", "repoDir"] {
        let mut w = wt(json!({}));
        w.as_object_mut().unwrap().remove(missing);
        let (s, b) = post(json!({"worktree": w, "action": "keep"})).await;
        assert_eq!(
            (s, b),
            (
                StatusCode::BAD_REQUEST,
                json!({"error": "worktree record requires path, branch, and repoDir"})
            )
        );
    }
    // Fields are checked before the action.
    let (_, b) = post(json!({"worktree": {"path": "/p"}, "action": "bogus"})).await;
    assert_eq!(
        b["error"],
        "worktree record requires path, branch, and repoDir"
    );
    let (s, b) = post(json!({"worktree": wt(json!({})), "action": "bogus"})).await;
    assert_eq!(
        (s, b),
        (
            StatusCode::BAD_REQUEST,
            json!({"error": "action must be one of merge, discard, keep"})
        )
    );
    // Action is checked before the branch guard.
    let (_, b) = post(json!({"worktree": wt(json!({"branch": "main"})), "action": "nope"})).await;
    assert_eq!(b["error"], "action must be one of merge, discard, keep");
}

#[tokio::test]
async fn finish_refuses_non_garage_branches_but_keep_is_a_pure_echo() {
    let garage = tempfile::tempdir().unwrap();
    let (st, _) = fake_state(garage.path());
    for action in ["merge", "discard"] {
        let body = json!({"worktree": {"path": "/p", "branch": "main", "repoDir": "/r"}, "action": action});
        let (s, b) = call(&st, "POST", "/api/worktrees/finish", Some(body)).await;
        assert_eq!(
            (s, b),
            (
                StatusCode::BAD_REQUEST,
                json!({"error": "refusing to touch non-garage branch: main"})
            )
        );
    }
    // `keep` does no git and no branch check, even for paths that do not exist.
    let body =
        json!({"worktree": {"path": "/p", "branch": "main", "repoDir": "/r"}, "action": "keep"});
    let (s, b) = call(&st, "POST", "/api/worktrees/finish", Some(body)).await;
    assert_eq!(
        (s, b),
        (
            StatusCode::OK,
            json!({"action": "keep", "path": "/p", "branch": "main"})
        )
    );
}

#[tokio::test]
async fn finish_discard_and_dirty_merge_over_the_route() {
    let garage = tempfile::tempdir().unwrap();
    let scratch = tempfile::tempdir().unwrap();
    let repo = scratch.path().canonicalize().unwrap().join("repo");
    let wt = garage.path().canonicalize().unwrap().join("worktrees/ws/x");
    std::fs::create_dir(&repo).unwrap();
    std::fs::create_dir_all(wt.parent().unwrap()).unwrap();
    git(&repo, &["init", "-q", "-b", "main"]);
    git(&repo, &["commit", "-q", "--allow-empty", "-m", "base"]);
    git(
        &repo,
        &[
            "worktree",
            "add",
            "-q",
            "-b",
            "garage/x",
            wt.to_str().unwrap(),
        ],
    );
    let (st, _) = fake_state(garage.path());
    let record = json!({"path": wt.to_str().unwrap(), "branch": "garage/x", "repoDir": repo.to_str().unwrap()});

    std::fs::write(wt.join("untracked.txt"), "x").unwrap();
    let (s, b) = call(
        &st,
        "POST",
        "/api/worktrees/finish",
        Some(json!({"worktree": record, "action": "merge"})),
    )
    .await;
    assert_eq!(s, StatusCode::CONFLICT);
    assert!(b["error"].as_str().unwrap().contains("uncommitted"));

    let (s, b) = call(
        &st,
        "POST",
        "/api/worktrees/finish",
        Some(json!({"worktree": record, "action": "discard"})),
    )
    .await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(
        b,
        json!({"action": "discard", "path": wt.to_str().unwrap(), "branch": "garage/x", "discarded": true})
    );
    assert!(!wt.exists());

    // A vanished path outside the worktrees root is refused before git runs.
    let forged = json!({"path": "/nonexistent/garage/wt", "branch": "garage/x", "repoDir": repo.to_str().unwrap()});
    let (s, b) = call(
        &st,
        "POST",
        "/api/worktrees/finish",
        Some(json!({"worktree": forged, "action": "discard"})),
    )
    .await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    assert!(b["error"]
        .as_str()
        .unwrap()
        .contains("not a garage worktree"));
}

// -------------------------------------------------------------------- editor

async fn editor_rig(garage: &Path) -> (AppState, Arc<FakeOs>) {
    let (st, os) = fake_state(garage);
    st.registry
        .upsert_workspace("ws", "/work/proj")
        .await
        .unwrap();
    (st, os)
}

#[test]
fn resolve_folds_dot_segments_and_lets_absolute_paths_win_like_node() {
    assert_eq!(resolve("/work/proj", "src/a.rs"), "/work/proj/src/a.rs");
    assert_eq!(
        resolve("/work/proj", "./src/../src/a.rs"),
        "/work/proj/src/a.rs"
    );
    assert_eq!(resolve("/work/proj", "../../etc/passwd"), "/etc/passwd");
    assert_eq!(
        resolve("/work/proj", "../../../../../etc/passwd"),
        "/etc/passwd"
    );
    assert_eq!(resolve("/work/proj", "/etc/passwd"), "/etc/passwd");
    assert_eq!(resolve("/work/proj", "sub/"), "/work/proj/sub");
}

#[tokio::test]
async fn open_editor_unknown_workspace_is_404_and_starts_nothing() {
    let garage = tempfile::tempdir().unwrap();
    let (st, os) = editor_rig(garage.path()).await;
    let (s, b) = call(
        &st,
        "POST",
        "/api/open-editor",
        Some(json!({"workspace": "nope"})),
    )
    .await;
    assert_eq!(
        (s, b),
        (
            StatusCode::NOT_FOUND,
            json!({"error": "unknown workspace: nope"})
        )
    );
    let (_, b) = call(&st, "POST", "/api/open-editor", Some(json!({}))).await;
    assert_eq!(b["error"], "unknown workspace: undefined");
    assert!(os.opened().is_empty());
}

#[tokio::test]
async fn open_editor_path_escape_is_400_and_no_editor_starts() {
    let garage = tempfile::tempdir().unwrap();
    let (st, os) = editor_rig(garage.path()).await;
    for file in [
        "../../etc/passwd",
        "/etc/passwd",
        "sub/../../x",
        "../proj2/a.rs",
    ] {
        let (s, b) = call(
            &st,
            "POST",
            "/api/open-editor",
            Some(json!({"workspace": "ws", "file": file})),
        )
        .await;
        assert_eq!(
            (s, b),
            (
                StatusCode::BAD_REQUEST,
                json!({"error": "file escapes workspace directory"})
            ),
            "{file}"
        );
    }
    let (s, b) = call(
        &st,
        "POST",
        "/api/open-editor",
        Some(json!({"workspace": "ws", "file": 5})),
    )
    .await;
    assert_eq!(
        (s, b),
        (
            StatusCode::BAD_REQUEST,
            json!({"error": "file must be a string"})
        )
    );
    assert!(os.opened().is_empty());
}

#[tokio::test]
async fn open_editor_opens_the_dir_or_goto_file_line() {
    let garage = tempfile::tempdir().unwrap();
    let (st, os) = editor_rig(garage.path()).await;
    let cmd = editor_cmd().to_owned();
    let open = |body: Value| call(&st, "POST", "/api/open-editor", Some(body));

    for body in [
        json!({"workspace": "ws"}),
        json!({"workspace": "ws", "file": ""}),
        json!({"workspace": "ws", "file": null}),
    ] {
        assert_eq!(open(body).await, (StatusCode::OK, json!({"ok": true})));
    }
    assert_eq!(
        open(json!({"workspace": "ws", "file": "src/a.rs", "line": 7}))
            .await
            .0,
        StatusCode::OK
    );
    assert_eq!(
        open(json!({"workspace": "ws", "file": "src/a.rs"})).await.0,
        StatusCode::OK
    );
    // `line ?? 1`: zero is a real line number.
    assert_eq!(
        open(json!({"workspace": "ws", "file": "./x/../a.rs", "line": 0}))
            .await
            .0,
        StatusCode::OK
    );
    // The workspace dir itself is not an escape.
    assert_eq!(
        open(json!({"workspace": "ws", "file": "."})).await.0,
        StatusCode::OK
    );

    let dir = || vec![cmd.clone(), "/work/proj".to_string()];
    let goto = |t: &str| vec![cmd.clone(), "--goto".to_string(), t.to_string()];
    assert_eq!(
        os.opened(),
        [
            dir(),
            dir(),
            dir(),
            goto("/work/proj/src/a.rs:7"),
            goto("/work/proj/src/a.rs:1"),
            goto("/work/proj/a.rs:0"),
            goto("/work/proj:1"),
        ]
    );
}

#[tokio::test]
async fn open_editor_missing_cli_is_501_naming_it_and_other_failures_500() {
    let garage = tempfile::tempdir().unwrap();
    let os = Arc::new(FakeOs::with_mode(Mode::NotFound));
    let st = state_with(os.clone(), Arc::new(FakeBackend::new()), garage.path());
    st.registry
        .upsert_workspace("ws", "/work/proj")
        .await
        .unwrap();
    let (s, b) = call(
        &st,
        "POST",
        "/api/open-editor",
        Some(json!({"workspace": "ws"})),
    )
    .await;
    assert_eq!(
        (s, b),
        (
            StatusCode::NOT_IMPLEMENTED,
            json!({"error": format!("editor CLI not found ({})", editor_cmd())})
        )
    );
    *os.mode.lock().unwrap() = Mode::Failed;
    assert_eq!(
        call(
            &st,
            "POST",
            "/api/open-editor",
            Some(json!({"workspace": "ws"}))
        )
        .await
        .0,
        StatusCode::INTERNAL_SERVER_ERROR
    );
}

// -------------------------------------------------------------------- picker

#[tokio::test]
async fn pick_directory_strips_the_trailing_slash_and_maps_cancel_unsupported_failure() {
    let garage = tempfile::tempdir().unwrap();
    let (st, os) = fake_state(garage.path());
    let pick = || call(&st, "POST", "/api/pick-directory", None);

    *os.pick.lock().unwrap() = Some("/Users/me/proj/".into());
    assert_eq!(
        pick().await,
        (StatusCode::OK, json!({"dir": "/Users/me/proj"}))
    );
    *os.pick.lock().unwrap() = Some("/".into());
    assert_eq!(pick().await.1, json!({"dir": "/"}));
    *os.pick.lock().unwrap() = None;
    assert_eq!(pick().await, (StatusCode::OK, json!({"cancelled": true})));

    *os.mode.lock().unwrap() = Mode::Unsupported;
    assert_eq!(
        pick().await,
        (
            StatusCode::NOT_IMPLEMENTED,
            json!({"error": "directory picker is only available on macOS"})
        )
    );
    *os.mode.lock().unwrap() = Mode::Failed;
    assert_eq!(
        pick().await,
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            json!({"error": "fake failure"})
        )
    );
}

// ---------------------------------------------------------------- visibility

#[tokio::test]
async fn visibility_requires_client_id_and_boolean_and_feeds_the_notification_rule() {
    let garage = tempfile::tempdir().unwrap();
    let (st, os) = fake_state(garage.path());
    for body in [
        json!({}),
        json!({"clientId": "tui"}),
        json!({"visible": true}),
        json!({"clientId": "", "visible": true}),
        json!({"clientId": 7, "visible": true}),
        json!({"clientId": "tui", "visible": "yes"}),
    ] {
        let (s, b) = call(&st, "POST", "/api/ui/visibility", Some(body.clone())).await;
        assert_eq!(
            (s, b),
            (
                StatusCode::BAD_REQUEST,
                json!({"error": "clientId (string) and visible (boolean) are required"})
            ),
            "{body}"
        );
    }
    assert!(!st.visibility.any_visible());

    let (s, b) = call(
        &st,
        "POST",
        "/api/ui/visibility",
        Some(json!({"clientId": "tui", "visible": true})),
    )
    .await;
    assert_eq!((s, b), (StatusCode::OK, json!({"ok": true})));
    assert!(st.visibility.any_visible());

    // A visible wall suppresses the notification; hiding it re-enables it.
    let needs_input = crate::bus::DaemonEvent::StatusTransition {
        id: "garage/ws/a".into(),
        from: garage_protocol::SessionStatus::Working,
        to: garage_protocol::SessionStatus::NeedsInput,
        since: Some(1),
    };
    assert!(!crate::os::notify::handle(&needs_input, os.as_ref(), &st.visibility).await);
    call(
        &st,
        "POST",
        "/api/ui/visibility",
        Some(json!({"clientId": "tui", "visible": false})),
    )
    .await;
    assert!(crate::os::notify::handle(&needs_input, os.as_ref(), &st.visibility).await);
    assert_eq!(
        os.notifications(),
        [(
            "claude-garage".to_string(),
            "garage/ws/a needs input".to_string()
        )]
    );
}

// ------------------------------------------------------------------- restart

#[tokio::test]
async fn restart_spawns_the_successor_with_the_predecessor_pid_replies_202_then_signals_shutdown() {
    let garage = tempfile::tempdir().unwrap();
    let out = tempfile::tempdir().unwrap();
    let pidfile = out.path().join("pid");
    let (mut st, _) = fake_state(&garage.path().join("nested"));
    // A harmless stand-in for the successor; never the test binary itself.
    st.lifecycle = Lifecycle::default().with_successor(
        "sh",
        vec![
            "-c".into(),
            format!("echo \"$GARAGE_PREDECESSOR_PID\" > {}", pidfile.display()),
        ],
    );
    let shutdown = st.lifecycle.shutdown.clone();

    let (s, b) = call(&st, "POST", "/api/daemon/restart", None).await;
    assert_eq!(s, StatusCode::ACCEPTED);
    assert!(b["pid"].as_u64().unwrap() > 0);
    tokio::time::timeout(std::time::Duration::from_secs(5), shutdown.notified())
        .await
        .expect("shutdown is signalled after the reply");

    for _ in 0..200 {
        if std::fs::read_to_string(&pidfile).is_ok_and(|s| !s.trim().is_empty()) {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(25)).await;
    }
    assert_eq!(
        std::fs::read_to_string(&pidfile).unwrap().trim(),
        std::process::id().to_string()
    );
    assert!(
        garage.path().join("nested/daemon.log").exists(),
        "log dir is created under GARAGE_DIR"
    );
}

#[tokio::test]
async fn restart_with_an_unspawnable_successor_is_500_and_does_not_shut_down() {
    let garage = tempfile::tempdir().unwrap();
    let (mut st, _) = fake_state(garage.path());
    st.lifecycle = Lifecycle::default().with_successor("/nonexistent/garage-successor", vec![]);
    let shutdown = st.lifecycle.shutdown.clone();
    assert_eq!(
        call(&st, "POST", "/api/daemon/restart", None).await.0,
        StatusCode::INTERNAL_SERVER_ERROR
    );
    assert!(
        tokio::time::timeout(std::time::Duration::from_millis(300), shutdown.notified())
            .await
            .is_err(),
        "a failed spawn must leave the running daemon alone"
    );
}

#[test]
fn the_default_successor_keeps_the_listening_port() {
    assert_eq!(default_successor_args(5001), ["daemon", "--port", "5001"]);
}

// restart.test.js: waitForPredecessor (fake probe, sleep and clock)

#[tokio::test]
async fn wait_returns_as_soon_as_the_probe_reports_the_predecessor_gone() {
    let calls = Cell::new(0);
    let sleeps = std::cell::RefCell::new(Vec::new());
    wait_for_predecessor(
        4747,
        Some(123),
        || {
            calls.set(calls.get() + 1);
            let still_there = calls.get() < 3; // "still there" twice, then "gone"
            async move { still_there }
        },
        |ms| {
            sleeps.borrow_mut().push(ms);
            async {}
        },
        || 0,
        10_000,
    )
    .await
    .unwrap();
    assert_eq!(calls.get(), 3);
    assert_eq!(*sleeps.borrow(), [200, 200]);
}

#[tokio::test]
async fn wait_returns_immediately_when_the_predecessor_is_already_gone() {
    wait_for_predecessor(
        4747,
        Some(123),
        || async { false },
        |_| async { panic!("must not sleep") },
        || 0,
        10_000,
    )
    .await
    .unwrap();
}

#[tokio::test]
async fn wait_fails_naming_port_and_pid_once_the_deadline_passes() {
    let now = Cell::new(0u64);
    let err = wait_for_predecessor(
        4747,
        Some(999),
        || async { true }, // the predecessor never lets go
        |ms| {
            now.set(now.get() + ms);
            async {}
        },
        || now.get(),
        1000,
    )
    .await
    .unwrap_err();
    assert!(err.contains("4747") && err.contains("999"), "{err}");
}

#[tokio::test]
async fn default_probe_sees_a_listening_health_endpoint_and_a_free_port() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
        .await
        .unwrap();
    let port = listener.local_addr().unwrap().port();
    let server = tokio::spawn(async move {
        loop {
            let (mut s, _) = listener.accept().await.unwrap();
            let mut buf = [0u8; 256];
            let _ = s.read(&mut buf).await;
            let _ = s
                .write_all(b"HTTP/1.0 200 OK\r\ncontent-length: 2\r\n\r\n{}")
                .await;
        }
    });
    assert!(crate::api::daemon_restart::default_probe(port).await);
    server.abort();
    let _ = server.await;
    assert!(!crate::api::daemon_restart::default_probe(port).await);
}

/// The successor's gate runs against the real router, not a stub: a 2xx from
/// `app()`'s /api/health keeps it waiting, and it releases once that daemon stops.
#[tokio::test]
async fn default_probe_against_the_real_app_health_route() {
    let garage = tempfile::tempdir().unwrap();
    let (st, _) = fake_state(garage.path());
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
        .await
        .unwrap();
    let port = listener.local_addr().unwrap().port();
    let (stop, stopped) = tokio::sync::oneshot::channel::<()>();
    let server = tokio::spawn(crate::serve(listener, st, async {
        let _ = stopped.await;
    }));
    assert!(crate::api::daemon_restart::default_probe(port).await);
    stop.send(()).unwrap();
    server.await.unwrap().unwrap();
    assert!(!crate::api::daemon_restart::default_probe(port).await);
}
