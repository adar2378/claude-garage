//! The run request the wall sends for a plugin action. The daemon completes
//! it (dir, status, branch, worktree, `ui.slots`) from its own registry and
//! trusts only the ids from here, so this carries the focus and the trigger
//! and nothing else. Never the UI token.

use serde_json::{Value, json};

use super::Need;
use crate::actions::{ActionCtx, ActionSource};

/// `(kind, key, slot)` of the trigger, as the daemon's context names them.
fn trigger(source: &ActionSource) -> (&'static str, Option<&str>, Option<&'static str>) {
    match source {
        ActionSource::Key(k) | ActionSource::Chord(k) => ("key", Some(k.as_str()), None),
        ActionSource::Widget { slot, .. } => ("widget", None, Some(slot.as_str())),
        ActionSource::Mouse { .. } => ("button", None, None),
        ActionSource::Plugin { .. } => ("palette", None, None),
    }
}

/// The body of `POST /api/plugins/{id}/actions/{action}/run`.
pub fn run_body(ctx: &ActionCtx) -> Value {
    let (kind, key, slot) = trigger(&ctx.source);
    json!({
        "trigger": {"kind": kind, "key": key, "slot": slot, "args": ctx.args.as_deref().cloned().unwrap_or_else(|| json!({}))},
        "focus": {
            "workspace": ctx.workspace,
            "session": ctx.session.as_ref().map(|id| json!({"id": id})),
        },
        "selection": {"session": ctx.session, "index": ctx.selection.or(ctx.index), "view": ctx.view},
    })
}

/// The action and context for a plugin's `ui_action` request. The source is
/// always `Plugin { confirmed: false }`: the frame has no way to say
/// otherwise. A core action takes its session and workspace from `target`
/// alone (never the focus, never `args`); a plugin action also gets the
/// focus, since it runs with the user's context, and its `args`. `None` when
/// no manifest or core id has that name.
pub fn ui_action_ctx(
    state: &crate::state::wall_state::WallState,
    frame: &garage_protocol::plugins::UiActionFrame,
) -> Option<(crate::actions::ActionId, ActionCtx)> {
    let id = crate::actions::ActionId::lookup(&frame.action)?;
    let mut ctx = ActionCtx::from_focus(state, ActionSource::plugin(&frame.plugin));
    if !id.is_plugin() {
        ctx.session = None;
        ctx.workspace = None;
    }
    if let Some(session) = &frame.target.session {
        ctx.session = Some(session.clone());
        if frame.target.workspace.is_none() {
            ctx.workspace = state
                .session_by_id(Some(session))
                .map(|s| s.workspace.clone())
                .or(ctx.workspace);
        }
    }
    if let Some(ws) = &frame.target.workspace {
        ctx.workspace = Some(ws.clone());
    }
    if id.is_plugin() {
        ctx.args = frame.args.clone().map(std::sync::Arc::new);
    }
    Some((id, ctx))
}

/// Why an action cannot run with this context, as a toast; `None` when it can.
pub fn unmet(need: Need, title: &str, ctx: &ActionCtx) -> Option<String> {
    match need {
        Need::Session if ctx.session.is_none() => Some(format!("{title}: focus a session first")),
        Need::Workspace if ctx.workspace.is_none() => {
            Some(format!("{title}: focus a workspace first"))
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::slots::SlotId;
    use std::sync::Arc;

    fn ctx(source: ActionSource) -> ActionCtx {
        ActionCtx {
            workspace: Some("w".into()),
            session: Some("garage/w/a".into()),
            index: None,
            selection: None,
            view: None,
            worktree: None,
            args: None,
            region: None,
            source,
        }
    }

    #[test]
    fn a_key_trigger_names_the_key_and_a_click_names_the_slot_and_carries_args() {
        let key = run_body(&ctx(ActionSource::Key("u".into())));
        assert_eq!(
            key["trigger"],
            json!({"kind": "key", "key": "u", "slot": null, "args": {}})
        );
        assert_eq!(
            key["focus"],
            json!({"workspace": "w", "session": {"id": "garage/w/a"}})
        );
        let mut c = ctx(ActionSource::Widget {
            plugin: "p".into(),
            slot: SlotId::TileTitleRight,
        });
        c.args = Some(Arc::new(json!({"keep": 2})));
        let click = run_body(&c);
        assert_eq!(
            click["trigger"],
            json!({"kind": "widget", "key": null, "slot": "tile.title.right", "args": {"keep": 2}})
        );
    }

    #[test]
    fn missing_focus_is_null_and_the_body_never_carries_a_token() {
        let mut c = ctx(ActionSource::Key("u".into()));
        c.workspace = None;
        c.session = None;
        let body = run_body(&c);
        assert_eq!(body["focus"], json!({"workspace": null, "session": null}));
        assert_eq!(
            body["selection"],
            json!({"session": null, "index": null, "view": null})
        );
        assert!(!body.to_string().to_lowercase().contains("token"));
    }

    #[test]
    fn contexts_gate_with_a_hint() {
        let mut c = ctx(ActionSource::Key("u".into()));
        assert_eq!(unmet(Need::Session, "Go", &c), None);
        c.session = None;
        assert_eq!(
            unmet(Need::Session, "Go", &c).as_deref(),
            Some("Go: focus a session first")
        );
        assert_eq!(unmet(Need::Global, "Go", &c), None);
        c.workspace = None;
        assert_eq!(
            unmet(Need::Workspace, "Go", &c).as_deref(),
            Some("Go: focus a workspace first")
        );
    }
}
