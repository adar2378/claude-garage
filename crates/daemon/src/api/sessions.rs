//! Port of `daemon/src/sessions.js`: list (live + restorable), create (with
//! worktree), delete (incl. `?meta=1`), restore and restart.

use std::collections::HashMap;
use std::path::Path;
use std::time::Duration;

use anyhow::Result;
use async_trait::async_trait;
use axum::body::Bytes;
use axum::extract::{Path as UrlPath, Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use futures_util::future::join_all;
use garage_protocol::{SessionStatus, Worktree};
use serde_json::{json, Map, Value};

use super::hooks::parse_body;
use crate::backend::tmux::{
    is_valid_name, normalize_title, session_id, split_session_id, GARAGE_PREFIX,
};
use crate::backend::{BackendSession, PaneInfo, SessionBackend};
use crate::error::error_response;
use crate::git::resolve_branch;
use crate::git::{create_worktree, current_branch, remove_worktree};
use crate::state::registry::{MetaUpdate, SessionMeta};
use crate::state::AppState;

/// What sessions run (`GARAGE_CLAUDE_CMD`, default `claude`) and how long a
/// resume gets to prove it did not die (JS: 1.5 s, then 2 s more if still up).
/// Injectable so tests never run the real `claude` and stay fast.
#[derive(Debug, Clone)]
pub struct ClaudeCmd {
    pub program: String,
    pub settle: (Duration, Duration),
}

impl ClaudeCmd {
    pub const SETTLE: (Duration, Duration) =
        (Duration::from_millis(1500), Duration::from_millis(2000));

    pub fn from_env() -> Self {
        Self::new(std::env::var("GARAGE_CLAUDE_CMD").unwrap_or_else(|_| "claude".into()))
    }

    pub fn new(program: impl Into<String>) -> Self {
        Self {
            program: program.into(),
            settle: Self::SETTLE,
        }
    }
}

// ---------------------------------------------------------------------
// Pure planning and the shared restore/restart spawn helper
// ---------------------------------------------------------------------

/// How a session's process is started: a fresh tmux session (restore) or the
/// same pane respawned in place (restart).
#[async_trait]
pub trait Spawn: Send + Sync {
    async fn spawn(&self, id: &str, dir: &Path, cmd: &str, args: &[String]) -> Result<()>;
    /// `hasSession`: errors read as "not there".
    async fn has(&self, id: &str) -> bool;
}

#[derive(Clone, Copy)]
enum Kind {
    Create,
    Respawn,
}

struct BackendSpawn<'a> {
    backend: &'a dyn SessionBackend,
    kind: Kind,
}

#[async_trait]
impl Spawn for BackendSpawn<'_> {
    async fn spawn(&self, id: &str, dir: &Path, cmd: &str, args: &[String]) -> Result<()> {
        match self.kind {
            Kind::Create => self.backend.create(id, dir, cmd, args, &[]).await,
            // A failed `--resume` can take the one-pane session with it; a
            // respawn would then fail and the session would vanish, so start
            // it again from scratch (same dir/cmd as the original create).
            Kind::Respawn if !self.has(id).await => {
                self.backend.create(id, dir, cmd, args, &[]).await
            }
            Kind::Respawn => self.backend.respawn(id, dir, cmd, args, &[]).await,
        }
    }

    async fn has(&self, id: &str) -> bool {
        self.backend.has(id).await.unwrap_or(false)
    }
}

/// Start `claude --resume <id>`, then confirm it did not immediately die:
/// `claude --resume` exits (after "No conversation found") when the
/// conversation was never persisted, taking the pane/session with it, and the
/// exit is not instant, so confirm twice before trusting it. If it died, or
/// there is no `claude_session_id` to resume, start a plain `claude` and
/// report `false`. (On restart the dead resume may have ended the one-pane
/// session; `BackendSpawn` then recreates it instead of respawning.)
pub async fn spawn_claude_resumed(
    spawner: &dyn Spawn,
    claude: &ClaudeCmd,
    id: &str,
    dir: &Path,
    claude_session_id: Option<&str>,
) -> Result<bool> {
    let Some(csid) = claude_session_id.filter(|c| !c.is_empty()) else {
        spawner.spawn(id, dir, &claude.program, &[]).await?;
        return Ok(false);
    };
    spawner
        .spawn(
            id,
            dir,
            &claude.program,
            &["--resume".to_owned(), csid.to_owned()],
        )
        .await?;
    tokio::time::sleep(claude.settle.0).await;
    if spawner.has(id).await {
        tokio::time::sleep(claude.settle.1).await;
    }
    if spawner.has(id).await {
        return Ok(true);
    }
    spawner.spawn(id, dir, &claude.program, &[]).await?;
    Ok(false)
}

/// One restart candidate: `status` is a status name, or `"unknown"` when the
/// poller never observed the id.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Candidate {
    pub id: String,
    pub status: String,
}

/// Busy (`working`, `needs-input`, `unknown`) sessions are skipped unless
/// `force`. `unknown` is never silently planned as idle.
pub fn plan_restart_targets(sessions: &[Candidate], force: bool) -> (Vec<String>, Vec<Candidate>) {
    let mut targets = Vec::new();
    let mut skipped = Vec::new();
    for s in sessions {
        let busy = matches!(s.status.as_str(), "working" | "needs-input" | "unknown");
        if !force && busy {
            skipped.push(s.clone());
        } else {
            targets.push(s.id.clone());
        }
    }
    (targets, skipped)
}

/// Poll once before planning whenever any id has no status entry yet, i.e.
/// right after a fresh successor daemon boots with an empty status store.
pub fn needs_poll_before_plan(ids: &[String], has_status: impl Fn(&str) -> bool) -> bool {
    ids.iter().any(|id| !has_status(id))
}

// ---------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------

fn status_name(s: SessionStatus) -> String {
    serde_json::to_value(s)
        .ok()
        .and_then(|v| v.as_str().map(str::to_owned))
        .unwrap_or_default()
}

/// JS truthiness for a body field.
fn truthy(v: Option<&Value>) -> bool {
    match v {
        None | Some(Value::Null) | Some(Value::Bool(false)) => false,
        Some(Value::String(s)) => !s.is_empty(),
        Some(Value::Number(n)) => n.as_f64().is_some_and(|f| f != 0.0),
        Some(_) => true,
    }
}

fn str_of(v: Option<&Value>) -> Option<&str> {
    v.and_then(Value::as_str)
}

async fn is_dir(path: &str) -> bool {
    tokio::fs::metadata(path).await.is_ok_and(|m| m.is_dir())
}

fn server_error(e: impl std::fmt::Display) -> Response {
    error_response(StatusCode::INTERNAL_SERVER_ERROR, e.to_string())
}

/// `{path, branch, repoDir, target}`; `target` is the branch `repoDir` has
/// checked out (informational), `null` when unreadable.
async fn with_target(w: &Worktree) -> Value {
    json!({
        "path": w.path,
        "branch": w.branch,
        "repoDir": w.repo_dir,
        "target": current_branch(&w.repo_dir).await,
    })
}

fn worktree_json(w: &Option<Worktree>) -> Value {
    w.as_ref()
        .map(|w| serde_json::to_value(w).unwrap_or(Value::Null))
        .unwrap_or(Value::Null)
}

// ---------------------------------------------------------------------
// GET /api/sessions
// ---------------------------------------------------------------------

pub async fn list(State(st): State<AppState>) -> Response {
    let sessions = st.backend.list().await.unwrap_or_default();
    let panes = st.backend.panes().await.unwrap_or_default();
    let hostname = gethostname::gethostname().to_str().map(str::to_owned);
    let metas = st.registry.list_session_metas().await;
    let meta_by_id: HashMap<&str, _> = metas.iter().map(|(id, m)| (id.as_str(), m)).collect();
    let live_ids: std::collections::HashSet<&str> =
        sessions.iter().map(|s| s.id.as_str()).collect();

    // Restorable entries resolve `dir` from the registry (not a cached copy)
    // so a workspace re-registered since the crash is reflected.
    let mut restorable_in = Vec::new();
    for (id, meta) in &metas {
        if live_ids.contains(id.as_str()) {
            continue;
        }
        let dir = match &meta.workspace {
            Some(ws) => st.registry.get_workspace(ws).await.and_then(|w| w.dir),
            None => None,
        };
        restorable_in.push((id, meta, dir));
    }

    // One `git rev-parse` per unique directory per request, run concurrently.
    let live_dir = |s: &BackendSession| -> String {
        panes
            .get(&s.id)
            .map(|p: &PaneInfo| p.cwd.clone())
            .unwrap_or_else(|| s.dir.clone())
    };
    let mut dirs: Vec<String> = sessions.iter().map(live_dir).collect();
    dirs.extend(restorable_in.iter().filter_map(|(_, _, d)| d.clone()));
    dirs.retain(|d| !d.is_empty());
    dirs.sort();
    dirs.dedup();
    let branches: HashMap<String, Option<String>> =
        join_all(dirs.into_iter().map(|d| async move {
            let b = resolve_branch(&d).await;
            (d, b)
        }))
        .await
        .into_iter()
        .collect();
    let branch_of = |d: &str| branches.get(d).cloned().flatten();

    let mut out: Vec<Value> = Vec::new();
    for s in &sessions {
        let entry = st.status.entry(&s.id);
        let pane = panes.get(&s.id);
        let dir = live_dir(s);

        // statusline beats the transcript cache beats null; the transcript
        // path is a cache read plus a background refresh, never file IO here.
        let mut context = st.telemetry.context.statusline_context(&s.id);
        if context.is_none() {
            context = st
                .telemetry
                .transcripts
                .cached_context(&s.id)
                .map(|c| c.to_json());
            let csid = meta_by_id
                .get(s.id.as_str())
                .and_then(|m| m.claude_session_id.as_deref());
            if let Some(csid) = csid.filter(|c| !c.is_empty()) {
                if !dir.is_empty() {
                    st.telemetry.transcripts.refresh_context(&s.id, &dir, csid);
                }
            }
        }

        let (workspace, label) = split_session_id(&s.id);
        let mut o = Map::new();
        o.insert("id".into(), json!(s.id));
        o.insert("workspace".into(), json!(workspace));
        o.insert("label".into(), json!(label));
        // The session's start dir (tmux session_path), as the JS `...s` spread.
        o.insert("dir".into(), json!(s.dir));
        o.insert("attached".into(), json!(s.attached));
        o.insert("createdAt".into(), json!(s.created_at));
        o.insert("status".into(), json!(entry.state));
        o.insert("message".into(), json!(entry.message));
        // A never-transitioned session falls back to when tmux created it.
        o.insert("since".into(), json!(entry.since.or(s.created_at)));
        o.insert("branch".into(), json!(branch_of(&dir)));
        o.insert(
            "title".into(),
            json!(normalize_title(
                pane.map(|p| p.title.as_str()),
                hostname.as_deref()
            )),
        );
        o.insert("context".into(), context.unwrap_or(Value::Null));
        out.push(Value::Object(o));
    }

    for (id, meta, dir) in restorable_in {
        let mut o = Map::new();
        o.insert("id".into(), json!(id));
        if let Some(w) = &meta.workspace {
            o.insert("workspace".into(), json!(w));
        }
        if let Some(l) = &meta.label {
            o.insert("label".into(), json!(l));
        }
        o.insert("dir".into(), json!(dir));
        o.insert("attached".into(), json!(false));
        o.insert("status".into(), json!("restorable"));
        o.insert("since".into(), Value::Null);
        o.insert("message".into(), Value::Null);
        o.insert("title".into(), Value::Null);
        o.insert("restorable".into(), json!(true));
        o.insert("branch".into(), json!(dir.as_deref().and_then(branch_of)));
        o.insert("context".into(), Value::Null);
        out.push(Value::Object(o));
    }
    Json(Value::Array(out)).into_response()
}

// ---------------------------------------------------------------------
// POST /api/sessions
// ---------------------------------------------------------------------

pub async fn create(State(st): State<AppState>, body: Bytes) -> Response {
    let body = match parse_body(&body) {
        Ok(b) => b,
        Err(r) => return *r,
    };
    let workspace = str_of(body.get("workspace")).unwrap_or("");
    let label = str_of(body.get("label")).unwrap_or("");
    if !is_valid_name(workspace) || !is_valid_name(label) {
        return error_response(
            StatusCode::BAD_REQUEST,
            "workspace and label must match [a-z0-9-]+",
        );
    }
    let Some(registered) = st.registry.get_workspace(workspace).await else {
        return error_response(
            StatusCode::NOT_FOUND,
            format!("unknown workspace: {workspace}"),
        );
    };
    let reg_dir = registered.dir.clone().unwrap_or_default();
    if !is_dir(&reg_dir).await {
        let shown = registered.dir.as_deref().unwrap_or("undefined");
        return error_response(
            StatusCode::BAD_REQUEST,
            format!("registered dir no longer exists: {shown}"),
        );
    }
    let id = session_id(workspace, label);
    if st.backend.has(&id).await.unwrap_or(false) {
        return error_response(
            StatusCode::CONFLICT,
            format!("session already exists: {id}"),
        );
    }

    // Worktree creation happens before any tmux state exists, so a failure
    // has nothing to clean up.
    let mut worktree = None;
    if truthy(body.get("worktree")) {
        match create_worktree(&st.config.garage_dir, &reg_dir, workspace, label).await {
            Ok(w) => worktree = Some((w.path, w.branch)),
            Err(e) => return error_response(StatusCode::BAD_REQUEST, e.message),
        }
    }

    let spawn_dir = worktree
        .as_ref()
        .map(|(p, _)| p.clone())
        .unwrap_or_else(|| reg_dir.clone());
    if let Err(e) = st
        .backend
        .create(&id, Path::new(&spawn_dir), &st.claude.program, &[], &[])
        .await
    {
        // The worktree was made for this session: do not leave it behind.
        if let Some((path, branch)) = &worktree {
            remove_worktree(&reg_dir, path, branch).await;
        }
        return server_error(e);
    }

    if let Some((path, branch)) = &worktree {
        // Written at spawn, not by the poller; the poller's own upserts merge
        // alongside without clobbering it.
        let upd = MetaUpdate {
            workspace: Some(workspace.to_owned()),
            label: Some(label.to_owned()),
            worktree: Some(Some(Worktree {
                path: path.clone(),
                branch: branch.clone(),
                repo_dir: reg_dir.clone(),
                target: None,
            })),
            ..Default::default()
        };
        if let Err(e) = st.registry.upsert_session_meta(&id, upd).await {
            return server_error(e);
        }
    }

    if let Some((path, branch)) = &worktree {
        st.bus.publish(crate::bus::DaemonEvent::WorktreeCreated {
            workspace: workspace.to_owned(),
            session: id.clone(),
            path: path.clone(),
            branch: branch.clone(),
            repo_dir: reg_dir.clone(),
        });
    }
    let wt = worktree.map(|(path, branch)| json!({"path": path, "branch": branch})).unwrap_or(Value::Null);
    (
        StatusCode::CREATED,
        Json(json!({"id": id, "workspace": workspace, "label": label, "dir": spawn_dir, "worktree": wt})),
    )
        .into_response()
}

// ---------------------------------------------------------------------
// POST /api/sessions/restore
// ---------------------------------------------------------------------

enum Outcome {
    Done(Value),
    Failed(Value),
}

pub async fn restore(State(st): State<AppState>, body: Bytes) -> Response {
    let body = match parse_body(&body) {
        Ok(b) => b,
        Err(r) => return *r,
    };
    let id = str_of(body.get("id")).filter(|s| !s.is_empty());
    let all = truthy(body.get("all"));
    if id.is_none() && !all {
        return error_response(StatusCode::BAD_REQUEST, "must provide id or all:true");
    }

    let metas = st.registry.list_session_metas().await;
    // (id, metadata, the dir a closed record remembers). Restore-all covers
    // metadata only: a deliberately closed session is restored by naming it.
    let targets: Vec<(String, SessionMeta, Option<String>)> = if all {
        let live: std::collections::HashSet<String> =
            st.backend.list().await.unwrap_or_default().into_iter().map(|s| s.id).collect();
        metas.into_iter().filter(|(mid, _)| !live.contains(mid)).map(|(i, m)| (i, m, None)).collect()
    } else {
        let id = id.unwrap_or_default();
        match metas.into_iter().find(|(mid, _)| mid == id) {
            Some((i, m)) => vec![(i, m, None)],
            None => match st.registry.get_closed(id).await {
                // No metadata (a deliberate close removed it) but a closed
                // record: restore from the record.
                Some(rec) => {
                    let (workspace, label) = split_session_id(&rec.id);
                    let meta = SessionMeta {
                        claude_session_id: rec.claude_session_id.clone(),
                        workspace: Some(workspace),
                        label: Some(label),
                        worktree: rec.worktree.clone(),
                    };
                    vec![(rec.id.clone(), meta, Some(rec.dir.clone()))]
                }
                None => return error_response(StatusCode::NOT_FOUND, format!("no resume metadata for: {id}")),
            },
        }
    };

    // Independent targets restore concurrently, so the died-on-resume
    // confirmation waits do not serialise restore-all.
    let spawner = BackendSpawn { backend: st.backend.as_ref(), kind: Kind::Create };
    let results = join_all(targets.into_iter().map(|(mid, meta, closed_dir)| {
        let st = &st;
        let spawner = &spawner;
        async move {
            let registered = match &meta.workspace {
                Some(ws) => st.registry.get_workspace(ws).await,
                None => None,
            };
            let failed = |reason: &str| Ok(Outcome::Failed(json!({"id": mid, "reason": reason})));
            let Some(registered) = registered else {
                return failed("workspace no longer registered");
            };

            // A worktree session restores INTO its worktree. Metadata is kept
            // on every failure so a retry (or a manual finish) can recover.
            let mut spawn_dir = meta.worktree.as_ref().map(|w| w.path.clone());
            if spawn_dir.is_none() {
                if let Some(d) = closed_dir.as_deref() {
                    if is_dir(d).await {
                        spawn_dir = Some(d.to_owned());
                    }
                }
            }
            let spawn_dir = spawn_dir.or_else(|| registered.dir.clone()).unwrap_or_default();
            if !is_dir(&spawn_dir).await {
                return failed(if meta.worktree.is_some() {
                    "worktree missing"
                } else {
                    "registered dir no longer exists"
                });
            }
            if st.backend.has(&mid).await.unwrap_or(false) {
                return failed("session already running");
            }
            let resumed =
                spawn_claude_resumed(spawner, &st.claude, &mid, Path::new(&spawn_dir), meta.claude_session_id.as_deref()).await?;
            if closed_dir.is_some() {
                // Restored from a record: the session has metadata again, so
                // a later restart or restore can resume it.
                let _ = st
                    .registry
                    .upsert_session_meta(
                        &mid,
                        MetaUpdate {
                            claude_session_id: Some(meta.claude_session_id.clone()),
                            workspace: meta.workspace.clone(),
                            label: meta.label.clone(),
                            worktree: meta.worktree.clone().map(Some),
                            ..Default::default()
                        },
                    )
                    .await;
            }
            anyhow::Ok(Outcome::Done(json!({
                "id": mid,
                "workspace": meta.workspace,
                "label": meta.label,
                "dir": spawn_dir,
                "worktree": worktree_json(&meta.worktree),
                "resumed": resumed,
            })))
        }
    }))
    .await;

    let (mut restored, mut failed) = (Vec::new(), Vec::new());
    for r in results {
        match r {
            // A tmux failure rejects the whole batch (a 500 in JS too).
            Err(e) => return server_error(e),
            Ok(Outcome::Done(v)) => restored.push(v),
            Ok(Outcome::Failed(v)) => failed.push(v),
        }
    }
    for r in &restored {
        if let Some(rid) = r.get("id").and_then(Value::as_str) {
            let _ = st.registry.remove_closed(rid).await;
        }
    }
    let code = if restored.is_empty() {
        StatusCode::CONFLICT
    } else if all {
        StatusCode::OK
    } else {
        StatusCode::CREATED
    };
    (code, Json(json!({"restored": restored, "failed": failed}))).into_response()
}

// ---------------------------------------------------------------------
// POST /api/sessions/restart
// ---------------------------------------------------------------------

pub async fn restart(State(st): State<AppState>, body: Bytes) -> Response {
    let body = match parse_body(&body) {
        Ok(b) => b,
        Err(r) => return *r,
    };
    let id = str_of(body.get("id")).filter(|s| !s.is_empty());
    let all = truthy(body.get("all"));
    let force = truthy(body.get("force"));
    if id.is_none() && !all {
        return error_response(StatusCode::BAD_REQUEST, "must provide id or all:true");
    }

    let live = st.backend.list().await.unwrap_or_default();
    if let Some(id) = id {
        if !live.iter().any(|s| s.id == id) {
            return error_response(StatusCode::NOT_FOUND, format!("no live session: {id}"));
        }
    }
    let scoped: Vec<&BackendSession> = match id {
        Some(id) => live.iter().filter(|s| s.id == id).collect(),
        None => live.iter().collect(),
    };

    // `get` reads an unobserved id as idle, which is wrong right after a fresh
    // successor daemon boots: await one real poll first so a working session
    // is not restarted without `force`.
    let ids: Vec<String> = scoped.iter().map(|s| s.id.clone()).collect();
    if needs_poll_before_plan(&ids, |i| st.status.has(i)) {
        st.shared_poller().poll_once().await;
    }

    let candidates: Vec<Candidate> = ids
        .iter()
        .map(|i| Candidate {
            id: i.clone(),
            // Still unobserved after the poll: "unknown", never idle.
            status: if st.status.has(i) {
                status_name(st.status.get(i))
            } else {
                "unknown".into()
            },
        })
        .collect();
    let (targets, skipped) = plan_restart_targets(&candidates, force);

    let metas: HashMap<String, _> = st.registry.list_session_metas().await.into_iter().collect();
    let spawner = BackendSpawn { backend: st.backend.as_ref(), kind: Kind::Respawn };
    // A restart may recreate a session whose resume died: that vanish is part
    // of the restart, not a crash.
    for t in &targets {
        st.closing.mark(t, crate::closing::CloseReason::DaemonRestart);
    }
    let results = join_all(targets.into_iter().map(|target| {
        let st = &st;
        let spawner = &spawner;
        let live = live.iter().find(|s| s.id == target);
        let meta = metas.get(&target);
        async move {
            let run = async {
                // Same dir resolution as restore; the live tmux dir only for a
                // session the poller has not recorded yet.
                let (ws, _) = split_session_id(&target);
                let registered = st.registry.get_workspace(&ws).await.and_then(|w| w.dir);
                let dir = meta
                    .and_then(|m| m.worktree.as_ref().map(|w| w.path.clone()))
                    .or(registered)
                    .or_else(|| live.map(|l| l.dir.clone()))
                    .unwrap_or_default();
                let resumed = spawn_claude_resumed(
                    spawner,
                    &st.claude,
                    &target,
                    Path::new(&dir),
                    meta.and_then(|m| m.claude_session_id.as_deref()),
                )
                .await?;
                // The pane runs a new process: the old status and statusline
                // context describe nothing now.
                st.status.drop_session(&target);
                st.telemetry.context.drop_session(&target);
                anyhow::Ok(resumed)
            };
            match run.await {
                Ok(resumed) => Ok(json!({"id": target, "resumed": resumed})),
                Err(e) => Err(json!({"id": target, "error": e.to_string()})),
            }
        }
    }))
    .await;

    // Markers of restarted sessions stay: the poller reads them when it sees
    // the old process vanish (up to one tick after join_all returns) and the
    // marker TTL forgets any that never get used. Only a target whose restart
    // failed outright has nothing to explain, so its marker goes.
    let (mut restarted, mut failed) = (Vec::new(), Vec::new());
    for r in results {
        match r {
            Ok(v) => restarted.push(v),
            Err(v) => {
                if let Some(id) = v.get("id").and_then(Value::as_str) {
                    st.closing.clear(id);
                }
                failed.push(v);
            }
        }
    }
    // respawn-pane never changes the session-id set the poller diffs on, so
    // push the event explicitly or the wall waits for an unrelated one.
    if !restarted.is_empty() {
        st.bus.publish(crate::bus::DaemonEvent::SessionsChanged);
    }
    let skipped: Vec<Value> = skipped
        .into_iter()
        .map(|c| json!({"id": c.id, "status": c.status}))
        .collect();
    Json(json!({"restarted": restarted, "skipped": skipped, "failed": failed})).into_response()
}

// ---------------------------------------------------------------------
// DELETE /api/sessions/{*id}
// ---------------------------------------------------------------------

pub async fn remove(
    State(st): State<AppState>,
    UrlPath(id): UrlPath<String>,
    Query(q): Query<HashMap<String, String>>,
) -> Response {
    if !id.starts_with(GARAGE_PREFIX) {
        return error_response(
            StatusCode::FORBIDDEN,
            "refusing to touch non-garage sessions",
        );
    }
    let live = st.backend.has(&id).await.unwrap_or(false);

    // `?meta=1` deletes ONLY the stored resume metadata of a non-live
    // (restorable) session; the worktree record rides the response so the
    // caller can still surface "worktree kept".
    if q.get("meta").map(String::as_str) == Some("1") {
        if live {
            return error_response(
                StatusCode::CONFLICT,
                format!("session is live — delete it without meta=1: {id}"),
            );
        }
        let Some(meta) = st.registry.get_session_meta(&id).await else {
            return error_response(
                StatusCode::NOT_FOUND,
                format!("no resume metadata for: {id}"),
            );
        };
        let _ = st.registry.remove_session_meta(&id).await;
        let _ = st.registry.remove_closed(&id).await;
        let worktree = match &meta.worktree {
            Some(w) => with_target(w).await,
            None => Value::Null,
        };
        return Json(json!({"deleted": true, "meta": true, "worktree": worktree})).into_response();
    }

    if !live {
        return error_response(StatusCode::NOT_FOUND, format!("no such session: {id}"));
    }
    // Capture the meta BEFORE removing it: the worktree record rides the
    // response so the caller can drive worktrees/finish afterwards.
    let meta = st.registry.get_session_meta(&id).await;
    // Where the session was, for the closed record; asked before the kill.
    let live_dir = match st.backend.panes().await.ok().and_then(|p| p.get(&id).map(|p| p.cwd.clone())) {
        Some(d) => Some(d),
        None => st.backend.list().await.ok().and_then(|l| l.into_iter().find(|s| s.id == id).map(|s| s.dir)),
    };
    let last_status = status_name(st.status.get(&id));
    st.closing.mark(&id, crate::closing::CloseReason::User);
    if let Err(e) = st.backend.kill(&id).await {
        st.closing.clear(&id);
        return server_error(e);
    }
    // Keep the record BEFORE the metadata goes: the metadata is where the
    // Claude session id lives, and the record is what lets a plugin offer
    // (and restore) this session afterwards.
    if let Some(dir) = live_dir {
        let (workspace, _) = split_session_id(&id);
        let rec = crate::state::registry::ClosedRecord {
            id: id.clone(),
            workspace,
            branch: crate::git::resolve_branch(&dir).await,
            dir,
            closed_at: crate::state::status::now_ms(),
            reason: crate::closing::CloseReason::User.as_str().to_owned(),
            last_status: Some(last_status),
            claude_session_id: meta.as_ref().and_then(|m| m.claude_session_id.clone()),
            worktree: meta.as_ref().and_then(|m| m.worktree.clone()),
        };
        let _ = st.registry.record_closed(rec).await;
    }
    // A deliberate kill: no resume metadata (restore-all skips it); the
    // closed record above is the way back.
    let _ = st.registry.remove_session_meta(&id).await;
    st.telemetry.context.drop_session(&id);
    st.telemetry.transcripts.clear_cache(&id);
    // A recreated id must not show the old status/since.
    st.status.drop_session(&id);
    let worktree = match meta.as_ref().and_then(|m| m.worktree.as_ref()) {
        Some(w) => with_target(w).await,
        None => Value::Null,
    };
    Json(json!({"deleted": true, "worktree": worktree})).into_response()
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};

    use axum::body::Body;
    use axum::http::Request;
    use http_body_util::BodyExt;
    use tower::ServiceExt;

    use super::*;
    use crate::bus::DaemonEvent;
    use crate::poller::{Agent, AgentSource};
    use crate::test_support::TestTmux;
    use crate::transcript::transcript_path;
    use crate::Config;

    fn s(id: &str, status: &str) -> Candidate {
        Candidate {
            id: id.into(),
            status: status.into(),
        }
    }

    // --- restart.test.js: planRestartTargets ---

    #[test]
    fn idle_and_done_sessions_are_targets_by_default() {
        let (t, sk) =
            plan_restart_targets(&[s("garage/ws/a", "idle"), s("garage/ws/b", "done")], false);
        assert_eq!(t, ["garage/ws/a", "garage/ws/b"]);
        assert!(sk.is_empty());
    }

    #[test]
    fn working_and_needs_input_are_skipped_without_force() {
        let (t, sk) = plan_restart_targets(
            &[
                s("garage/ws/a", "working"),
                s("garage/ws/b", "needs-input"),
                s("garage/ws/c", "idle"),
            ],
            false,
        );
        assert_eq!(t, ["garage/ws/c"]);
        assert_eq!(
            sk,
            [s("garage/ws/a", "working"), s("garage/ws/b", "needs-input")]
        );
    }

    #[test]
    fn force_includes_working_needs_input_and_unknown() {
        let all = [
            s("garage/ws/a", "working"),
            s("garage/ws/b", "needs-input"),
            s("garage/ws/c", "unknown"),
        ];
        let (t, sk) = plan_restart_targets(&all, true);
        assert_eq!(t, ["garage/ws/a", "garage/ws/b", "garage/ws/c"]);
        assert!(sk.is_empty());
    }

    #[test]
    fn unknown_status_is_skipped_without_force_never_planned_as_idle() {
        let (t, sk) = plan_restart_targets(
            &[s("garage/ws/a", "unknown"), s("garage/ws/b", "idle")],
            false,
        );
        assert_eq!(t, ["garage/ws/b"]);
        assert_eq!(sk, [s("garage/ws/a", "unknown")]);
    }

    // --- restart.test.js: needsPollBeforePlan ---

    #[test]
    fn poll_needed_only_when_some_id_has_no_status_entry() {
        let has = |id: &str| id != "garage/ws/missing";
        let ids = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert!(!needs_poll_before_plan(
            &ids(&["garage/ws/a", "garage/ws/b"]),
            has
        ));
        assert!(needs_poll_before_plan(
            &ids(&["garage/ws/a", "garage/ws/missing"]),
            has
        ));
        assert!(!needs_poll_before_plan(&[], |_| panic!(
            "has_status must not be called for an empty list"
        )));
    }

    // --- restart.test.js: spawnClaudeResumed (no tmux: a fake spawner) ---

    struct FakeSpawn {
        calls: Mutex<Vec<(String, String, Vec<String>)>>,
        /// Answers of successive `has` calls; the last one repeats.
        has: Mutex<Vec<bool>>,
    }

    impl FakeSpawn {
        fn new(has: &[bool]) -> Self {
            Self {
                calls: Mutex::default(),
                has: Mutex::new(has.to_vec()),
            }
        }
        fn calls(&self) -> Vec<(String, String, Vec<String>)> {
            self.calls.lock().unwrap().clone()
        }
    }

    #[async_trait]
    impl Spawn for FakeSpawn {
        async fn spawn(&self, id: &str, dir: &Path, cmd: &str, args: &[String]) -> Result<()> {
            let _ = cmd;
            self.calls.lock().unwrap().push((
                id.into(),
                dir.to_string_lossy().into(),
                args.to_vec(),
            ));
            Ok(())
        }
        async fn has(&self, _id: &str) -> bool {
            let mut h = self.has.lock().unwrap();
            if h.len() > 1 {
                h.remove(0)
            } else {
                h[0]
            }
        }
    }

    fn fast() -> ClaudeCmd {
        ClaudeCmd {
            program: "claude".into(),
            settle: (Duration::ZERO, Duration::ZERO),
        }
    }

    #[tokio::test]
    async fn no_claude_session_id_spawns_plain_claude_without_touching_the_backend() {
        let f = FakeSpawn::new(&[]); // `has` would panic on an empty list: it must not be called
        let r = spawn_claude_resumed(&f, &fast(), "garage/ws/one", Path::new("/tmp/one"), None)
            .await
            .unwrap();
        assert!(!r);
        assert_eq!(
            f.calls(),
            [("garage/ws/one".to_string(), "/tmp/one".to_string(), vec![])]
        );
    }

    #[tokio::test]
    async fn a_resume_that_survives_both_checks_reports_resumed() {
        let f = FakeSpawn::new(&[true, true]);
        let r = spawn_claude_resumed(&f, &fast(), "garage/ws/one", Path::new("/d"), Some("abc"))
            .await
            .unwrap();
        assert!(r);
        assert_eq!(f.calls().len(), 1);
        assert_eq!(f.calls()[0].2, ["--resume", "abc"]);
    }

    #[tokio::test]
    async fn a_resume_that_dies_at_either_check_falls_back_to_plain_claude() {
        // Dies before the first check, and dies between the two checks.
        for has in [vec![false], vec![true, false]] {
            let f = FakeSpawn::new(&has);
            let r =
                spawn_claude_resumed(&f, &fast(), "garage/ws/one", Path::new("/d"), Some("abc"))
                    .await
                    .unwrap();
            assert!(!r, "{has:?}");
            let c = f.calls();
            assert_eq!(c.len(), 2);
            assert_eq!(
                (c[0].2.clone(), c[1].2.clone()),
                (vec!["--resume".to_string(), "abc".to_string()], vec![])
            );
        }
    }

    // --- route harness: isolated tmux, temp GARAGE_DIR / claude home, no real claude ---

    struct NoAgents(AtomicUsize);

    #[async_trait]
    impl AgentSource for NoAgents {
        async fn agents(&self) -> Vec<Agent> {
            self.0.fetch_add(1, Ordering::SeqCst);
            Vec::new()
        }
    }

    struct H {
        tmux: TestTmux,
        garage: tempfile::TempDir,
        claude_home: tempfile::TempDir,
        agents: Arc<NoAgents>,
        st: AppState,
    }

    fn harness() -> H {
        let tmux = TestTmux::new();
        let garage = tempfile::tempdir().unwrap();
        let claude_home = tempfile::tempdir().unwrap();
        let agents = Arc::new(NoAgents(AtomicUsize::new(0)));
        let config = Config {
            port: 0,
            garage_dir: garage.path().to_path_buf(),
            tmux_socket: Some(tmux.socket().to_owned()),
            tmux_control: false,
            ui_origins: vec![],
        };
        let mut st = AppState::with_agents(
            config,
            Arc::new(tmux.backend()),
            Arc::new(crate::os::UnsupportedOs),
            agents.clone(),
            claude_home.path().to_path_buf(),
        );
        // Never the real `claude`; a plain `sleep` takes no flags, so tests that
        // go through the routes install a script (see `script`).
        st.claude = ClaudeCmd {
            program: "sleep".into(),
            settle: (Duration::from_millis(250), Duration::from_millis(250)),
        };
        H {
            tmux,
            garage,
            claude_home,
            agents,
            st,
        }
    }

    impl H {
        async fn call(&self, method: &str, uri: &str, body: Option<&str>) -> (StatusCode, Value) {
            let mut req = Request::builder().method(method).uri(uri);
            if body.is_some() {
                req = req.header("content-type", "application/json");
            }
            let req = req.body(Body::from(body.unwrap_or("").to_owned())).unwrap();
            let res = crate::app(self.st.clone()).oneshot(req).await.unwrap();
            let status = res.status();
            let bytes = res.into_body().collect().await.unwrap().to_bytes();
            (
                status,
                serde_json::from_slice(&bytes).unwrap_or(Value::Null),
            )
        }

        /// An executable stand-in for `claude`; runs in the session's dir.
        fn script(&mut self, name: &str, body: &str) {
            let path = self.garage.path().join(name);
            std::fs::write(
                &path,
                format!("#!/bin/sh\n[ -n \"$GARAGE_WARM\" ] && exit 0\n{body}\n"),
            )
            .unwrap();
            crate::test_support::make_executable(&path);
            // macOS scans a new executable on its first exec, which can take
            // longer than the resume timing under test: pay that now.
            let warm = std::process::Command::new(&path)
                .env("GARAGE_WARM", "1")
                .status()
                .unwrap();
            assert!(warm.success());
            self.st.claude.program = path.to_string_lossy().into_owned();
        }

        /// A registered workspace over a fresh directory; returns the dir.
        async fn workspace(&self, name: &str) -> String {
            let dir = self.garage.path().join(format!("dir-{name}"));
            std::fs::create_dir_all(&dir).unwrap();
            let dir = dir.canonicalize().unwrap().to_string_lossy().into_owned();
            self.st.registry.upsert_workspace(name, &dir).await.unwrap();
            dir
        }

        async fn live(&self, id: &str, dir: &str) {
            self.st
                .backend
                .create(id, Path::new(dir), "sleep", &["60".into()], &[])
                .await
                .unwrap();
        }

        async fn entry(&self, id: &str) -> Value {
            let (code, body) = self.call("GET", "/api/sessions", None).await;
            assert_eq!(code, StatusCode::OK);
            body.as_array()
                .unwrap()
                .iter()
                .find(|e| e["id"] == id)
                .cloned()
                .unwrap_or_else(|| panic!("no entry {id}"))
        }
    }

    fn meta(csid: Option<&str>, ws: &str, label: &str) -> MetaUpdate {
        MetaUpdate {
            claude_session_id: csid.map(|c| Some(c.to_owned())),
            workspace: Some(ws.into()),
            label: Some(label.into()),
            ..Default::default()
        }
    }

    // --- sessions.delete-meta.test.js ---

    #[tokio::test]
    async fn meta_delete_without_stored_metadata_is_404() {
        let h = harness();
        let (c, _) = h
            .call("DELETE", "/api/sessions/garage%2Fp81%2Fone?meta=1", None)
            .await;
        assert_eq!(c, StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn plain_delete_of_a_non_live_session_is_404_and_keeps_the_meta() {
        let h = harness();
        let id = "garage/p81/one";
        h.st.registry
            .upsert_session_meta(id, meta(Some("abc-123"), "p81", "one"))
            .await
            .unwrap();
        let (c, _) = h
            .call("DELETE", "/api/sessions/garage%2Fp81%2Fone", None)
            .await;
        assert_eq!(c, StatusCode::NOT_FOUND);
        assert!(
            h.st.registry.get_session_meta(id).await.is_some(),
            "meta must be retained"
        );
    }

    #[tokio::test]
    async fn meta_delete_drops_the_metadata_and_echoes_the_worktree_with_a_null_target() {
        let h = harness();
        let id = "garage/p81/one";
        let wt = Worktree {
            path: "/tmp/wt/p81/one".into(),
            branch: "garage/one".into(),
            repo_dir: "/tmp/not-a-repo".into(),
            target: None,
        };
        h.st.registry
            .upsert_session_meta(
                id,
                MetaUpdate {
                    worktree: Some(Some(wt)),
                    ..meta(Some("c"), "p81", "one")
                },
            )
            .await
            .unwrap();
        let (c, body) = h
            .call("DELETE", "/api/sessions/garage%2Fp81%2Fone?meta=1", None)
            .await;
        assert_eq!(c, StatusCode::OK);
        assert_eq!(
            body,
            json!({"deleted": true, "meta": true, "worktree": {
                "path": "/tmp/wt/p81/one", "branch": "garage/one", "repoDir": "/tmp/not-a-repo", "target": null}})
        );
        assert!(
            h.st.registry.get_session_meta(id).await.is_none(),
            "meta must be gone"
        );
    }

    #[tokio::test]
    async fn delete_refuses_non_garage_ids_with_and_without_meta() {
        let h = harness();
        for u in [
            "/api/sessions/not-garage%2Fx?meta=1",
            "/api/sessions/work%2Fmain",
            "/api/sessions/x",
        ] {
            let (c, body) = h.call("DELETE", u, None).await;
            assert_eq!(
                (c, body),
                (
                    StatusCode::FORBIDDEN,
                    json!({"error": "refusing to touch non-garage sessions"})
                ),
                "{u}"
            );
        }
    }

    #[tokio::test]
    async fn meta_delete_of_a_live_session_is_409_and_keeps_everything() {
        let h = harness();
        let dir = h.workspace("ws").await;
        h.live("garage/ws/a", &dir).await;
        h.st.registry
            .upsert_session_meta("garage/ws/a", meta(Some("c"), "ws", "a"))
            .await
            .unwrap();
        let (c, _) = h
            .call("DELETE", "/api/sessions/garage%2Fws%2Fa?meta=1", None)
            .await;
        assert_eq!(c, StatusCode::CONFLICT);
        assert!(h.st.backend.has("garage/ws/a").await.unwrap());
        assert!(h
            .st
            .registry
            .get_session_meta("garage/ws/a")
            .await
            .is_some());
    }

    // --- sessions.title.test.js ---

    #[tokio::test]
    async fn restorable_entries_have_title_null_and_the_documented_shape() {
        let h = harness();
        let dir = h.workspace("p10").await;
        h.st.registry
            .upsert_session_meta(
                "garage/p10/restorable",
                meta(Some("abc"), "p10", "restorable"),
            )
            .await
            .unwrap();
        let e = h.entry("garage/p10/restorable").await;
        assert_eq!(
            e,
            json!({"id": "garage/p10/restorable", "workspace": "p10", "label": "restorable", "dir": dir,
                   "attached": false, "status": "restorable", "since": null, "message": null, "title": null,
                   "restorable": true, "branch": null, "context": null})
        );
    }

    #[tokio::test]
    async fn a_live_sessions_set_pane_title_surfaces_trimmed_and_a_default_title_is_null() {
        let h = harness();
        let dir = h.workspace("p10").await;
        h.live("garage/p10/live", &dir).await;
        h.live("garage/p10/default-title", &dir).await;
        h.tmux.tmux(&[
            "select-pane",
            "-t",
            "garage/p10/live",
            "-T",
            "  ✳ Refactoring the poller  ",
        ]);
        let live = h.entry("garage/p10/live").await;
        assert_eq!(live["title"], "✳ Refactoring the poller");
        assert!(live.get("restorable").is_none());
        assert_eq!(
            h.entry("garage/p10/default-title").await["title"],
            Value::Null
        );
    }

    // --- sessions.context.test.js ---

    fn assistant_line(input: u64, model: &str) -> String {
        json!({"type": "assistant", "message": {"model": model, "usage": {
            "input_tokens": input, "cache_read_input_tokens": 0, "cache_creation_input_tokens": 0}}})
        .to_string()
    }

    fn write_transcript(h: &H, dir: &str, csid: &str, content: &str) {
        let p = transcript_path(h.claude_home.path(), dir, csid);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, content).unwrap();
    }

    #[tokio::test]
    async fn context_is_null_cold_then_statusline_then_transcript_and_statusline_wins() {
        let h = harness();
        let dir = h.workspace("p11").await;

        // cold: neither statusline nor transcript meta
        h.live("garage/p11/cold", &dir).await;
        assert_eq!(h.entry("garage/p11/cold").await["context"], Value::Null);

        // statusline-fed
        h.live("garage/p11/sl", &dir).await;
        h.st.telemetry.context.set_context("garage/p11/sl", 42.0);
        assert_eq!(
            h.entry("garage/p11/sl").await["context"],
            json!({"usedPercentage": 42, "source": "statusline"})
        );

        // transcript-only: the first fetch serves the empty cache (never blocks
        // on file IO) and kicks a background read; a later fetch shows it.
        h.live("garage/p11/tr", &dir).await;
        h.st.registry
            .upsert_session_meta("garage/p11/tr", meta(Some("tr-cs"), "p11", "tr"))
            .await
            .unwrap();
        write_transcript(
            &h,
            &dir,
            "tr-cs",
            &assistant_line(150_000, "claude-haiku-4-5"),
        ); // 75% of 200k
        assert_eq!(h.entry("garage/p11/tr").await["context"], Value::Null);
        h.tmux
            .wait_for_async(|| async {
                h.entry("garage/p11/tr").await["context"]
                    == json!({"usedPercentage": 75, "source": "transcript"})
            })
            .await;

        // both present: the statusline wins over a populated transcript cache
        h.live("garage/p11/both", &dir).await;
        h.st.registry
            .upsert_session_meta("garage/p11/both", meta(Some("both-cs"), "p11", "both"))
            .await
            .unwrap();
        write_transcript(
            &h,
            &dir,
            "both-cs",
            &assistant_line(180_000, "claude-haiku-4-5"),
        ); // 90%
        h.entry("garage/p11/both").await;
        h.tmux
            .wait_for(|| {
                h.st.telemetry
                    .transcripts
                    .cached_context("garage/p11/both")
                    .is_some()
            })
            .await;
        h.st.telemetry.context.set_context("garage/p11/both", 11.0);
        assert_eq!(
            h.entry("garage/p11/both").await["context"],
            json!({"usedPercentage": 11, "source": "statusline"})
        );
    }

    #[tokio::test]
    async fn restorable_entries_ignore_a_statusline_post_for_their_id() {
        let h = harness();
        h.st.registry
            .upsert_session_meta(
                "garage/p11/restorable",
                meta(Some("r"), "p11", "restorable"),
            )
            .await
            .unwrap();
        h.st.telemetry
            .context
            .set_context("garage/p11/restorable", 99.0);
        let e = h.entry("garage/p11/restorable").await;
        assert_eq!(e["restorable"], true);
        assert_eq!(e["context"], Value::Null);
    }

    // --- GET /api/sessions: live entry shape ---

    #[tokio::test]
    async fn live_entry_has_every_field_and_since_falls_back_to_created_at() {
        let h = harness();
        let dir = h.workspace("ws").await;
        h.live("garage/ws/a", &dir).await;
        let e = h.entry("garage/ws/a").await;
        let created = e["createdAt"]
            .as_i64()
            .expect("tmux reports a creation time");
        assert_eq!(
            e["since"], created,
            "never transitioned: since is when tmux created it"
        );
        assert_eq!(
            (
                e["workspace"].as_str(),
                e["label"].as_str(),
                e["attached"].clone(),
                e["status"].as_str(),
                e["message"].clone()
            ),
            (
                Some("ws"),
                Some("a"),
                json!(false),
                Some("idle"),
                Value::Null
            )
        );
        assert_eq!(
            Path::new(e["dir"].as_str().unwrap())
                .canonicalize()
                .unwrap(),
            Path::new(&dir)
        );
        for k in ["branch", "title", "context"] {
            assert!(e.get(k).is_some(), "{k} must be present (null)");
        }
        // After a real transition `since` is the transition's.
        h.st.status
            .set_at("garage/ws/a", SessionStatus::Working, None, 1234);
        let e = h.entry("garage/ws/a").await;
        assert_eq!(
            (e["status"].as_str(), e["since"].as_i64()),
            (Some("working"), Some(1234))
        );
    }

    #[tokio::test]
    async fn restorable_dir_follows_the_registered_workspace_and_is_null_when_unregistered() {
        let h = harness();
        let dir = h.workspace("moved").await;
        h.st.registry
            .upsert_session_meta("garage/moved/a", meta(Some("c"), "moved", "a"))
            .await
            .unwrap();
        h.st.registry
            .upsert_session_meta("garage/gone/a", meta(Some("c"), "gone", "a"))
            .await
            .unwrap();
        assert_eq!(h.entry("garage/moved/a").await["dir"], dir);
        assert_eq!(h.entry("garage/gone/a").await["dir"], Value::Null);
    }

    // --- POST /api/sessions ---

    #[tokio::test]
    async fn create_validates_then_starts_a_detached_session_in_the_registered_dir() {
        let mut h = harness();
        h.script("claude.sh", "exec sleep 60");
        let dir = h.workspace("ws").await;

        for (body, code, msg) in [
            (
                r#"{"workspace":"Bad Name","label":"a"}"#,
                400,
                "workspace and label must match [a-z0-9-]+".to_string(),
            ),
            (
                r#"{"workspace":"ws"}"#,
                400,
                "workspace and label must match [a-z0-9-]+".to_string(),
            ),
            (
                r#"{"workspace":"nope","label":"a"}"#,
                404,
                "unknown workspace: nope".to_string(),
            ),
        ] {
            let (c, b) = h.call("POST", "/api/sessions", Some(body)).await;
            assert_eq!((c.as_u16(), b), (code, json!({"error": msg})), "{body}");
        }

        let (c, b) = h
            .call(
                "POST",
                "/api/sessions",
                Some(r#"{"workspace":"ws","label":"a"}"#),
            )
            .await;
        assert_eq!(c, StatusCode::CREATED);
        assert_eq!(
            b,
            json!({"id": "garage/ws/a", "workspace": "ws", "label": "a", "dir": dir, "worktree": null})
        );
        assert!(h.st.backend.has("garage/ws/a").await.unwrap());
        assert!(
            h.st.registry
                .get_session_meta("garage/ws/a")
                .await
                .is_none(),
            "meta is the poller's job unless a worktree"
        );

        let (c, b) = h
            .call(
                "POST",
                "/api/sessions",
                Some(r#"{"workspace":"ws","label":"a"}"#),
            )
            .await;
        assert_eq!(
            (c, b),
            (
                StatusCode::CONFLICT,
                json!({"error": "session already exists: garage/ws/a"})
            )
        );

        std::fs::remove_dir_all(&dir).unwrap();
        let (c, b) = h
            .call(
                "POST",
                "/api/sessions",
                Some(r#"{"workspace":"ws","label":"b"}"#),
            )
            .await;
        assert_eq!(
            (c, b),
            (
                StatusCode::BAD_REQUEST,
                json!({"error": format!("registered dir no longer exists: {dir}")})
            )
        );
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

    #[tokio::test]
    async fn worktree_session_lifecycle_create_list_delete_and_a_failed_worktree_spawns_nothing() {
        let mut h = harness();
        h.script("claude.sh", "exec sleep 60");
        let repo = h.workspace("ws").await;
        git(Path::new(&repo), &["init", "-q", "-b", "main"]);
        git(
            Path::new(&repo),
            &["commit", "-q", "--allow-empty", "-m", "init"],
        );

        let mut bus = h.st.bus.subscribe();
        let (c, b) = h.call("POST", "/api/sessions", Some(r#"{"workspace":"ws","label":"fix","worktree":true}"#)).await;
        assert_eq!(c, StatusCode::CREATED);
        let wt_path = h.garage.path().join("worktrees/ws/fix").to_string_lossy().into_owned();
        assert_eq!(
            bus.try_recv().unwrap(),
            DaemonEvent::WorktreeCreated {
                workspace: "ws".into(),
                session: "garage/ws/fix".into(),
                path: wt_path.clone(),
                branch: "garage/fix".into(),
                repo_dir: repo.clone(),
            },
            "worktree.created fires once the session exists"
        );
        assert_eq!(b["dir"], wt_path);
        assert_eq!(
            b["worktree"],
            json!({"path": wt_path, "branch": "garage/fix"})
        );
        // The record is saved at spawn, with the repo it was cut from.
        let m =
            h.st.registry
                .get_session_meta("garage/ws/fix")
                .await
                .unwrap();
        assert_eq!(
            m.worktree,
            Some(Worktree {
                path: wt_path.clone(),
                branch: "garage/fix".into(),
                repo_dir: repo.clone(),
                target: None
            })
        );
        assert_eq!(
            (m.workspace.as_deref(), m.label.as_deref()),
            (Some("ws"), Some("fix"))
        );
        // The pane runs in the worktree, so the listed branch is its branch.
        h.tmux
            .wait_for_async(|| async { h.entry("garage/ws/fix").await["branch"] == "garage/fix" })
            .await;

        // Live delete returns the record (with the repo's checked-out branch)
        // and drops the context caches along with the metadata.
        h.st.telemetry.context.set_context("garage/ws/fix", 7.0);
        let (c, b) = h
            .call("DELETE", "/api/sessions/garage%2Fws%2Ffix", None)
            .await;
        assert_eq!(c, StatusCode::OK);
        assert_eq!(
            b,
            json!({"deleted": true, "worktree": {"path": wt_path, "branch": "garage/fix", "repoDir": repo, "target": "main"}})
        );
        assert!(!h.st.backend.has("garage/ws/fix").await.unwrap());
        assert!(h
            .st
            .registry
            .get_session_meta("garage/ws/fix")
            .await
            .is_none());
        assert!(h
            .st
            .telemetry
            .context
            .statusline_context("garage/ws/fix")
            .is_none());

        // A workspace that is not a git repo: 400 with the message, nothing spawned.
        h.workspace("plain").await;
        let (c, b) = h
            .call(
                "POST",
                "/api/sessions",
                Some(r#"{"workspace":"plain","label":"x","worktree":true}"#),
            )
            .await;
        assert_eq!(c, StatusCode::BAD_REQUEST);
        assert!(
            b["error"]
                .as_str()
                .unwrap()
                .starts_with("not a git repository: "),
            "{b}"
        );
        assert!(!h.st.backend.has("garage/plain/x").await.unwrap());
    }

    #[tokio::test]
    async fn plain_live_delete_kills_the_session_and_answers_a_null_worktree() {
        let h = harness();
        let dir = h.workspace("ws").await;
        h.live("garage/ws/a", &dir).await;
        h.st.registry
            .upsert_session_meta("garage/ws/a", meta(Some("c"), "ws", "a"))
            .await
            .unwrap();
        h.st.status.set("garage/ws/a", SessionStatus::Working, None);
        // The id arrives percent-encoded (%2F) and still resolves.
        let (c, b) = h
            .call("DELETE", "/api/sessions/garage%2Fws%2Fa", None)
            .await;
        assert_eq!(
            (c, b),
            (StatusCode::OK, json!({"deleted": true, "worktree": null}))
        );
        assert!(!h.st.backend.has("garage/ws/a").await.unwrap());
        assert!(!h.st.status.has("garage/ws/a"), "the status entry goes with the session");
        assert!(h.st.registry.get_session_meta("garage/ws/a").await.is_none(), "a deliberate kill offers no restore");
        assert_eq!(
            h.st.closing.take("garage/ws/a"),
            Some(crate::closing::Intent::Reason(crate::closing::CloseReason::User)),
            "the poller will report this vanish as a user close"
        );
        // gone now: 404
        let (c, _) = h
            .call("DELETE", "/api/sessions/garage%2Fws%2Fa", None)
            .await;
        assert_eq!(c, StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn a_deliberate_close_keeps_a_record_with_the_claude_session_id_and_restore_works_from_it() {
        let mut h = harness();
        h.script("claude.sh", RESUMABLE);
        let dir = h.workspace("ws").await;
        h.live("garage/ws/a", &dir).await;
        h.st.registry.upsert_session_meta("garage/ws/a", meta(Some("conv-42"), "ws", "a")).await.unwrap();
        h.st.status.set("garage/ws/a", SessionStatus::Idle, None);

        let (c, _) = h.call("DELETE", "/api/sessions/garage%2Fws%2Fa", None).await;
        assert_eq!(c, StatusCode::OK);
        assert!(h.st.registry.get_session_meta("garage/ws/a").await.is_none(), "the metadata is gone");
        let rec = h.st.registry.get_closed("garage/ws/a").await.expect("the record was written before the metadata went");
        assert_eq!(rec.claude_session_id.as_deref(), Some("conv-42"));
        assert_eq!((rec.reason.as_str(), rec.dir.as_str(), rec.last_status.as_deref()), ("user", dir.as_str(), Some("idle")));

        // the poller's close event (a tick later) must not wipe the id
        let recorder = crate::closed::spawn(&h.st.bus, h.st.registry.clone(), h.st.backend.clone());
        h.st.bus.publish(DaemonEvent::SessionClosed {
            id: "garage/ws/a".into(),
            dir: dir.clone(),
            workspace: "ws".into(),
            reason: crate::closing::CloseReason::User,
            last_status: Some("idle".into()),
        });
        let first = rec.closed_at;
        h.tmux
            .wait_for_async(|| async { h.st.registry.get_closed("garage/ws/a").await.is_some_and(|r| r.closed_at > first) })
            .await;
        recorder.abort();
        assert_eq!(h.st.registry.get_closed("garage/ws/a").await.unwrap().claude_session_id.as_deref(), Some("conv-42"));

        // restore-all skips it (deliberately closed), naming it restores and resumes it
        let (c, b) = h.call("POST", "/api/sessions/restore", Some(r#"{"all":true}"#)).await;
        assert_eq!((c, b["restored"].as_array().unwrap().len()), (StatusCode::CONFLICT, 0));
        let (c, b) = h.call("POST", "/api/sessions/restore", Some(r#"{"id":"garage/ws/a"}"#)).await;
        assert_eq!(c, StatusCode::CREATED, "{b}");
        assert_eq!(b["restored"][0]["resumed"], true);
        assert_eq!(read_in(&dir, "args.txt", &h.tmux).await.trim(), "--resume conv-42");
        assert!(h.st.registry.get_closed("garage/ws/a").await.is_none(), "a restored session drops its record");
        assert_eq!(
            h.st.registry.get_session_meta("garage/ws/a").await.and_then(|m| m.claude_session_id).as_deref(),
            Some("conv-42"),
            "and has resume metadata again"
        );
        // neither metadata nor record: still a 404
        let (c, _) = h.call("POST", "/api/sessions/restore", Some(r#"{"id":"garage/ws/zzz"}"#)).await;
        assert_eq!(c, StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn a_closed_worktree_chat_keeps_its_worktree_and_restores_into_it_so_it_can_still_be_finished() {
        let mut h = harness();
        h.script("claude.sh", RESUMABLE);
        let dir = h.workspace("ws").await;
        let wt = h.garage.path().join("wt");
        std::fs::create_dir_all(&wt).unwrap();
        let wt = wt.canonicalize().unwrap().to_string_lossy().into_owned();
        let w = Worktree { path: wt.clone(), branch: "garage/w".into(), repo_dir: dir.clone(), target: None };
        h.live("garage/ws/w", &wt).await;
        h.st.registry
            .upsert_session_meta("garage/ws/w", MetaUpdate { worktree: Some(Some(w.clone())), ..meta(Some("wcs"), "ws", "w") })
            .await
            .unwrap();

        let (c, _) = h.call("DELETE", "/api/sessions/garage%2Fws%2Fw", None).await;
        assert_eq!(c, StatusCode::OK);
        assert!(h.st.registry.get_session_meta("garage/ws/w").await.is_none());
        assert_eq!(h.st.registry.get_closed("garage/ws/w").await.unwrap().worktree, Some(w.clone()));

        // the poller's close event arrives after the metadata is gone: the
        // record it rewrites keeps the worktree from the first one
        let recorder = crate::closed::spawn(&h.st.bus, h.st.registry.clone(), h.st.backend.clone());
        let first = h.st.registry.get_closed("garage/ws/w").await.unwrap().closed_at;
        h.st.bus.publish(DaemonEvent::SessionClosed {
            id: "garage/ws/w".into(),
            dir: wt.clone(),
            workspace: "ws".into(),
            reason: crate::closing::CloseReason::User,
            last_status: None,
        });
        h.tmux
            .wait_for_async(|| async { h.st.registry.get_closed("garage/ws/w").await.is_some_and(|r| r.closed_at > first) })
            .await;
        recorder.abort();
        assert_eq!(h.st.registry.get_closed("garage/ws/w").await.unwrap().worktree, Some(w.clone()));

        let (c, b) = h.call("POST", "/api/sessions/restore", Some(r#"{"id":"garage/ws/w"}"#)).await;
        assert_eq!(c, StatusCode::CREATED, "{b}");
        assert_eq!(b["restored"][0]["dir"], wt);
        assert_eq!(b["restored"][0]["worktree"], json!({"path": wt, "branch": "garage/w", "repoDir": dir}));
        assert_eq!(h.st.registry.get_session_meta("garage/ws/w").await.and_then(|m| m.worktree), Some(w), "so finish and delete still see the worktree");
    }

    // --- POST /api/sessions/restore ---

    const RESUMABLE: &str = "echo \"$@\" > args.txt; exec sleep 60";
    const DIES_ON_RESUME: &str =
        "if [ \"$1\" = \"--resume\" ]; then exit 1; fi; echo plain > plain.txt; exec sleep 60";

    async fn read_in(dir: &str, file: &str, tmux: &TestTmux) -> String {
        let p = Path::new(dir).join(file);
        tmux.wait_for(|| p.exists()).await;
        std::fs::read_to_string(p).unwrap()
    }

    #[tokio::test]
    async fn restore_resumes_in_the_registered_dir_and_a_resume_that_dies_falls_back_to_plain() {
        let mut h = harness();
        let dir = h.workspace("ws").await;
        h.st.registry
            .upsert_session_meta("garage/ws/a", meta(Some("abc"), "ws", "a"))
            .await
            .unwrap();

        h.script("claude.sh", RESUMABLE);
        let (c, b) = h
            .call(
                "POST",
                "/api/sessions/restore",
                Some(r#"{"id":"garage/ws/a"}"#),
            )
            .await;
        assert_eq!(c, StatusCode::CREATED);
        assert_eq!(
            b,
            json!({"restored": [{"id": "garage/ws/a", "workspace": "ws", "label": "a", "dir": dir, "worktree": null, "resumed": true}],
                   "failed": []})
        );
        assert_eq!(
            read_in(&dir, "args.txt", &h.tmux).await.trim(),
            "--resume abc"
        );
        assert!(
            h.st.registry
                .get_session_meta("garage/ws/a")
                .await
                .is_some(),
            "restore keeps the metadata"
        );

        // Resume dies immediately ("No conversation found"): plain claude, resumed:false.
        h.st.backend.kill("garage/ws/a").await.unwrap();
        h.script("dies.sh", DIES_ON_RESUME);
        let (c, b) = h
            .call(
                "POST",
                "/api/sessions/restore",
                Some(r#"{"id":"garage/ws/a"}"#),
            )
            .await;
        assert_eq!(c, StatusCode::CREATED);
        assert_eq!(b["restored"][0]["resumed"], false);
        assert_eq!(read_in(&dir, "plain.txt", &h.tmux).await.trim(), "plain");
        assert!(h.st.backend.has("garage/ws/a").await.unwrap());
    }

    #[tokio::test]
    async fn restore_without_a_claude_session_id_starts_plain_and_a_worktree_session_restores_into_its_worktree(
    ) {
        let mut h = harness();
        h.script("claude.sh", RESUMABLE);
        let dir = h.workspace("ws").await;
        let wt = h.garage.path().join("wt");
        std::fs::create_dir_all(&wt).unwrap();
        let wt = wt.canonicalize().unwrap().to_string_lossy().into_owned();
        h.st.registry
            .upsert_session_meta("garage/ws/plain", meta(None, "ws", "plain"))
            .await
            .unwrap();
        let w = Worktree {
            path: wt.clone(),
            branch: "garage/w".into(),
            repo_dir: dir.clone(),
            target: None,
        };
        h.st.registry
            .upsert_session_meta(
                "garage/ws/w",
                MetaUpdate {
                    worktree: Some(Some(w)),
                    ..meta(Some("wcs"), "ws", "w")
                },
            )
            .await
            .unwrap();

        let (c, b) = h
            .call("POST", "/api/sessions/restore", Some(r#"{"all":true}"#))
            .await;
        assert_eq!(c, StatusCode::OK, "restore-all answers 200");
        let by_id = |id: &str| {
            b["restored"]
                .as_array()
                .unwrap()
                .iter()
                .find(|r| r["id"] == id)
                .cloned()
                .unwrap()
        };
        assert_eq!(by_id("garage/ws/plain")["resumed"], false);
        assert_eq!(by_id("garage/ws/w")["resumed"], true);
        assert_eq!(by_id("garage/ws/w")["dir"], wt);
        assert_eq!(
            by_id("garage/ws/w")["worktree"],
            json!({"path": wt, "branch": "garage/w", "repoDir": dir})
        );
        assert_eq!(
            read_in(&wt, "args.txt", &h.tmux).await.trim(),
            "--resume wcs"
        );
        // the plain one ran in the registered dir with no args
        assert_eq!(read_in(&dir, "args.txt", &h.tmux).await.trim(), "");
    }

    #[tokio::test]
    async fn restore_failures_are_per_target_keep_metadata_and_answer_409_when_nothing_restored() {
        let mut h = harness();
        h.script("claude.sh", RESUMABLE);
        let dir = h.workspace("ws").await;
        let reg = &h.st.registry;
        reg.upsert_session_meta("garage/ws/ok", meta(Some("c"), "ws", "ok"))
            .await
            .unwrap();
        reg.upsert_session_meta("garage/gone/a", meta(Some("c"), "gone", "a"))
            .await
            .unwrap();

        // Spec scenario: one record's workspace was unregistered, the other restores.
        let (c, b) = h
            .call("POST", "/api/sessions/restore", Some(r#"{"all":true}"#))
            .await;
        assert_eq!(c, StatusCode::OK);
        assert_eq!(b["restored"].as_array().unwrap().len(), 1);
        assert_eq!(
            b["failed"],
            json!([{"id": "garage/gone/a", "reason": "workspace no longer registered"}])
        );

        // Single-id failures: 409 with the reason each, metadata kept.
        let wt = h
            .garage
            .path()
            .join("missing-wt")
            .to_string_lossy()
            .into_owned();
        let w = Worktree {
            path: wt,
            branch: "garage/x".into(),
            repo_dir: dir.clone(),
            target: None,
        };
        h.st.registry
            .upsert_session_meta(
                "garage/ws/wtm",
                MetaUpdate {
                    worktree: Some(Some(w)),
                    ..meta(Some("c"), "ws", "wtm")
                },
            )
            .await
            .unwrap();
        h.workspace("nodir").await;
        h.st.registry
            .upsert_session_meta("garage/nodir/a", meta(Some("c"), "nodir", "a"))
            .await
            .unwrap();
        std::fs::remove_dir_all(h.garage.path().join("dir-nodir")).unwrap();
        for (id, reason) in [
            ("garage/gone/a", "workspace no longer registered"),
            ("garage/ws/wtm", "worktree missing"),
            ("garage/nodir/a", "registered dir no longer exists"),
            ("garage/ws/ok", "session already running"), // restored above, still live
        ] {
            let (c, b) = h
                .call(
                    "POST",
                    "/api/sessions/restore",
                    Some(&json!({"id": id}).to_string()),
                )
                .await;
            assert_eq!(c, StatusCode::CONFLICT, "{id}");
            assert_eq!(
                b,
                json!({"restored": [], "failed": [{"id": id, "reason": reason}]}),
                "{id}"
            );
            assert!(
                h.st.registry.get_session_meta(id).await.is_some(),
                "{id} keeps its metadata"
            );
        }

        assert_eq!(
            h.call("POST", "/api/sessions/restore", Some("{}")).await.0,
            StatusCode::BAD_REQUEST
        );
        let (c, b) = h
            .call(
                "POST",
                "/api/sessions/restore",
                Some(r#"{"id":"garage/ws/none"}"#),
            )
            .await;
        assert_eq!(
            (c, b),
            (
                StatusCode::NOT_FOUND,
                json!({"error": "no resume metadata for: garage/ws/none"})
            )
        );
    }

    // --- POST /api/sessions/restart ---

    #[tokio::test]
    async fn restart_polls_first_skips_unknown_and_busy_and_force_respawns_in_place() {
        let mut h = harness();
        h.script("claude.sh", RESUMABLE);
        let dir = h.workspace("ws").await;
        h.live("garage/ws/a", &dir).await;
        h.st.registry
            .upsert_session_meta("garage/ws/a", meta(Some("abc"), "ws", "a"))
            .await
            .unwrap();
        let pane_pid = |h: &H| {
            h.tmux
                .tmux(&["list-panes", "-t", "garage/ws/a", "-F", "#{pane_pid}"])
        };
        let before = pane_pid(&h);

        // Never observed: one poll runs, still unknown, so it is skipped.
        assert_eq!(h.agents.0.load(Ordering::SeqCst), 0);
        let (c, b) = h
            .call("POST", "/api/sessions/restart", Some(r#"{"all":true}"#))
            .await;
        assert_eq!(c, StatusCode::OK);
        assert_eq!(
            b,
            json!({"restarted": [], "skipped": [{"id": "garage/ws/a", "status": "unknown"}], "failed": []})
        );
        assert_eq!(
            h.agents.0.load(Ordering::SeqCst),
            1,
            "one poll before planning"
        );

        // Observed but busy: no further poll, skipped with its real status.
        h.st.status.set("garage/ws/a", SessionStatus::Working, None);
        let (_, b) = h
            .call(
                "POST",
                "/api/sessions/restart",
                Some(r#"{"id":"garage/ws/a"}"#),
            )
            .await;
        assert_eq!(
            b["skipped"],
            json!([{"id": "garage/ws/a", "status": "working"}])
        );
        assert_eq!(h.agents.0.load(Ordering::SeqCst), 1);
        assert_eq!(
            pane_pid(&h),
            before,
            "a skipped session keeps running untouched"
        );

        // force: respawned in place with --resume; status and context cleared; sessions event pushed.
        h.st.telemetry.context.set_context("garage/ws/a", 50.0);
        let mut events = h.st.bus.subscribe();
        let (c, b) = h
            .call(
                "POST",
                "/api/sessions/restart",
                Some(r#"{"id":"garage/ws/a","force":true}"#),
            )
            .await;
        assert_eq!(c, StatusCode::OK);
        assert_eq!(
            b,
            json!({"restarted": [{"id": "garage/ws/a", "resumed": true}], "skipped": [], "failed": []})
        );
        assert_eq!(
            read_in(&dir, "args.txt", &h.tmux).await.trim(),
            "--resume abc"
        );
        assert_ne!(pane_pid(&h), before, "a new process");
        assert!(
            h.st.backend.has("garage/ws/a").await.unwrap(),
            "same tmux session"
        );
        assert!(!h.st.status.has("garage/ws/a"));
        assert!(h
            .st
            .telemetry
            .context
            .statusline_context("garage/ws/a")
            .is_none());
        assert_eq!(events.try_recv().unwrap(), DaemonEvent::SessionsChanged);
        // The old process vanishes under the poller a tick later: the marker
        // must still be there to name the reason.
        assert_eq!(
            h.st.closing.take("garage/ws/a"),
            Some(crate::closing::Intent::Reason(crate::closing::CloseReason::DaemonRestart)),
            "the restart marker outlives the request"
        );
    }

    #[tokio::test]
    async fn restart_input_errors_and_a_dead_resume_on_respawn_recreates_the_session() {
        let mut h = harness();
        let dir = h.workspace("ws").await;
        h.live("garage/ws/a", &dir).await;
        assert_eq!(
            h.call("POST", "/api/sessions/restart", Some("{}")).await.0,
            StatusCode::BAD_REQUEST
        );
        let (c, b) = h
            .call(
                "POST",
                "/api/sessions/restart",
                Some(r#"{"id":"garage/ws/none"}"#),
            )
            .await;
        assert_eq!(
            (c, b),
            (
                StatusCode::NOT_FOUND,
                json!({"error": "no live session: garage/ws/none"})
            )
        );

        // A resume that dies takes the one-pane session with it: the plain
        // fallback recreates it, so the session survives instead of vanishing.
        h.script("dies.sh", DIES_ON_RESUME);
        h.st.registry
            .upsert_session_meta("garage/ws/a", meta(Some("abc"), "ws", "a"))
            .await
            .unwrap();
        h.st.status.set("garage/ws/a", SessionStatus::Idle, None);
        let mut events = h.st.bus.subscribe();
        let (c, b) = h
            .call(
                "POST",
                "/api/sessions/restart",
                Some(r#"{"id":"garage/ws/a"}"#),
            )
            .await;
        assert_eq!(c, StatusCode::OK);
        assert_eq!(
            b["restarted"],
            json!([{"id": "garage/ws/a", "resumed": false}])
        );
        assert_eq!(b["failed"], json!([]));
        assert!(
            h.st.backend.has("garage/ws/a").await.unwrap(),
            "the session survives"
        );
        assert_eq!(read_in(&dir, "plain.txt", &h.tmux).await.trim(), "plain");
        assert_eq!(events.try_recv().unwrap(), DaemonEvent::SessionsChanged);
    }

    // --- create: a failed spawn must not leave its worktree behind ---

    /// Delegates to a real backend but refuses to create.
    struct FailCreate(crate::backend::tmux::TmuxBackend);

    #[async_trait]
    impl SessionBackend for FailCreate {
        async fn init(&self) -> Result<()> {
            self.0.init().await
        }
        async fn list(&self) -> Result<Vec<BackendSession>> {
            self.0.list().await
        }
        async fn panes(&self) -> Result<HashMap<String, PaneInfo>> {
            self.0.panes().await
        }
        async fn has(&self, id: &str) -> Result<bool> {
            self.0.has(id).await
        }
        async fn create(
            &self,
            _: &str,
            _: &Path,
            _: &str,
            _: &[String],
            _: &[(String, String)],
        ) -> Result<()> {
            anyhow::bail!("tmux exploded")
        }
        async fn respawn(
            &self,
            id: &str,
            d: &Path,
            c: &str,
            a: &[String],
            e: &[(String, String)],
        ) -> Result<()> {
            self.0.respawn(id, d, c, a, e).await
        }
        async fn kill(&self, id: &str) -> Result<()> {
            self.0.kill(id).await
        }
        async fn rename(&self, from: &str, to: &str) -> Result<()> {
            self.0.rename(from, to).await
        }
        async fn capture(&self, id: &str, lines: usize) -> Result<String> {
            self.0.capture(id, lines).await
        }
        fn attach_spec(&self, id: &str) -> garage_protocol::AttachSpec {
            self.0.attach_spec(id)
        }
    }

    #[tokio::test]
    async fn a_failed_spawn_after_the_worktree_was_made_removes_the_worktree_and_its_branch() {
        let mut h = harness();
        let repo = h.workspace("ws").await;
        git(Path::new(&repo), &["init", "-q", "-b", "main"]);
        git(
            Path::new(&repo),
            &["commit", "-q", "--allow-empty", "-m", "init"],
        );
        h.st.backend = Arc::new(FailCreate(h.tmux.backend()));

        let (c, _) = h
            .call(
                "POST",
                "/api/sessions",
                Some(r#"{"workspace":"ws","label":"fix","worktree":true}"#),
            )
            .await;
        assert_eq!(c, StatusCode::INTERNAL_SERVER_ERROR);
        assert!(
            !h.garage.path().join("worktrees/ws/fix").exists(),
            "worktree dir removed"
        );
        let branches = std::process::Command::new("git")
            .arg("-C")
            .arg(&repo)
            .args(["branch", "--list", "garage/*"])
            .output()
            .unwrap();
        assert!(
            String::from_utf8_lossy(&branches.stdout).trim().is_empty(),
            "branch deleted"
        );
        assert!(h
            .st
            .registry
            .get_session_meta("garage/ws/fix")
            .await
            .is_none());
    }

    // --- GET /api/events ---

    #[tokio::test]
    async fn event_stream_greets_then_relays_status_and_sessions_events() {
        let h = harness();
        let res = crate::app(h.st.clone())
            .oneshot(
                Request::builder()
                    .uri("/api/events")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        assert_eq!(res.headers()["content-type"], "text/event-stream");
        let mut body = res.into_body();

        async fn next_text(body: &mut Body) -> String {
            let frame = tokio::time::timeout(Duration::from_secs(5), body.frame())
                .await
                .expect("frame in time");
            String::from_utf8(frame.unwrap().unwrap().into_data().unwrap().to_vec()).unwrap()
        }

        assert_eq!(next_text(&mut body).await, ": connected\n\n");
        // Published after the response started: still delivered.
        h.st.status
            .set_at("garage/ws/a", SessionStatus::Working, None, 4242);
        assert_eq!(
            next_text(&mut body).await,
            "event: status\ndata: {\"id\":\"garage/ws/a\",\"status\":\"working\",\"since\":4242}\n\n"
        );
        h.st.bus.publish(DaemonEvent::SessionsChanged);
        assert_eq!(next_text(&mut body).await, "event: sessions\ndata: {}\n\n");
        h.st.status
            .set_at("garage/ws/a", SessionStatus::NeedsInput, Some("ok?"), 4300);
        assert!(next_text(&mut body)
            .await
            .contains("\"status\":\"needs-input\""));
    }
}
