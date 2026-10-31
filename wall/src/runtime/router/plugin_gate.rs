//! The confirm rule for plugin-originated triggers (p24 design.md Gap E).
//!
//! A plugin source (`Widget`: a click on a plugin's slot item, `Plugin`: a
//! plugin's own request) may only ARM a dangerous core action; only a human
//! `Key`, `Mouse` or `Chord` may CONFIRM it. The arm lives here, separate
//! from the native `x` / `X` / `r r` machines, so a plugin trigger can never
//! reach a `press()` that confirms: it only ever calls
//! [`ArmedAction::arm`].
//!
//! An arm records its ORIGIN. A `Plugin` request (the API) is confirmable
//! only by the `y` key; a `Widget` click is confirmable by `y` or by a click
//! on the exact region (slot, rect, item identity) that armed it. A core
//! click target never confirms a plugin arm and a click anywhere else
//! cancels it. A plugin source can never replace or refresh a pending arm.
//! The arm lives only while its own prompt is the notice on screen. `y` and
//! a confirming click are ignored for the first 300 ms after the prompt
//! appears; a click also needs a frame that showed the prompt, and an early
//! click on the region is swallowed (neither confirms nor cancels). A plugin
//! API request may not move the focus while the user is engaged.
//!
//! Core actions take their target from the context (`session` / `workspace`,
//! seeded from the clicked region or the request's `target`), never from
//! `args`: nothing in this file reads `ctx.args`.

use crate::actions::{ActionCtx, ActionId, ActionSource, ids, meta};
use crate::slots::RegionId;
use crate::state::salience::restorable_session_ids;
use crate::state::store::WallStore;

use super::{Effect, GarageRouter};

/// Who raised an arm, which decides what may confirm it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(in crate::runtime) enum ArmOrigin {
    /// A plugin's own request: `y` only, never a mouse click.
    Api,
    /// A click on a plugin's slot item: `y`, or a click on this same region.
    Region(RegionId),
    /// A human trigger (`session.delete` has no native double press): the
    /// same trigger again, or `y`.
    Human,
}

/// The pending plugin arm: what a confirming human trigger will run.
pub(in crate::runtime) struct PluginArm {
    id: ActionId,
    ctx: ActionCtx,
    key: String,
    notice: String,
    origin: ArmOrigin,
    /// Restart only: decided when the prompt was raised, exactly as the
    /// native `r r` (design D2: the armed press IS the force).
    force: bool,
    armed_at: i64,
    /// A frame with this prompt on screen has been drawn: a click cannot
    /// confirm what the human has not had a chance to see.
    drawn: bool,
}

/// What `y` did while an arm was pending.
pub(in crate::runtime) enum ConfirmKey {
    Run(Box<(ActionId, ActionCtx)>),
    /// Inside the guard window: the key is swallowed and the arm stays.
    TooSoon,
    /// Nothing live to confirm (the arm, if any, is dropped).
    Gone,
}

/// What a click did while an arm was pending.
pub(in crate::runtime) enum ClickConfirm {
    Run(Box<(ActionId, ActionCtx)>),
    /// On the armed region but too early (inside the guard window, or the
    /// prompt not drawn yet): swallowed, neither confirming nor cancelling.
    Swallow,
    /// Anywhere else: the arm was dropped; the click lands as usual.
    Cancelled,
}

/// `y` and a confirming click are ignored this long after the prompt first appears, so a keypress
/// that raced the arming click cannot confirm it.
pub(in crate::runtime) const Y_GUARD_MS: i64 = 300;

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
    /// Drop a pending arm whose prompt is no longer the notice on screen
    /// (any other notice replaced it, or it timed out): an arm is valid
    /// only while the human can see what it would do.
    pub(in crate::runtime) fn refresh_plugin_arm(&mut self, now_ms: i64) {
        let showing = self
            .plugin_arm
            .as_ref()
            .map(|a| self.notices.current(now_ms) == Some(a.notice.as_str()));
        if showing == Some(false)
            || (showing.is_some() && self.armed_plugin.armed_live(now_ms).is_none())
        {
            self.disarm_plugin();
        }
    }

    /// Drop a pending plugin arm and its notice (any key but `y`).
    pub(in crate::runtime) fn disarm_plugin(&mut self) {
        self.armed_plugin.disarm();
        if let Some(arm) = self.plugin_arm.take() {
            self.notices.clear_prefix(&arm.notice);
        }
    }

    /// Called after each frame is drawn: records that the pending arm's
    /// prompt was on screen in it.
    pub(in crate::runtime) fn mark_prompt_drawn(&mut self, now_ms: i64) {
        let current = self.notices.current(now_ms).map(str::to_owned);
        if let Some(arm) = self.plugin_arm.as_mut()
            && current.as_deref() == Some(arm.notice.as_str())
        {
            arm.drawn = true;
        }
    }

    /// True when the pending arm was raised by a human trigger of `id`.
    pub(in crate::runtime) fn human_arm_is(&self, id: ActionId) -> bool {
        self.plugin_arm
            .as_ref()
            .is_some_and(|a| a.origin == ArmOrigin::Human && a.id == id)
    }

    pub(in crate::runtime) fn plugin_arm_pending(&self) -> bool {
        self.plugin_arm.is_some()
    }

    /// True when the pending arm is a plugin's (a click elsewhere cancels
    /// it); a human-raised arm is left to its own trigger.
    pub(in crate::runtime) fn plugin_origin_arm_pending(&self) -> bool {
        self.plugin_arm
            .as_ref()
            .is_some_and(|a| a.origin != ArmOrigin::Human)
    }

    /// `y` while an arm is pending. The action comes back as a confirmed
    /// plugin source; the router remembers a one-shot token so only this
    /// path can produce one the gate will honour.
    pub(in crate::runtime) fn plugin_confirm_key(&mut self, now_ms: i64) -> ConfirmKey {
        self.refresh_plugin_arm(now_ms);
        let Some(arm) = self.plugin_arm.as_ref() else {
            return ConfirmKey::Gone;
        };
        if now_ms - arm.armed_at < Y_GUARD_MS {
            return ConfirmKey::TooSoon;
        }
        self.take_confirmed(now_ms)
            .map_or(ConfirmKey::Gone, |(id, ctx)| {
                ConfirmKey::Run(Box::new((id, ctx)))
            })
    }

    /// A click while an arm is pending. `region` is the slot region the
    /// click landed on (`None` for a core target or empty space). Only a
    /// click on the region that armed it confirms, and only once the guard
    /// window has passed and the prompt has been drawn; anything else
    /// cancels.
    pub(in crate::runtime) fn plugin_confirm_click(
        &mut self,
        region: Option<&RegionId>,
        now_ms: i64,
    ) -> ClickConfirm {
        self.refresh_plugin_arm(now_ms);
        let Some(arm) = self.plugin_arm.as_ref() else {
            return ClickConfirm::Cancelled;
        };
        let hit = match (&arm.origin, region) {
            (ArmOrigin::Region(armed), Some(clicked)) => armed == clicked,
            _ => false,
        };
        if !hit {
            self.disarm_plugin();
            return ClickConfirm::Cancelled;
        }
        if now_ms - arm.armed_at < Y_GUARD_MS || !arm.drawn {
            return ClickConfirm::Swallow;
        }
        match self.take_confirmed(now_ms) {
            Some(action) => ClickConfirm::Run(Box::new(action)),
            None => ClickConfirm::Cancelled,
        }
    }

    /// Consume the live arm into a confirmed context and issue the token.
    fn take_confirmed(&mut self, now_ms: i64) -> Option<(ActionId, ActionCtx)> {
        let key = self.plugin_arm.as_ref()?.key.clone();
        if !self.armed_plugin.confirm(&key, now_ms) {
            self.disarm_plugin();
            return None;
        }
        let arm = self.plugin_arm.take()?;
        self.notices.clear_prefix(&arm.notice);
        self.confirm_token = Some((arm.key, arm.force));
        let mut ctx = arm.ctx;
        ctx.source = ActionSource::Plugin {
            plugin: plugin_name(&ctx.source),
            confirmed: true,
        };
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
        self.refresh_plugin_arm(now_ms);
        let info = meta(id);
        // A plugin action whose manifest says `confirm` goes through the same
        // arm-then-confirm step as a dangerous core action.
        let asks_confirm = self.plugin_actions.get(&id).is_some_and(|a| a.confirm);
        let dangerous = info.is_some_and(|m| m.dangerous) || asks_confirm;
        // `confirmed` counts only with the token the confirm path issued for
        // this very action; a forged one is just another plugin trigger.
        let mut confirmed_force = None;
        if let ActionSource::Plugin {
            confirmed: true, ..
        } = ctx.source
        {
            let token = self.confirm_token.take();
            confirmed_force = token
                .filter(|(key, _)| *key == arm_key(id, ctx))
                .map(|(_, force)| force);
        }
        if let Some(force) = confirmed_force {
            self.run_confirmed(store, id, ctx, force, now_ms, effects);
            return true;
        }
        let forged = matches!(
            ctx.source,
            ActionSource::Plugin {
                confirmed: true,
                ..
            }
        );
        if ctx.source.is_plugin_origin() || forged {
            if info.is_some_and(|m| !m.plugin_invocable) {
                self.notices
                    .show("that action is not available to plugins", 3000, now_ms);
                return true;
            }
            // A row that is not a session's own (an extra row without
            // `about_session`) has no session: session actions are refused,
            // never pointed at the focused one.
            if session_scoped(id) && ctx.session.is_none() {
                self.notices.show("nothing to act on", 2000, now_ms);
                return true;
            }
            if dangerous {
                // A plugin source never replaces or refreshes a pending arm
                // (its own or a human's): refused, silently, so the prompt
                // the human is looking at stays put.
                if self.plugin_arm.is_none() {
                    let origin = match (&ctx.source, &ctx.region) {
                        (ActionSource::Widget { .. }, Some(region)) => {
                            ArmOrigin::Region(region.clone())
                        }
                        _ => ArmOrigin::Api,
                    };
                    self.arm_plugin(store, id, ctx, origin, now_ms);
                }
                return true;
            }
            if matches!(ctx.source, ActionSource::Plugin { .. })
                && store.state().layer == crate::state::wall_state::KeyLayer::Engaged
                && moves_focus_away(store, id, ctx)
            {
                // Typing never moves to another session behind the human's
                // back: only a human click or key changes where they type.
                self.notices.show(
                    format!(
                        "plugin {} wanted to switch sessions — press Ctrl+G to leave the session first",
                        plugin_name(&ctx.source)
                    ),
                    4000,
                    now_ms,
                );
                return true;
            }
            if matches!(ctx.source, ActionSource::Plugin { .. }) {
                // Focus moves are for a person to make at a person's pace: a
                // burst from a plugin lands once and the rest is dropped.
                if moves_focus(id) {
                    if self
                        .plugin_focus_at
                        .is_some_and(|t| now_ms - t < PLUGIN_FOCUS_GAP_MS)
                    {
                        return true;
                    }
                    self.plugin_focus_at = Some(now_ms);
                }
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
        if !dangerous {
            return false;
        }
        // A human trigger on a dangerous id confirms only an arm a human
        // raised (`session.delete`'s own double trigger); a plugin arm is
        // confirmed by `y` or its own region's click, never by a core
        // trigger, however similar.
        let human_arm = self
            .plugin_arm
            .as_ref()
            .is_some_and(|a| a.origin == ArmOrigin::Human && a.key == arm_key(id, ctx));
        if human_arm
            && self.armed_plugin.confirm(&arm_key(id, ctx), now_ms)
            && let Some(arm) = self.plugin_arm.take()
        {
            self.notices.clear_prefix(&arm.notice);
            if store.state().layer == crate::state::wall_state::KeyLayer::Engaged {
                store.disengage();
            }
            self.run_confirmed(store, id, &arm.ctx, arm.force, now_ms, effects);
            return true;
        }
        // ... and `session.delete`, which has no native double-press of its
        // own, arms on a human's first trigger instead of running at once.
        if id == ids::SESSION_DELETE || asks_confirm {
            self.arm_plugin(store, id, ctx, ArmOrigin::Human, now_ms);
            return true;
        }
        false
    }

    fn arm_plugin(
        &mut self,
        store: &WallStore,
        id: ActionId,
        ctx: &ActionCtx,
        origin: ArmOrigin,
        now_ms: i64,
    ) {
        let prompt = match self.plugin_actions.get(&id) {
            Some(action) => Some((format!("Run {}{CONFIRM_TAIL}", action.title), false)),
            None => arm_prompt(store, id, ctx),
        };
        let Some((notice, force)) = prompt else {
            self.notices.show("nothing to act on", 2000, now_ms);
            return;
        };
        // Only a human trigger can get here with an arm pending: it replaces
        // the older arm and clears its line first.
        if let Some(old) = self.plugin_arm.take() {
            self.notices.clear_prefix(&old.notice);
        }
        let key = arm_key(id, ctx);
        self.armed_plugin.arm(&key, now_ms);
        self.notices.show(notice.clone(), 3000, now_ms);
        self.plugin_arm = Some(PluginArm {
            id,
            ctx: ctx.clone(),
            key,
            notice,
            origin,
            force,
            armed_at: now_ms,
            drawn: false,
        });
    }

    /// Run a dangerous action a human just confirmed. Targets come from the
    /// armed context, never from the focus and never from `args`.
    fn run_confirmed(
        &mut self,
        store: &mut WallStore,
        id: ActionId,
        ctx: &ActionCtx,
        force: bool,
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
                let label = id.rsplit('/').next().unwrap_or(id).to_owned();
                // A live session is killed with no closed record (the arm
                // prompt said so); a closed one just loses its record.
                if store
                    .state()
                    .session_by_id(Some(id))
                    .is_some_and(|s| s.live())
                {
                    effects.push(Effect::Forget {
                        id: id.to_owned(),
                        label,
                    });
                    return;
                }
                effects.push(Effect::Close {
                    id: id.to_owned(),
                    label,
                    meta_only: true,
                });
            }
            ids::SESSION_RESTART => {
                let Some(s) = store.state().session_by_id(session).filter(|s| s.live()) else {
                    self.notices
                        .show("no live session to restart", 2000, now_ms);
                    return;
                };
                // Same words as the native `r r` once it fires; `force` is what
                // the arm prompt warned about, not a fresh look at the state.
                let name = s.display_name().to_owned();
                effects.push(Effect::RestartSession {
                    id: s.id.clone(),
                    name: name.clone(),
                    force,
                });
                self.notices.show(
                    format!("restarting {name} — resumes the conversation"),
                    5000,
                    now_ms,
                );
            }
            ids::WORKSPACE_RESTART => {
                let Some(ws) = ctx.workspace.as_deref() else {
                    return;
                };
                let ids: Vec<String> = store
                    .state()
                    .sessions
                    .iter()
                    .filter(|s| s.workspace == ws && (s.status == "idle" || s.status == "done"))
                    .map(|s| s.id.clone())
                    .collect();
                if ids.is_empty() {
                    self.notices.show(
                        format!("nothing to restart in {ws} (all busy or none)"),
                        3000,
                        now_ms,
                    );
                    return;
                }
                self.notices.show(
                    format!("restarting {} sessions in {ws}", ids.len()),
                    5000,
                    now_ms,
                );
                effects.push(Effect::RestartWorkspace { ids });
            }
            ids::DAEMON_RESTART => {
                self.notices.show("restarting the daemon…", 5000, now_ms);
                effects.push(Effect::RestartDaemon);
            }
            ids::WORKSPACE_REMOVE | ids::WORKSPACE_REMOVE_KILL => {
                let Some(name) = ctx.workspace.clone() else {
                    return;
                };
                if store
                    .state()
                    .group_by_name(Some(&name))
                    .is_some_and(|g| g.registered)
                {
                    effects.push(Effect::RemoveWorkspace {
                        name,
                        kill: id == ids::WORKSPACE_REMOVE_KILL,
                    });
                }
            }
            ids::SESSION_RESTORE_ALL => {
                let Some(ws) = ctx.workspace.clone() else {
                    return;
                };
                let ids: Vec<String> = restorable_session_ids(&store.state().sessions, &ws)
                    .into_iter()
                    .filter(|id| !self.restoring().contains(id))
                    .collect();
                if ids.is_empty() {
                    self.notices
                        .show(format!("nothing to restore in {ws}"), 2000, now_ms);
                }
                for id in ids {
                    self.restore(id, effects);
                }
            }
            ids::HOOKS_INSTALL => self.install_pressed(effects),
            _ if id.is_plugin() => self.run_plugin_action(id, ctx, now_ms, effects),
            _ => {}
        }
    }
}

/// `workspace/label`, the unambiguous name a prompt gives a session. A
/// session the store no longer lists is named from its id.
fn target_name(state: &crate::state::wall_state::WallState, sid: &str) -> String {
    match state.session_by_id(Some(sid)) {
        Some(s) => format!("{}/{}", s.workspace, s.label),
        None => {
            let mut parts = sid.rsplit('/');
            let label = parts.next().unwrap_or(sid);
            match parts.next() {
                Some(ws) => format!("{ws}/{label}"),
                None => label.to_owned(),
            }
        }
    }
}

/// Session-scoped actions: with no session in the context they have no
/// target, and a plugin trigger must not fall back to the focused one.
fn session_scoped(id: ActionId) -> bool {
    matches!(
        id,
        ids::SESSION_CLOSE
            | ids::SESSION_DELETE
            | ids::SESSION_RESTART
            | ids::SESSION_MAXIMIZE
            | ids::SESSION_DETACH
            | ids::SESSION_OPEN_WINDOW
            | ids::SESSION_ENGAGE
            | ids::SESSION_RESTORE
            | ids::OVERLAY_OPEN_VIEW_PICKER
    )
}

/// The arm prompt ("Close ws/foo? click again or press y") and, for a
/// restart, whether the session was busy when it was raised (the force).
/// `None` when the target the action needs is missing.
fn arm_prompt(store: &WallStore, id: ActionId, ctx: &ActionCtx) -> Option<(String, bool)> {
    let state = store.state();
    let session = || ctx.session.as_deref();
    let known = || state.session_by_id(session()).is_some();
    let name = || session().map(|sid| target_name(state, sid));
    let subject = match id {
        ids::SESSION_CLOSE if known() => format!("Close {}", name()?),
        ids::SESSION_RESTART if known() => {
            let s = state.session_by_id(session())?;
            if s.status == "working" || s.needs_input() {
                // The native `r r` warning, with this prompt's confirm keys.
                let text = format!(
                    "{} is working — click again or press y to restart anyway",
                    name()?
                );
                return Some((text, true));
            }
            format!("Restart {}", name()?)
        }
        ids::SESSION_DELETE => {
            let live = state.session_by_id(session()).is_some_and(|s| s.live());
            if live {
                // Not the usual tail: this one has no way back.
                let text = format!(
                    "Delete {}? (kills it, no history) — click again or press y",
                    name()?
                );
                return Some((text, false));
            }
            format!("Delete {}", name()?)
        }
        ids::WORKSPACE_RESTART => format!("Restart idle sessions in {}", ctx.workspace.as_deref()?),
        ids::WORKSPACE_REMOVE => format!("Remove workspace {}", ctx.workspace.as_deref()?),
        ids::WORKSPACE_REMOVE_KILL => {
            format!(
                "Remove workspace {} and kill its sessions",
                ctx.workspace.as_deref()?
            )
        }
        ids::SESSION_RESTORE_ALL => {
            format!("Restore every session in {}", ctx.workspace.as_deref()?)
        }
        ids::DAEMON_RESTART => "Restart the daemon".to_owned(),
        ids::HOOKS_INSTALL => "Install the statusline and hooks".to_owned(),
        _ => return None,
    };
    Some((format!("{subject}{CONFIRM_TAIL}"), false))
}

/// Least time between two focus moves a plugin may make.
pub(in crate::runtime) const PLUGIN_FOCUS_GAP_MS: i64 = 500;

/// The ids whose whole point is to move the focus (or the typing target).
fn moves_focus(id: ActionId) -> bool {
    matches!(
        id,
        ids::SESSION_FOCUS
            | ids::WORKSPACE_FOCUS
            | ids::VIEW_FOCUS
            | ids::VIEW_CYCLE
            | ids::JUMP_BLOCKED
            | ids::FOCUS_CYCLE_PREV
            | ids::FOCUS_CYCLE_NEXT
            | ids::SESSION_RESTORE
            | ids::SESSION_MAXIMIZE
            | ids::OVERLAY_OPEN_VIEW_PICKER
    )
}

/// True when running `id` for `ctx` would move the focus (and so the typing
/// target) off what is focused now.
fn moves_focus_away(store: &WallStore, id: ActionId, ctx: &ActionCtx) -> bool {
    let state = store.state();
    match id {
        ids::SESSION_FOCUS
        | ids::SESSION_ENGAGE
        | ids::SESSION_RESTORE
        | ids::SESSION_MAXIMIZE
        | ids::SESSION_DETACH
        | ids::SESSION_OPEN_WINDOW
        | ids::OVERLAY_OPEN_VIEW_PICKER => ctx
            .session
            .as_deref()
            .is_some_and(|s| state.focused_session_id.as_deref() != Some(s)),
        ids::WORKSPACE_FOCUS | ids::SESSION_SPAWN | ids::SESSION_SPAWN_WORKTREE => ctx
            .workspace
            .as_deref()
            .is_some_and(|w| state.focused_workspace.as_deref() != Some(w)),
        ids::VIEW_FOCUS => true,
        _ => false,
    }
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
            if let Some(session) = ctx.session.as_deref()
                && store.state().focused_session_id.as_deref() != Some(session)
                && store.state().session_by_id(Some(session)).is_some()
            {
                store.focus_session(session);
            }
        }
        ids::SESSION_SPAWN | ids::SESSION_SPAWN_WORKTREE | ids::WORKSPACE_FOCUS => {
            if let Some(ws) = ctx.workspace.as_deref()
                && store.state().focused_workspace.as_deref() != Some(ws)
            {
                store.focus_workspace_named(ws);
            }
        }
        _ => {}
    }
}
