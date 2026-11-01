//! The context JSON every action, event, pane and CLI run receives (design
//! section 4): `GARAGE_CONTEXT` and stdin. Missing pieces are `null`, never
//! absent, so plugin code can index without checks.
//!
//! The TUI knows the focus, so it sends it; the daemon trusts only the
//! non-authoritative fields and recomputes dir, status, branch and worktree
//! from its own registry.

use std::sync::Arc;

use async_trait::async_trait;
use serde_json::{Value, json};

use crate::backend::SessionBackend;
use crate::git::resolve_branch;
use crate::state::{Registry, StatusStore};

/// What the daemon knows about one live session.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct SessionInfo {
    pub dir: Option<String>,
    pub status: Option<String>,
    pub branch: Option<String>,
    pub worktree: Value,
}

#[async_trait]
pub trait SessionLookup: Send + Sync {
    async fn session(&self, id: &str) -> Option<SessionInfo>;
}

/// The real lookup over the backend, status store and registry.
pub struct StateLookup {
    pub backend: Arc<dyn SessionBackend>,
    pub status: StatusStore,
    pub registry: Registry,
}

#[async_trait]
impl SessionLookup for StateLookup {
    async fn session(&self, id: &str) -> Option<SessionInfo> {
        let live = self
            .backend
            .list()
            .await
            .ok()?
            .into_iter()
            .find(|s| s.id == id)?;
        let panes = self.backend.panes().await.unwrap_or_default();
        let dir = panes.get(id).map(|p| p.cwd.clone()).unwrap_or(live.dir);
        let branch = resolve_branch(&dir).await;
        let worktree = match self
            .registry
            .get_session_meta(id)
            .await
            .and_then(|m| m.worktree)
        {
            Some(w) => json!({"path": w.path, "branch": w.branch, "repo_dir": w.repo_dir}),
            None => Value::Null,
        };
        let status = serde_json::to_value(self.status.get(id))
            .ok()
            .and_then(|v| v.as_str().map(str::to_owned));
        Some(SessionInfo {
            dir: Some(dir),
            status,
            branch,
            worktree,
        })
    }
}

/// What a caller says about the trigger and the focus.
#[derive(Debug, Clone, Default)]
pub struct CtxRequest {
    pub kind: String,
    pub key: Option<String>,
    pub slot: Option<String>,
    pub action: Option<String>,
    pub args: Value,
    pub workspace: Option<String>,
    pub session: Option<String>,
    pub selection: Value,
    pub event: Option<Value>,
    /// Used when the session is no longer live (a `session.closed` event).
    pub fallback: SessionInfo,
}

impl CtxRequest {
    /// Read the TUI's run request body leniently: anything absent is null.
    pub fn from_run_body(kind: &str, action: &str, body: &Value) -> Self {
        let text = |v: Option<&Value>| {
            v.and_then(Value::as_str)
                .filter(|s| !s.is_empty())
                .map(str::to_owned)
        };
        let trigger = body.get("trigger");
        let focus = body.get("focus");
        let session = focus.and_then(|f| f.get("session"));
        Self {
            kind: text(trigger.and_then(|t| t.get("kind"))).unwrap_or_else(|| kind.to_owned()),
            key: text(trigger.and_then(|t| t.get("key"))),
            slot: text(trigger.and_then(|t| t.get("slot"))),
            action: Some(action.to_owned()),
            args: trigger
                .and_then(|t| t.get("args"))
                .cloned()
                .unwrap_or_else(|| json!({})),
            workspace: text(focus.and_then(|f| f.get("workspace"))),
            session: text(session.and_then(|s| s.get("id"))).or_else(|| text(session)),
            selection: body.get("selection").cloned().unwrap_or(Value::Null),
            event: None,
            fallback: SessionInfo::default(),
        }
    }
}

pub struct PluginFacts<'a> {
    pub id: &'a str,
    pub version: &'a str,
    pub state_dir: &'a std::path::Path,
}

/// The `ui` part of the context: what the wall reported and whether one is in view.
pub struct UiFacts {
    pub slots: Value,
    pub visible: bool,
    pub terminal: Option<String>,
    pub garage: garage_protocol::api::ContextGarage,
}

pub async fn build(
    lookup: &dyn SessionLookup,
    plugin: &PluginFacts<'_>,
    req: CtxRequest,
    ui: &UiFacts,
) -> Value {
    let (session, worktree) = match &req.session {
        Some(id) => {
            let info = lookup.session(id).await.unwrap_or(req.fallback.clone());
            (
                json!({"id": id, "dir": info.dir, "status": info.status, "branch": info.branch, "worktree": info.worktree}),
                info.worktree,
            )
        }
        None => (Value::Null, Value::Null),
    };
    let selection = |k: &str| req.selection.get(k).cloned().unwrap_or(Value::Null);
    json!({
        "api_version": 1,
        "plugin": {"id": plugin.id, "version": plugin.version, "state_dir": plugin.state_dir},
        "trigger": {"kind": req.kind, "key": req.key, "slot": req.slot, "action": req.action, "args": req.args},
        "focus": {"workspace": req.workspace, "session": session, "dir": null},
        "selection": {"session": selection("session"), "index": selection("index"), "view": selection("view")},
        "worktree": worktree,
        "event": req.event,
        "ui": {"slots": ui.slots, "visible": ui.visible, "terminal": ui.terminal},
        "garage": ui.garage,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Fixed(Option<SessionInfo>);
    #[async_trait]
    impl SessionLookup for Fixed {
        async fn session(&self, _: &str) -> Option<SessionInfo> {
            self.0.clone()
        }
    }

    fn garage() -> garage_protocol::api::ContextGarage {
        garage_protocol::api::ContextGarage {
            bin: Some("/usr/local/bin/claude-garage".into()),
            port: 4747,
            dir: "/home/u/.garage".into(),
            tmux_socket: None,
        }
    }

    fn facts() -> PluginFacts<'static> {
        PluginFacts {
            id: "p",
            version: "1.0.0",
            state_dir: std::path::Path::new("/data/p"),
        }
    }

    #[tokio::test]
    async fn the_daemon_fills_dir_status_and_branch_for_the_focused_session() {
        let info = SessionInfo {
            dir: Some("/dev/garage".into()),
            status: Some("working".into()),
            branch: Some("main".into()),
            worktree: json!({"path": "/wt", "branch": "b", "repo_dir": "/r"}),
        };
        let body = json!({
            "trigger": {"kind": "key", "key": "u"},
            "focus": {"workspace": "garage", "session": {"id": "garage/main", "dir": "/spoofed"}},
            "selection": {"session": "garage/main", "index": 2}
        });
        let req = CtxRequest::from_run_body("button", "p:refresh", &body);
        let ctx = build(
            &Fixed(Some(info)),
            &facts(),
            req,
            &UiFacts {
                slots: json!({"strip.right": {"width": 38}}),
                visible: true,
                terminal: Some("ghostty".into()),
                garage: garage(),
            },
        )
        .await;
        assert_eq!(
            ctx["focus"]["session"]["dir"], "/dev/garage",
            "the TUI's dir is not trusted"
        );
        assert_eq!(ctx["focus"]["session"]["status"], "working");
        assert_eq!(ctx["focus"]["workspace"], "garage");
        assert_eq!(
            ctx["trigger"],
            json!({"kind": "key", "key": "u", "slot": null, "action": "p:refresh", "args": {}})
        );
        assert_eq!(
            ctx["selection"],
            json!({"session": "garage/main", "index": 2, "view": null})
        );
        assert_eq!(ctx["worktree"]["branch"], "b");
        assert_eq!(ctx["ui"]["slots"]["strip.right"]["width"], 38);
        assert_eq!(ctx["ui"]["visible"], true);
        assert_eq!(ctx["ui"]["terminal"], "ghostty");
        assert_eq!(
            ctx["garage"],
            json!({"bin": "/usr/local/bin/claude-garage", "port": 4747, "dir": "/home/u/.garage", "tmux_socket": null})
        );
        assert_eq!(ctx["plugin"]["state_dir"], "/data/p");
        assert_eq!(ctx["api_version"], 1);
    }

    #[tokio::test]
    async fn everything_missing_is_null_not_absent() {
        let ctx = build(
            &Fixed(None),
            &facts(),
            CtxRequest::from_run_body("palette", "p:x", &json!({})),
            &UiFacts {
                slots: json!({}),
                visible: false,
                terminal: None,
                garage: garage(),
            },
        )
        .await;
        assert_eq!(
            ctx["focus"],
            json!({"workspace": null, "session": null, "dir": null})
        );
        assert_eq!(ctx["trigger"]["kind"], "palette");
        assert_eq!(
            ctx["selection"],
            json!({"session": null, "index": null, "view": null})
        );
        assert_eq!(
            ctx["ui"],
            json!({"slots": {}, "visible": false, "terminal": null})
        );
        for k in ["worktree", "event"] {
            assert!(ctx.get(k).is_some_and(Value::is_null), "{k}");
        }
        // a session that is no longer live falls back to what the event carried
        let mut req = CtxRequest::from_run_body(
            "event",
            "x",
            &json!({"focus": {"session": "garage/w/gone"}}),
        );
        req.fallback = SessionInfo {
            dir: Some("/was".into()),
            status: Some("idle".into()),
            ..Default::default()
        };
        let ui = UiFacts {
            slots: json!({}),
            visible: false,
            terminal: None,
            garage: garage(),
        };
        let ctx = build(&Fixed(None), &facts(), req, &ui).await;
        assert_eq!(ctx["focus"]["session"]["dir"], "/was");
        assert_eq!(ctx["focus"]["session"]["branch"], Value::Null);
    }
}
