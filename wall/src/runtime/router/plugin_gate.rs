//! The confirm rule for plugin-originated triggers (p24 design.md Gap E).
//!
//! A plugin source (`Widget`: a click on a plugin's slot item, `Plugin`: a
//! plugin's own request) may only ARM a dangerous core action; only a human
//! `Key`, `Mouse` or `Chord` may CONFIRM it. The arm lives here, separate
//! from the native `x` / `X` / `r r` machines, so a plugin trigger can never
//! reach a `press()` that confirms: it only ever calls
//! [`ArmedAction::arm`], and triggering twice just re-arms.
//!
//! Core actions take their target from the context (`session` / `workspace`,
//! seeded from the clicked region or the request's `target`), never from
//! `args`: nothing in this file reads `ctx.args`.

use crate::actions::{meta, ActionCtx, ActionId, ActionSource, ids};
use crate::state::salience::restorable_session_ids;
use crate::state::store::WallStore;

use super::{Effect, GarageRouter};

/// The pending plugin arm: what a confirming human trigger will run.
pub(in crate::runtime) struct PluginArm {
    id: ActionId,
    ctx: ActionCtx,
    key: String,
    notice: String,
}

/// The tail every arm notice carries (the human's two ways to confirm).
const CONFIRM_TAIL: &str = "? click again or press y";

/// The scope an armed action is about: its session, its workspace, or none.
fn arm_key(id: ActionId, ctx: &ActionCtx) -> String {
    let target = match id {
        ids::SESSION_CLOSE | ids::SESSION_DELETE | ids::SESSION_RESTART => ctx.session.as_deref(),
        ids::WORKSPACE_RESTART
        | ids::WORKSPACE_REMOVE
        | ids::WORKSPACE_REMOVE_KILL
        | ids::SESSION_RESTORE_ALL => ctx.workspace.as_deref(),
        _ => None,
    };
    format!("{}|{}", id.as_str(), target.unwrap_or(""))
}

fn plugin_name(source: &ActionSource) -> String {
    match source {
        ActionSource::Widget { plugin, .. } | ActionSource::Plugin { plugin, .. } => plugin.clone(),
        _ => String::new(),
    }
}

impl GarageRouter {
    /// The pending arm's key matches this trigger and is still live. Used by
    /// the mouse path to tell a click on the armed region (a human confirm)
    /// from a first click.
    pub fn plugin_arm_matches(&self, id: ActionId, ctx: &ActionCtx, now_ms: i64) -> bool {
        self.plugin_arm.as_ref().is_some_and(|a| a.key == arm_key(id, ctx))
            && self.armed_plugin.armed_live(now_ms).is_some()
    }

    /// Drop a pending plugin arm and its notice (any key but `y`).
    pub(in crate::runtime) fn disarm_plugin(&mut self) {
        self.armed_plugin.disarm();
        if let Some(arm) = self.plugin_arm.take() {
            self.notices.clear_prefix(&arm.notice);
        }
    }

    pub(in crate::runtime) fn plugin_arm_pending(&self) -> bool {
        self.plugin_arm.is_some()
    }

    /// `y` while a plugin arm is pending: the action to run, as a confirmed
    /// plugin source. `None` when nothing live is armed (the arm is dropped).
    pub(in crate::runtime) fn plugin_confirm_key(&mut self, now_ms: i64) -> Option<(ActionId, ActionCtx)> {
        let key = self.plugin_arm.as_ref()?.key.clone();
        if !self.armed_plugin.confirm(&key, now_ms) {
            self.disarm_plugin();
            return None;
        }
        let arm = self.plugin_arm.take()?;
        self.notices.clear_prefix(&arm.notice);
        let mut ctx = arm.ctx;
        ctx.source = ActionSource::Plugin { plugin: plugin_name(&ctx.source), confirmed: true };
        Some((arm.id, ctx))
    }

    /// Screen a trigger before the ordinary dispatch. Returns true when it
    /// was fully handled here (armed, refused, or confirmed and turned into
    /// effects); false to let the ordinary dispatch run it.
    pub(in crate::runtime) fn gate(
        &mut self,
        store: &mut WallStore,
        id: ActionId,
        ctx: &ActionCtx,
        now_ms: i64,
        effects: &mut Vec<Effect>,
    ) -> bool {
        let info = meta(id);
        let dangerous = info.is_some_and(|m| m.dangerous);
        if ctx.source.is_plugin_origin() {
            if info.is_some_and(|m| !m.plugin_invocable) {
                self.notices.show("that action is not available to plugins", 3000, now_ms);
                return true;
            }
            if dangerous {
                self.arm_plugin(store, id, ctx, now_ms);
                return true;
            }
            if matches!(ctx.source, ActionSource::Widget { .. }) {
                // A click on a plugin item lands like any click.
                if store.state().layer == crate::state::wall_state::KeyLayer::Engaged {
                    store.disengage();
                }
            }
            retarget(store, id, ctx);
            return false;
        }
        if let ActionSource::Plugin { confirmed: true, .. } = ctx.source {
            // Built only by `plugin_confirm_key` / the mouse confirm below.
            self.run_confirmed(store, id, ctx, now_ms, effects);
            return true;
        }
        if !dangerous {
            return false;
        }
        // A human trigger on a dangerous id: it confirms a pending plugin arm
        // of the very same action and target ...
        if self.plugin_arm_matches(id, ctx, now_ms) {
            let key = arm_key(id, ctx);
            if self.armed_plugin.confirm(&key, now_ms) {
                if let Some(arm) = self.plugin_arm.take() {
                    self.notices.clear_prefix(&arm.notice);
                    if store.state().layer == crate::state::wall_state::KeyLayer::Engaged {
                        store.disengage();
                    }
                    self.run_confirmed(store, id, &arm.ctx, now_ms, effects);
                    return true;
                }
            }
        }
        // ... and `session.delete`, which has no native double-press of its
        // own, arms on a human's first trigger instead of running at once.
        if id == ids::SESSION_DELETE {
            self.arm_plugin(store, id, ctx, now_ms);
            return true;
        }
        false
    }

    fn arm_plugin(&mut self, store: &WallStore, id: ActionId, ctx: &ActionCtx, now_ms: i64) {
        let Some(subject) = arm_subject(store, id, ctx) else {
            self.notices.show("nothing to act on", 2000, now_ms);
            return;
        };
        // Replacing an older arm clears its line first.
        if let Some(old) = self.plugin_arm.take() {
            self.notices.clear_prefix(&old.notice);
        }
        let key = arm_key(id, ctx);
        let notice = format!("{subject}{CONFIRM_TAIL}");
        self.armed_plugin.arm(&key, now_ms);
        self.notices.show(notice.clone(), 3000, now_ms);
        self.plugin_arm = Some(PluginArm { id, ctx: ctx.clone(), key, notice });
    }

    /// Run a dangerous action a human just confirmed. Targets come from the
    /// armed context, never from the focus and never from `args`.
    fn run_confirmed(
        &mut self,
        store: &mut WallStore,
        id: ActionId,
        ctx: &ActionCtx,
        now_ms: i64,
        effects: &mut Vec<Effect>,
    ) {
        let session = ctx.session.as_deref();
        match id {
            ids::SESSION_CLOSE => {
                if let Some(s) = store.state().session_by_id(session) {
                    effects.push(Effect::Close {
                        id: s.id.clone(),
                        label: s.label.clone(),
                        meta_only: !s.live(),
                    });
                }
            }
            ids::SESSION_DELETE => {
                let Some(id) = session else { return };
                if store.state().session_by_id(Some(id)).is_some_and(|s| s.live()) {
                    self.notices.show("close it first — delete forgets a closed session", 4000, now_ms);
                    return;
                }
                let label = id.rsplit('/').next().unwrap_or(id).to_owned();
                effects.push(Effect::Close { id: id.to_owned(), label, meta_only: true });
            }
            ids::SESSION_RESTART => {
                let Some(s) = store.state().session_by_id(session).filter(|s| s.live()) else {
                    self.notices.show("no live session to restart", 2000, now_ms);
                    return;
                };
                let busy = s.status == "working" || s.needs_input();
                effects.push(Effect::RestartSession {
                    id: s.id.clone(),
                    name: s.display_name().to_owned(),
                    force: busy,
                });
            }
            ids::WORKSPACE_RESTART => {
                let Some(ws) = ctx.workspace.as_deref() else { return };
                let ids: Vec<String> = store
                    .state()
                    .sessions
                    .iter()
                    .filter(|s| s.workspace == ws && (s.status == "idle" || s.status == "done"))
                    .map(|s| s.id.clone())
                    .collect();
                if ids.is_empty() {
                    self.notices
                        .show(format!("nothing to restart in {ws} (all busy or none)"), 3000, now_ms);
                    return;
                }
                self.notices
                    .show(format!("restarting {} sessions in {ws}", ids.len()), 5000, now_ms);
                effects.push(Effect::RestartWorkspace { ids });
            }
            ids::DAEMON_RESTART => {
                self.notices.show("restarting the daemon…", 5000, now_ms);
                effects.push(Effect::RestartDaemon);
            }
            ids::WORKSPACE_REMOVE | ids::WORKSPACE_REMOVE_KILL => {
                let Some(name) = ctx.workspace.clone() else { return };
                if store.state().group_by_name(Some(&name)).is_some_and(|g| g.registered) {
                    effects.push(Effect::RemoveWorkspace { name, kill: id == ids::WORKSPACE_REMOVE_KILL });
                }
            }
            ids::SESSION_RESTORE_ALL => {
                let Some(ws) = ctx.workspace.clone() else { return };
                let ids: Vec<String> = restorable_session_ids(&store.state().sessions, &ws)
                    .into_iter()
                    .filter(|id| !self.restoring().contains(id))
                    .collect();
                if ids.is_empty() {
                    self.notices.show(format!("nothing to restore in {ws}"), 2000, now_ms);
                }
                for id in ids {
                    self.restore(id, effects);
                }
            }
            ids::HOOKS_INSTALL => self.install_pressed(effects),
            _ => {}
        }
    }
}

/// "Close one", "Delete garage/ws/x" ...: what the arm notice asks about.
/// `None` when the target the action needs is missing.
fn arm_subject(store: &WallStore, id: ActionId, ctx: &ActionCtx) -> Option<String> {
    let state = store.state();
    let session_name = || -> Option<String> {
        let sid = ctx.session.as_deref()?;
        Some(match state.session_by_id(Some(sid)) {
            Some(s) => s.display_name().to_owned(),
            None => sid.rsplit('/').next().unwrap_or(sid).to_owned(),
        })
    };
    let known_session = || state.session_by_id(ctx.session.as_deref()).is_some();
    Some(match id {
        ids::SESSION_CLOSE if known_session() => format!("Close {}", session_name()?),
        ids::SESSION_RESTART if known_session() => format!("Restart {}", session_name()?),
        ids::SESSION_DELETE => format!("Delete {}", session_name()?),
        ids::WORKSPACE_RESTART => format!("Restart idle sessions in {}", ctx.workspace.as_deref()?),
        ids::WORKSPACE_REMOVE => format!("Remove workspace {}", ctx.workspace.as_deref()?),
        ids::WORKSPACE_REMOVE_KILL => {
            format!("Remove workspace {} and kill its sessions", ctx.workspace.as_deref()?)
        }
        ids::SESSION_RESTORE_ALL => format!("Restore every session in {}", ctx.workspace.as_deref()?),
        ids::DAEMON_RESTART => "Restart the daemon".to_owned(),
        ids::HOOKS_INSTALL => "Install the statusline and hooks".to_owned(),
        _ => return None,
    })
}

/// Point the focus at the clicked item's scope for the actions that act on
/// "the focused" session or workspace, so a button on an unfocused tile or
/// rail row acts on that tile or row. Every other action leaves the focus
/// alone ("a click on a plugin button does not move focus unless the action
/// does").
fn retarget(store: &mut WallStore, id: ActionId, ctx: &ActionCtx) {
    match id {
        ids::SESSION_MAXIMIZE
        | ids::SESSION_DETACH
        | ids::SESSION_OPEN_WINDOW
        | ids::OVERLAY_OPEN_VIEW_PICKER => {
            if let Some(session) = ctx.session.as_deref() {
                if store.state().focused_session_id.as_deref() != Some(session)
                    && store.state().session_by_id(Some(session)).is_some()
                {
                    store.focus_session(session);
                }
            }
        }
        ids::SESSION_SPAWN | ids::SESSION_SPAWN_WORKTREE | ids::WORKSPACE_FOCUS => {
            if let Some(ws) = ctx.workspace.as_deref() {
                if store.state().focused_workspace.as_deref() != Some(ws) {
                    store.focus_workspace_named(ws);
                }
            }
        }
        _ => {}
    }
}
