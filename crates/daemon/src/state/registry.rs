//! Port of `daemon/src/registry.js`: `<GARAGE_DIR>/state.json` (workspaces, resume metadata) with atomic writes.
//!
//! `{workspaces:{name:{dir}}, sessions:{id:{claudeSessionId,workspace,label,worktree}}, hookToken}`.
//! Created lazily on first write; tmux stays the source of truth for live
//! sessions. The file is kept as a JSON value so unknown top-level keys
//! (`hookToken`, anything future) survive every rewrite, and key order is
//! preserved (serde_json `preserve_order`). A missing, unreadable or
//! malformed file reads as an empty registry; write errors propagate.
//! One async mutex serialises read-modify-write cycles within the process.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use garage_protocol::Worktree;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use tokio::sync::Mutex;

pub const GARAGE_PREFIX: &str = "garage/";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Workspace {
    pub name: String,
    pub dir: Option<String>,
}

/// Resume metadata of one garage session.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionMeta {
    #[serde(default)]
    pub claude_session_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    #[serde(default)]
    pub worktree: Option<Worktree>,
}

/// Most closed-session records kept per workspace (oldest dropped).
pub const CLOSED_LIMIT: usize = 20;

/// What the core remembers about a session that left the backend, so a plugin
/// can offer it again (`GET /api/workspaces/{name}/closed`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClosedRecord {
    pub id: String,
    pub workspace: String,
    pub dir: String,
    pub branch: Option<String>,
    pub closed_at: i64,
    /// `user` | `crash` | `daemon-restart`.
    pub reason: String,
    pub last_status: Option<String>,
    /// The Claude conversation to `--resume` when the record is restored.
    #[serde(default)]
    pub claude_session_id: Option<String>,
    /// The worktree the session lived in, so a restored worktree chat can
    /// still be finished (merged or discarded).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub worktree: Option<Worktree>,
}

/// Field-level patch for `upsert_session_meta`. Outer `None` is JS
/// `undefined` (keep what is on disk); `Some(None)` is an explicit `null`.
#[derive(Debug, Clone, Default)]
pub struct MetaUpdate {
    pub claude_session_id: Option<Option<String>>,
    pub workspace: Option<String>,
    pub label: Option<String>,
    pub worktree: Option<Option<Worktree>>,
}

#[derive(Clone)]
pub struct Registry {
    path: Arc<PathBuf>,
    lock: Arc<Mutex<()>>,
}

impl Registry {
    pub fn new(garage_dir: impl AsRef<Path>) -> Self {
        Self {
            path: Arc::new(garage_dir.as_ref().join("state.json")),
            lock: Arc::default(),
        }
    }

    fn read(&self) -> Map<String, Value> {
        let parsed = std::fs::read_to_string(&*self.path)
            .ok()
            .and_then(|raw| serde_json::from_str::<Value>(&raw).ok());
        let mut top = match parsed {
            Some(Value::Object(m)) => m,
            _ => Map::new(),
        };
        // Defaulted in place: a pre-sessions state.json gains the key on the
        // first write; key order of existing keys is untouched.
        for key in ["workspaces", "sessions"] {
            if !matches!(top.get(key), Some(Value::Object(_))) {
                top.insert(key.into(), Value::Object(Map::new()));
            }
        }
        top
    }

    /// Temp file in the same directory, then rename: a crash leaves the old
    /// or the new complete file, never a truncated one.
    fn write(&self, state: &Map<String, Value>) -> std::io::Result<()> {
        static SEQ: AtomicU64 = AtomicU64::new(0);
        let dir = self.path.parent().expect("state.json has a parent");
        std::fs::create_dir_all(dir)?;
        let tmp = dir.join(format!(
            ".state.json.{}.{}.tmp",
            std::process::id(),
            SEQ.fetch_add(1, Ordering::Relaxed)
        ));
        let body = serde_json::to_string_pretty(state).map_err(std::io::Error::other)?;
        // state.json holds the hook token: owner-only, however it was created.
        let res = crate::plugins::layout::write_private(&tmp, &body)
            .and_then(|_| std::fs::rename(&tmp, &*self.path));
        if res.is_err() {
            let _ = std::fs::remove_file(&tmp);
        }
        res
    }

    pub async fn list_workspaces(&self) -> Vec<Workspace> {
        let _g = self.lock.lock().await;
        let state = self.read();
        state["workspaces"]
            .as_object()
            .unwrap()
            .iter()
            .map(|(name, e)| Workspace {
                name: name.clone(),
                dir: e.get("dir").and_then(Value::as_str).map(str::to_owned),
            })
            .collect()
    }

    pub async fn get_workspace(&self, name: &str) -> Option<Workspace> {
        let _g = self.lock.lock().await;
        let state = self.read();
        state["workspaces"][name].as_object().map(|e| Workspace {
            name: name.into(),
            dir: e.get("dir").and_then(Value::as_str).map(str::to_owned),
        })
    }

    pub async fn upsert_workspace(&self, name: &str, dir: &str) -> std::io::Result<Workspace> {
        let _g = self.lock.lock().await;
        let mut state = self.read();
        state["workspaces"][name] = json!({ "dir": dir });
        self.write(&state)?;
        Ok(Workspace {
            name: name.into(),
            dir: Some(dir.into()),
        })
    }

    /// Moves the workspace key and rewrites every resume-metadata key that
    /// embeds the old name (`garage/<old>/<label>` -> `garage/<new>/<label>`,
    /// plus the meta's `workspace` field) in one atomic write. Trusts the
    /// caller to have validated that `new` is free. `None` when `old` is
    /// unknown.
    pub async fn rename_workspace(
        &self,
        old: &str,
        new: &str,
    ) -> std::io::Result<Option<Workspace>> {
        let _g = self.lock.lock().await;
        let mut state = self.read();
        let mut workspaces = obj_of(&mut state, "workspaces");
        let Some(entry) = workspaces.shift_remove(old) else {
            return Ok(None);
        };
        let dir = entry.get("dir").and_then(Value::as_str).map(str::to_owned);
        workspaces.insert(new.into(), entry);
        let old_prefix = format!("{GARAGE_PREFIX}{old}/");
        let new_prefix = format!("{GARAGE_PREFIX}{new}/");
        let mut next = Map::new();
        for (id, meta) in obj_of(&mut state, "sessions") {
            if let Some(rest) = id.strip_prefix(&old_prefix) {
                let mut meta = meta;
                if let Some(m) = meta.as_object_mut() {
                    m.insert("workspace".into(), Value::String(new.into()));
                }
                next.insert(format!("{new_prefix}{rest}"), meta);
            } else {
                next.insert(id, meta);
            }
        }
        state.insert("workspaces".into(), Value::Object(workspaces));
        state.insert("sessions".into(), Value::Object(next));
        let mut closed = obj_of(&mut state, "closed");
        if let Some(Value::Array(records)) = closed.shift_remove(old) {
            let moved = records
                .into_iter()
                .map(|mut r| {
                    if let Some(o) = r.as_object_mut() {
                        let id = o.get("id").and_then(Value::as_str).unwrap_or("").to_owned();
                        if let Some(rest) = id.strip_prefix(&old_prefix) {
                            o.insert("id".into(), Value::String(format!("{new_prefix}{rest}")));
                        }
                        o.insert("workspace".into(), Value::String(new.into()));
                    }
                    r
                })
                .collect();
            closed.insert(new.into(), Value::Array(moved));
            state.insert("closed".into(), Value::Object(closed));
        }
        self.write(&state)?;
        Ok(Some(Workspace {
            name: new.into(),
            dir,
        }))
    }

    pub async fn get_session_meta(&self, id: &str) -> Option<SessionMeta> {
        let _g = self.lock.lock().await;
        let state = self.read();
        state["sessions"].get(id).map(meta_of)
    }

    pub async fn list_session_metas(&self) -> Vec<(String, SessionMeta)> {
        let _g = self.lock.lock().await;
        let state = self.read();
        state["sessions"]
            .as_object()
            .unwrap()
            .iter()
            .map(|(id, m)| (id.clone(), meta_of(m)))
            .collect()
    }

    /// Field-level merge, not an overwrite: the poller writes
    /// `claudeSessionId`/`workspace`/`label`, session spawn writes
    /// `worktree`, and an absent field keeps what is on disk so neither
    /// clobbers the other. Writes only when the serialised record changed.
    pub async fn upsert_session_meta(
        &self,
        id: &str,
        upd: MetaUpdate,
    ) -> std::io::Result<SessionMeta> {
        let _g = self.lock.lock().await;
        let mut state = self.read();
        let existing = state["sessions"].get(id).cloned();
        let ex = |k: &str| existing.as_ref().and_then(|e| e.get(k)).cloned();
        let mut next = Map::new();
        next.insert(
            "claudeSessionId".into(),
            match upd.claude_session_id {
                Some(v) => v.map(Value::String).unwrap_or(Value::Null),
                None => ex("claudeSessionId")
                    .filter(|v| !v.is_null())
                    .unwrap_or(Value::Null),
            },
        );
        for (k, v) in [("workspace", upd.workspace), ("label", upd.label)] {
            if let Some(v) = v.map(Value::String).or_else(|| ex(k)) {
                next.insert(k.into(), v);
            }
        }
        next.insert(
            "worktree".into(),
            match upd.worktree {
                Some(v) => v
                    .map(|w| serde_json::to_value(w).unwrap())
                    .unwrap_or(Value::Null),
                None => ex("worktree")
                    .filter(|v| !v.is_null())
                    .unwrap_or(Value::Null),
            },
        );
        let next = Value::Object(next);
        if let Some(e) = &existing
            && serde_json::to_string(e).ok() == serde_json::to_string(&next).ok()
        {
            return Ok(meta_of(e));
        }
        let meta = meta_of(&next);
        state["sessions"][id] = next;
        self.write(&state)?;
        Ok(meta)
    }

    pub async fn remove_session_meta(&self, id: &str) -> std::io::Result<()> {
        let _g = self.lock.lock().await;
        let mut state = self.read();
        if state["sessions"]
            .as_object_mut()
            .unwrap()
            .shift_remove(id)
            .is_none()
        {
            return Ok(());
        }
        self.write(&state)
    }

    /// Remember a closed session. A record with the same id is replaced, the
    /// newest [`CLOSED_LIMIT`] per workspace are kept.
    pub async fn record_closed(&self, rec: ClosedRecord) -> std::io::Result<()> {
        let _g = self.lock.lock().await;
        let mut state = self.read();
        let mut all = obj_of(&mut state, "closed");
        let mut list: Vec<Value> = match all.remove(&rec.workspace) {
            Some(Value::Array(a)) => a,
            _ => Vec::new(),
        };
        list.retain(|r| r.get("id").and_then(Value::as_str) != Some(rec.id.as_str()));
        list.push(serde_json::to_value(&rec).map_err(std::io::Error::other)?);
        if list.len() > CLOSED_LIMIT {
            list.drain(..list.len() - CLOSED_LIMIT);
        }
        all.insert(rec.workspace.clone(), Value::Array(list));
        state.insert("closed".into(), Value::Object(all));
        self.write(&state)
    }

    /// Closed sessions of a workspace, newest first.
    pub async fn list_closed(&self, workspace: &str) -> Vec<ClosedRecord> {
        let _g = self.lock.lock().await;
        let state = self.read();
        let list = state
            .get("closed")
            .and_then(|c| c.get(workspace))
            .and_then(Value::as_array);
        list.into_iter()
            .flatten()
            .rev()
            .filter_map(|r| serde_json::from_value(r.clone()).ok())
            .collect()
    }

    /// One closed record by session id.
    pub async fn get_closed(&self, id: &str) -> Option<ClosedRecord> {
        let _g = self.lock.lock().await;
        let state = self.read();
        let (workspace, _) = crate::backend::tmux::split_session_id(id);
        state
            .get("closed")
            .and_then(|c| c.get(&workspace))
            .and_then(Value::as_array)?
            .iter()
            .find(|r| r.get("id").and_then(Value::as_str) == Some(id))
            .and_then(|r| serde_json::from_value(r.clone()).ok())
    }

    /// Forget one closed record (restored or deleted). `true` when it existed.
    pub async fn remove_closed(&self, id: &str) -> std::io::Result<bool> {
        let _g = self.lock.lock().await;
        let mut state = self.read();
        let (workspace, _) = crate::backend::tmux::split_session_id(id);
        let mut all = obj_of(&mut state, "closed");
        let Some(Value::Array(list)) = all.get_mut(&workspace) else {
            return Ok(false);
        };
        let before = list.len();
        list.retain(|r| r.get("id").and_then(Value::as_str) != Some(id));
        if list.len() == before {
            return Ok(false);
        }
        if list.is_empty() {
            all.shift_remove(&workspace);
        }
        state.insert("closed".into(), Value::Object(all));
        self.write(&state)?;
        Ok(true)
    }

    /// Drops the registry entry AND its sessions' resume metadata (they could
    /// never restore without a registered dir); never touches tmux.
    pub async fn remove_workspace(&self, name: &str) -> std::io::Result<()> {
        let _g = self.lock.lock().await;
        let mut state = self.read();
        if state["workspaces"]
            .as_object_mut()
            .unwrap()
            .shift_remove(name)
            .is_none()
        {
            return Ok(());
        }
        let prefix = format!("{GARAGE_PREFIX}{name}/");
        state["sessions"]
            .as_object_mut()
            .unwrap()
            .retain(|id, _| !id.starts_with(&prefix));
        if let Some(closed) = state.get_mut("closed").and_then(Value::as_object_mut) {
            closed.shift_remove(name);
        }
        self.write(&state)
    }

    /// Per-install shared secret for the hook endpoint: generated once
    /// (32 random bytes, hex) and persisted so installed hooks survive daemon
    /// restarts.
    pub async fn get_hook_token(&self) -> std::io::Result<String> {
        let _g = self.lock.lock().await;
        let mut state = self.read();
        if let Some(t) = state
            .get("hookToken")
            .and_then(Value::as_str)
            .filter(|t| t.len() >= 32)
        {
            return Ok(t.to_owned());
        }
        let mut bytes = [0u8; 32];
        getrandom::fill(&mut bytes).map_err(|e| std::io::Error::other(e.to_string()))?;
        let token: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
        state.insert("hookToken".into(), Value::String(token.clone()));
        self.write(&state)?;
        Ok(token)
    }
}

fn obj_of(state: &mut Map<String, Value>, key: &str) -> Map<String, Value> {
    match state.get_mut(key).map(Value::take) {
        Some(Value::Object(m)) => m,
        _ => Map::new(),
    }
}

/// Decode field by field: one malformed field reads as absent without
/// discarding its healthy siblings.
fn meta_of(v: &Value) -> SessionMeta {
    let field = |k: &str| v.get(k).filter(|f| !f.is_null());
    let text = |k: &str| field(k).and_then(Value::as_str).map(str::to_owned);
    SessionMeta {
        claude_session_id: text("claudeSessionId"),
        workspace: text("workspace"),
        label: text("label"),
        worktree: field("worktree").and_then(|w| serde_json::from_value(w.clone()).ok()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Scratch(PathBuf);
    impl Scratch {
        fn new() -> Self {
            static N: AtomicU64 = AtomicU64::new(0);
            let p = std::env::temp_dir().join(format!(
                "garage-reg-{}-{}",
                std::process::id(),
                N.fetch_add(1, Ordering::Relaxed)
            ));
            let _ = std::fs::remove_dir_all(&p);
            Self(p)
        }
        fn reg(&self) -> Registry {
            Registry::new(&self.0)
        }
        fn raw(&self) -> Value {
            serde_json::from_str(&std::fs::read_to_string(self.0.join("state.json")).unwrap())
                .unwrap()
        }
    }
    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn malformed_field_only_loses_that_field() {
        let m = meta_of(
            &json!({"claudeSessionId": "c1", "workspace": "w", "label": 7, "worktree": {"path": 1}}),
        );
        assert_eq!(m.claude_session_id.as_deref(), Some("c1"));
        assert_eq!(m.workspace.as_deref(), Some("w"));
        assert_eq!(m.label, None);
        assert_eq!(m.worktree, None);
    }

    #[tokio::test]
    async fn parallel_upserts_lose_no_writes() {
        let s = Scratch::new();
        let r = s.reg();
        let mut tasks = vec![];
        for i in 0..24 {
            let r = r.clone();
            tasks.push(tokio::spawn(async move {
                let upd = MetaUpdate {
                    label: Some(format!("l{i}")),
                    ..Default::default()
                };
                r.upsert_session_meta(&format!("garage/w/s{i}"), upd)
                    .await
                    .unwrap();
            }));
        }
        for t in tasks {
            t.await.unwrap();
        }
        assert_eq!(r.list_session_metas().await.len(), 24);
        assert_eq!(s.raw()["sessions"].as_object().unwrap().len(), 24);
    }

    fn wt() -> Worktree {
        Worktree {
            path: "/w".into(),
            branch: "b".into(),
            repo_dir: "/r".into(),
            target: None,
        }
    }

    #[tokio::test]
    async fn no_file_until_the_first_write_and_missing_or_malformed_reads_empty() {
        let s = Scratch::new();
        let r = s.reg();
        assert!(r.list_workspaces().await.is_empty());
        assert!(!s.0.join("state.json").exists());
        std::fs::create_dir_all(&s.0).unwrap();
        std::fs::write(s.0.join("state.json"), "{not json").unwrap();
        assert!(r.list_workspaces().await.is_empty());
        assert!(r.list_session_metas().await.is_empty());
    }

    #[tokio::test]
    async fn workspace_upsert_get_list_remove() {
        let s = Scratch::new();
        let r = s.reg();
        r.upsert_workspace("a", "/dir/a").await.unwrap();
        r.upsert_workspace("b", "/dir/b").await.unwrap();
        assert_eq!(
            r.get_workspace("a").await.unwrap().dir.as_deref(),
            Some("/dir/a")
        );
        assert_eq!(r.get_workspace("zzz").await, None);
        let names: Vec<_> = r
            .list_workspaces()
            .await
            .into_iter()
            .map(|w| w.name)
            .collect();
        assert_eq!(names, ["a", "b"]);
        r.remove_workspace("a").await.unwrap();
        assert_eq!(r.get_workspace("a").await, None);
        assert_eq!(s.raw()["workspaces"], json!({"b": {"dir": "/dir/b"}}));
    }

    #[tokio::test]
    async fn atomic_write_leaves_no_temp_files_and_pretty_prints_two_spaces() {
        let s = Scratch::new();
        s.reg().upsert_workspace("a", "/d").await.unwrap();
        let names: Vec<_> = std::fs::read_dir(&s.0)
            .unwrap()
            .map(|e| e.unwrap().file_name().into_string().unwrap())
            .collect();
        assert_eq!(names, ["state.json"]);
        let raw = std::fs::read_to_string(s.0.join("state.json")).unwrap();
        assert!(
            raw.starts_with("{\n  \"workspaces\": {\n    \"a\": {\n      \"dir\": \"/d\""),
            "{raw}"
        );
    }

    #[tokio::test]
    async fn pre_sessions_file_loads_and_gains_the_key_keeping_unknown_keys() {
        let s = Scratch::new();
        std::fs::create_dir_all(&s.0).unwrap();
        std::fs::write(
            s.0.join("state.json"),
            r#"{"hookToken":"t","workspaces":{"a":{"dir":"/d"}},"future":[1]}"#,
        )
        .unwrap();
        let r = s.reg();
        r.upsert_session_meta(
            "garage/a/x",
            MetaUpdate {
                workspace: Some("a".into()),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        let raw = s.raw();
        assert_eq!(raw["hookToken"], "t");
        assert_eq!(raw["future"], json!([1]));
        assert!(raw["sessions"]["garage/a/x"].is_object());
    }

    #[tokio::test]
    async fn upsert_is_a_field_level_merge_poller_and_spawn_do_not_clobber() {
        let s = Scratch::new();
        let r = s.reg();
        let id = "garage/w/l";
        // spawn writes worktree only
        r.upsert_session_meta(
            id,
            MetaUpdate {
                worktree: Some(Some(wt())),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        // poller writes the rest and never mentions worktree
        let m = r
            .upsert_session_meta(
                id,
                MetaUpdate {
                    claude_session_id: Some(Some("cs-1".into())),
                    workspace: Some("w".into()),
                    label: Some("l".into()),
                    worktree: None,
                },
            )
            .await
            .unwrap();
        assert_eq!(m.worktree, Some(wt()));
        assert_eq!(m.claude_session_id.as_deref(), Some("cs-1"));
        // an explicit null clears
        let m = r
            .upsert_session_meta(
                id,
                MetaUpdate {
                    worktree: Some(None),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        assert_eq!(m.worktree, None);
        assert_eq!(s.raw()["sessions"][id]["worktree"], Value::Null);
        assert_eq!(s.raw()["sessions"][id]["claudeSessionId"], "cs-1");
    }

    #[tokio::test]
    async fn fresh_record_defaults_claude_session_id_and_worktree_to_null_and_omits_unset_fields() {
        let s = Scratch::new();
        s.reg()
            .upsert_session_meta("garage/w/l", MetaUpdate::default())
            .await
            .unwrap();
        assert_eq!(
            s.raw()["sessions"]["garage/w/l"],
            json!({"claudeSessionId": null, "worktree": null})
        );
    }

    #[tokio::test]
    async fn unchanged_upsert_does_not_rewrite_the_file() {
        let s = Scratch::new();
        let r = s.reg();
        let upd = || MetaUpdate {
            claude_session_id: Some(Some("c".into())),
            workspace: Some("w".into()),
            label: Some("l".into()),
            worktree: None,
        };
        r.upsert_session_meta("garage/w/l", upd()).await.unwrap();
        // Tamper with formatting only; a no-op upsert must leave it alone.
        let path = s.0.join("state.json");
        let marker = format!("{}\n\n", std::fs::read_to_string(&path).unwrap());
        std::fs::write(&path, &marker).unwrap();
        r.upsert_session_meta("garage/w/l", upd()).await.unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), marker);
    }

    #[tokio::test]
    async fn get_list_remove_session_meta() {
        let s = Scratch::new();
        let r = s.reg();
        r.upsert_session_meta(
            "garage/w/a",
            MetaUpdate {
                label: Some("a".into()),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        assert_eq!(
            r.get_session_meta("garage/w/a")
                .await
                .unwrap()
                .label
                .as_deref(),
            Some("a")
        );
        assert_eq!(r.get_session_meta("garage/w/none").await, None);
        assert_eq!(r.list_session_metas().await.len(), 1);
        r.remove_session_meta("garage/w/a").await.unwrap();
        r.remove_session_meta("garage/w/a").await.unwrap(); // absent: no-op
        assert_eq!(r.get_session_meta("garage/w/a").await, None);
    }

    #[tokio::test]
    async fn remove_workspace_drops_its_session_metas_only() {
        let s = Scratch::new();
        let r = s.reg();
        r.upsert_workspace("a", "/a").await.unwrap();
        r.upsert_workspace("ab", "/ab").await.unwrap();
        for id in ["garage/a/x", "garage/ab/x"] {
            r.upsert_session_meta(id, MetaUpdate::default())
                .await
                .unwrap();
        }
        r.remove_workspace("a").await.unwrap();
        let ids: Vec<_> = r
            .list_session_metas()
            .await
            .into_iter()
            .map(|(i, _)| i)
            .collect();
        assert_eq!(ids, ["garage/ab/x"]);
    }

    #[tokio::test]
    async fn rename_workspace_moves_key_and_rewrites_session_ids_and_workspace_field() {
        let s = Scratch::new();
        let r = s.reg();
        r.upsert_workspace("old", "/d").await.unwrap();
        r.upsert_workspace("other", "/o").await.unwrap();
        for (id, ws) in [("garage/old/one", "old"), ("garage/other/two", "other")] {
            r.upsert_session_meta(
                id,
                MetaUpdate {
                    workspace: Some(ws.into()),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        }
        let w = r.rename_workspace("old", "new").await.unwrap().unwrap();
        assert_eq!((w.name.as_str(), w.dir.as_deref()), ("new", Some("/d")));
        assert_eq!(r.rename_workspace("ghost", "x").await.unwrap(), None);
        let raw = s.raw();
        assert!(raw["workspaces"].get("old").is_none());
        assert_eq!(raw["workspaces"]["new"], json!({"dir": "/d"}));
        assert_eq!(raw["sessions"]["garage/new/one"]["workspace"], "new");
        assert!(raw["sessions"].get("garage/old/one").is_none());
        assert_eq!(raw["sessions"]["garage/other/two"]["workspace"], "other");
    }

    #[tokio::test]
    async fn hook_token_is_generated_once_persisted_and_long_enough() {
        let s = Scratch::new();
        let t = s.reg().get_hook_token().await.unwrap();
        assert_eq!(t.len(), 64);
        assert!(t.bytes().all(|b| b.is_ascii_hexdigit()));
        assert_eq!(s.reg().get_hook_token().await.unwrap(), t); // fresh handle, same file
        // a short stored token is replaced
        let mut raw = s.raw();
        raw["hookToken"] = json!("short");
        std::fs::write(s.0.join("state.json"), raw.to_string()).unwrap();
        assert_ne!(s.reg().get_hook_token().await.unwrap(), "short");
    }

    fn closed(id: &str, at: i64) -> ClosedRecord {
        let (ws, _) = crate::backend::tmux::split_session_id(id);
        ClosedRecord {
            id: id.into(),
            workspace: ws,
            dir: "/d".into(),
            branch: Some("main".into()),
            closed_at: at,
            reason: "crash".into(),
            last_status: Some("idle".into()),
            claude_session_id: Some("c1".into()),
            worktree: None,
        }
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn state_json_is_owner_only_even_when_it_was_created_loose() {
        use std::os::unix::fs::PermissionsExt;
        let s = Scratch::new();
        let r = s.reg();
        r.get_hook_token().await.unwrap();
        let p = s.0.join("state.json");
        assert_eq!(
            std::fs::metadata(&p).unwrap().permissions().mode() & 0o777,
            0o600
        );
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o644)).unwrap();
        r.upsert_workspace("w", "/w").await.unwrap();
        assert_eq!(
            std::fs::metadata(&p).unwrap().permissions().mode() & 0o777,
            0o600,
            "tightened on the next write"
        );
    }

    #[tokio::test]
    async fn a_closed_record_keeps_its_claude_session_id_and_reads_old_files_without_one() {
        let s = Scratch::new();
        let r = s.reg();
        r.record_closed(closed("garage/w/a", 1)).await.unwrap();
        assert_eq!(
            r.get_closed("garage/w/a")
                .await
                .unwrap()
                .claude_session_id
                .as_deref(),
            Some("c1")
        );
        assert_eq!(s.raw()["closed"]["w"][0]["claudeSessionId"], "c1");
        assert!(r.get_closed("garage/w/zzz").await.is_none());
        let mut raw = s.raw();
        raw["closed"]["w"][0]
            .as_object_mut()
            .unwrap()
            .remove("claudeSessionId");
        std::fs::write(s.0.join("state.json"), raw.to_string()).unwrap();
        assert_eq!(
            r.get_closed("garage/w/a").await.unwrap().claude_session_id,
            None
        );
        assert_eq!(
            r.list_closed("w").await.len(),
            1,
            "a record from before the field still lists"
        );
    }

    #[tokio::test]
    async fn closed_records_list_newest_first_are_bounded_and_replace_by_id() {
        let s = Scratch::new();
        let r = s.reg();
        assert!(r.list_closed("w").await.is_empty());
        for i in 0..(CLOSED_LIMIT as i64 + 5) {
            r.record_closed(closed(&format!("garage/w/s{i}"), i))
                .await
                .unwrap();
        }
        r.record_closed(closed("garage/other/x", 1)).await.unwrap();
        let list = r.list_closed("w").await;
        assert_eq!(list.len(), CLOSED_LIMIT);
        assert_eq!(
            list[0].id,
            format!("garage/w/s{}", CLOSED_LIMIT + 4),
            "newest first"
        );
        assert_eq!(
            list.last().unwrap().id,
            "garage/w/s5",
            "the five oldest were dropped"
        );
        // the same id again replaces the old record instead of duplicating it
        r.record_closed(closed("garage/w/s10", 999)).await.unwrap();
        let list = r.list_closed("w").await;
        assert_eq!(
            (list.len(), list[0].id.as_str(), list[0].closed_at),
            (CLOSED_LIMIT, "garage/w/s10", 999)
        );
        assert_eq!(r.list_closed("other").await.len(), 1);
    }

    #[tokio::test]
    async fn removing_a_closed_record_reports_whether_it_existed_and_survives_other_keys() {
        let s = Scratch::new();
        let r = s.reg();
        r.get_hook_token().await.unwrap();
        r.record_closed(closed("garage/w/a", 1)).await.unwrap();
        r.record_closed(closed("garage/w/b", 2)).await.unwrap();
        assert!(r.remove_closed("garage/w/a").await.unwrap());
        assert!(!r.remove_closed("garage/w/a").await.unwrap());
        assert!(!r.remove_closed("garage/none/z").await.unwrap());
        assert_eq!(r.list_closed("w").await.len(), 1);
        assert!(r.remove_closed("garage/w/b").await.unwrap());
        assert!(
            s.raw()["closed"].as_object().unwrap().is_empty(),
            "an empty workspace entry is dropped"
        );
        assert_eq!(
            s.raw()["hookToken"].as_str().map(str::len),
            Some(64),
            "other keys untouched"
        );
    }

    #[tokio::test]
    async fn workspace_remove_drops_and_rename_moves_its_closed_records() {
        let s = Scratch::new();
        let r = s.reg();
        r.upsert_workspace("old", "/o").await.unwrap();
        r.upsert_workspace("gone", "/g").await.unwrap();
        r.record_closed(closed("garage/old/a", 1)).await.unwrap();
        r.record_closed(closed("garage/gone/b", 1)).await.unwrap();
        r.rename_workspace("old", "new").await.unwrap();
        let moved = r.list_closed("new").await;
        assert_eq!(
            (moved[0].id.as_str(), moved[0].workspace.as_str()),
            ("garage/new/a", "new")
        );
        assert!(r.list_closed("old").await.is_empty());
        r.remove_workspace("gone").await.unwrap();
        assert!(r.list_closed("gone").await.is_empty());
    }
}
