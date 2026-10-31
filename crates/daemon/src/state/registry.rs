//! Port of `daemon/src/registry.js`: `<GARAGE_DIR>/state.json` (workspaces, resume metadata) with atomic writes.
//!
//! `{workspaces:{name:{dir}}, sessions:{id:{claudeSessionId,workspace,label,worktree}}, hookToken}`.
//! Created lazily on first write; tmux stays the source of truth for live
//! sessions. The file is kept as a JSON value so unknown top-level keys
//! (`hookToken`, anything future) survive every rewrite, and key order is
//! preserved (serde_json `preserve_order`). A missing, unreadable or
//! malformed file reads as an empty registry; write errors propagate.
//! One async mutex serialises read-modify-write cycles within the process.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use garage_protocol::Worktree;
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
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
        Self { path: Arc::new(garage_dir.as_ref().join("state.json")), lock: Arc::default() }
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
        let res = std::fs::write(&tmp, body).and_then(|_| std::fs::rename(&tmp, &*self.path));
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
        state["workspaces"][name]
            .as_object()
            .map(|e| Workspace { name: name.into(), dir: e.get("dir").and_then(Value::as_str).map(str::to_owned) })
    }

    pub async fn upsert_workspace(&self, name: &str, dir: &str) -> std::io::Result<Workspace> {
        let _g = self.lock.lock().await;
        let mut state = self.read();
        state["workspaces"][name] = json!({ "dir": dir });
        self.write(&state)?;
        Ok(Workspace { name: name.into(), dir: Some(dir.into()) })
    }

    /// Moves the workspace key and rewrites every resume-metadata key that
    /// embeds the old name (`garage/<old>/<label>` -> `garage/<new>/<label>`,
    /// plus the meta's `workspace` field) in one atomic write. Trusts the
    /// caller to have validated that `new` is free. `None` when `old` is
    /// unknown.
    pub async fn rename_workspace(&self, old: &str, new: &str) -> std::io::Result<Option<Workspace>> {
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
        self.write(&state)?;
        Ok(Some(Workspace { name: new.into(), dir }))
    }

    pub async fn get_session_meta(&self, id: &str) -> Option<SessionMeta> {
        let _g = self.lock.lock().await;
        let state = self.read();
        state["sessions"].get(id).map(meta_of)
    }

    pub async fn list_session_metas(&self) -> Vec<(String, SessionMeta)> {
        let _g = self.lock.lock().await;
        let state = self.read();
        state["sessions"].as_object().unwrap().iter().map(|(id, m)| (id.clone(), meta_of(m))).collect()
    }

    /// Field-level merge, not an overwrite: the poller writes
    /// `claudeSessionId`/`workspace`/`label`, session spawn writes
    /// `worktree`, and an absent field keeps what is on disk so neither
    /// clobbers the other. Writes only when the serialised record changed.
    pub async fn upsert_session_meta(&self, id: &str, upd: MetaUpdate) -> std::io::Result<SessionMeta> {
        let _g = self.lock.lock().await;
        let mut state = self.read();
        let existing = state["sessions"].get(id).cloned();
        let ex = |k: &str| existing.as_ref().and_then(|e| e.get(k)).cloned();
        let mut next = Map::new();
        next.insert(
            "claudeSessionId".into(),
            match upd.claude_session_id {
                Some(v) => v.map(Value::String).unwrap_or(Value::Null),
                None => ex("claudeSessionId").filter(|v| !v.is_null()).unwrap_or(Value::Null),
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
                Some(v) => v.map(|w| serde_json::to_value(w).unwrap()).unwrap_or(Value::Null),
                None => ex("worktree").filter(|v| !v.is_null()).unwrap_or(Value::Null),
            },
        );
        let next = Value::Object(next);
        if let Some(e) = &existing {
            if serde_json::to_string(e).ok() == serde_json::to_string(&next).ok() {
                return Ok(meta_of(e));
            }
        }
        let meta = meta_of(&next);
        state["sessions"][id] = next;
        self.write(&state)?;
        Ok(meta)
    }

    pub async fn remove_session_meta(&self, id: &str) -> std::io::Result<()> {
        let _g = self.lock.lock().await;
        let mut state = self.read();
        if state["sessions"].as_object_mut().unwrap().shift_remove(id).is_none() {
            return Ok(());
        }
        self.write(&state)
    }

    /// Drops the registry entry AND its sessions' resume metadata (they could
    /// never restore without a registered dir); never touches tmux.
    pub async fn remove_workspace(&self, name: &str) -> std::io::Result<()> {
        let _g = self.lock.lock().await;
        let mut state = self.read();
        if state["workspaces"].as_object_mut().unwrap().shift_remove(name).is_none() {
            return Ok(());
        }
        let prefix = format!("{GARAGE_PREFIX}{name}/");
        state["sessions"].as_object_mut().unwrap().retain(|id, _| !id.starts_with(&prefix));
        self.write(&state)
    }

    /// Per-install shared secret for the hook endpoint: generated once
    /// (32 random bytes, hex) and persisted so installed hooks survive daemon
    /// restarts.
    pub async fn get_hook_token(&self) -> std::io::Result<String> {
        let _g = self.lock.lock().await;
        let mut state = self.read();
        if let Some(t) = state.get("hookToken").and_then(Value::as_str).filter(|t| t.len() >= 32) {
            return Ok(t.to_owned());
        }
        let mut bytes = [0u8; 32];
        std::fs::File::open("/dev/urandom")?.read_exact(&mut bytes)?;
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

fn meta_of(v: &Value) -> SessionMeta {
    serde_json::from_value(v.clone()).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Scratch(PathBuf);
    impl Scratch {
        fn new() -> Self {
            static N: AtomicU64 = AtomicU64::new(0);
            let p = std::env::temp_dir()
                .join(format!("garage-reg-{}-{}", std::process::id(), N.fetch_add(1, Ordering::Relaxed)));
            let _ = std::fs::remove_dir_all(&p);
            Self(p)
        }
        fn reg(&self) -> Registry {
            Registry::new(&self.0)
        }
        fn raw(&self) -> Value {
            serde_json::from_str(&std::fs::read_to_string(self.0.join("state.json")).unwrap()).unwrap()
        }
    }
    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn wt() -> Worktree {
        Worktree { path: "/w".into(), branch: "b".into(), repo_dir: "/r".into(), target: None }
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
        assert_eq!(r.get_workspace("a").await.unwrap().dir.as_deref(), Some("/dir/a"));
        assert_eq!(r.get_workspace("zzz").await, None);
        let names: Vec<_> = r.list_workspaces().await.into_iter().map(|w| w.name).collect();
        assert_eq!(names, ["a", "b"]);
        r.remove_workspace("a").await.unwrap();
        assert_eq!(r.get_workspace("a").await, None);
        assert_eq!(s.raw()["workspaces"], json!({"b": {"dir": "/dir/b"}}));
    }

    #[tokio::test]
    async fn atomic_write_leaves_no_temp_files_and_pretty_prints_two_spaces() {
        let s = Scratch::new();
        s.reg().upsert_workspace("a", "/d").await.unwrap();
        let names: Vec<_> =
            std::fs::read_dir(&s.0).unwrap().map(|e| e.unwrap().file_name().into_string().unwrap()).collect();
        assert_eq!(names, ["state.json"]);
        let raw = std::fs::read_to_string(s.0.join("state.json")).unwrap();
        assert!(raw.starts_with("{\n  \"workspaces\": {\n    \"a\": {\n      \"dir\": \"/d\""), "{raw}");
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
        r.upsert_session_meta("garage/a/x", MetaUpdate { workspace: Some("a".into()), ..Default::default() })
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
        r.upsert_session_meta(id, MetaUpdate { worktree: Some(Some(wt())), ..Default::default() }).await.unwrap();
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
        let m = r.upsert_session_meta(id, MetaUpdate { worktree: Some(None), ..Default::default() }).await.unwrap();
        assert_eq!(m.worktree, None);
        assert_eq!(s.raw()["sessions"][id]["worktree"], Value::Null);
        assert_eq!(s.raw()["sessions"][id]["claudeSessionId"], "cs-1");
    }

    #[tokio::test]
    async fn fresh_record_defaults_claude_session_id_and_worktree_to_null_and_omits_unset_fields() {
        let s = Scratch::new();
        s.reg().upsert_session_meta("garage/w/l", MetaUpdate::default()).await.unwrap();
        assert_eq!(s.raw()["sessions"]["garage/w/l"], json!({"claudeSessionId": null, "worktree": null}));
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
        r.upsert_session_meta("garage/w/a", MetaUpdate { label: Some("a".into()), ..Default::default() })
            .await
            .unwrap();
        assert_eq!(r.get_session_meta("garage/w/a").await.unwrap().label.as_deref(), Some("a"));
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
            r.upsert_session_meta(id, MetaUpdate::default()).await.unwrap();
        }
        r.remove_workspace("a").await.unwrap();
        let ids: Vec<_> = r.list_session_metas().await.into_iter().map(|(i, _)| i).collect();
        assert_eq!(ids, ["garage/ab/x"]);
    }

    #[tokio::test]
    async fn rename_workspace_moves_key_and_rewrites_session_ids_and_workspace_field() {
        let s = Scratch::new();
        let r = s.reg();
        r.upsert_workspace("old", "/d").await.unwrap();
        r.upsert_workspace("other", "/o").await.unwrap();
        for (id, ws) in [("garage/old/one", "old"), ("garage/other/two", "other")] {
            r.upsert_session_meta(id, MetaUpdate { workspace: Some(ws.into()), ..Default::default() })
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
}
