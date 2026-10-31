//! The garage-layer router: key actions, arming state, busy guards and the
//! strip notices (moved out of `runtime/mod.rs`, p22 task 1.3).

use std::collections::HashSet;

use crate::actions::{ActionCtx, ActionId, ActionSource, command_for_action, ids};
use crate::keymap::{Keymap, Layer};
use crate::state::armed_action::{ArmedAction, ArmedClose};
use crate::state::salience::restorable_session_ids;
use crate::state::store::{ArmedRestart, GarageCommand, WallStore, restart_arm_notice};
use crate::state::views;
use crate::state::wall_state::{KeyLayer, WallState};
use crate::state::workspace_remove::{confirm_kill_target, remove_arm_notice};
use crate::ui::pet::PetHost;

use super::Effect;

pub(in crate::runtime) mod plugin_gate;

/// The garage-layer typing hint (post-review Dart wording, verbatim — the
/// click-smoke harness greps it).
pub const TYPING_HINT: &str = "enter engages the focused terminal — keys go to garage now";

/// What the one-shot `r` prefix is waiting for (p16-restart). Without it the
/// prefix would swallow the next keystroke in silence — the same reason
/// unbound garage keys raise [`TYPING_HINT`] instead of doing nothing.
pub const RESTART_PREFIX_HINT: &str = "r: r session · a workspace · d daemon";

/// Who put the current strip notice up (spec tui-pit-pet "Species-voiced
/// chatter": "SHALL NOT replace a non-pet notice that is still showing").
/// `User` covers everything that existed before p15 — armed-close/-remove
/// wording, the typing hint, lifecycle results, the statusline hint — all
/// of which must always win over a pet's chatter line.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum NoticeKind {
    #[default]
    User,
    Pet,
}

/// Transient strip notice with a TTL, plus the Dart TUI's two behaviors the
/// armed flows depend on: prefix-clearing (disarming clears its own arm
/// notice, never an unrelated one) and the rate-limited typing hint (a burst
/// of typing does not restart the hint's timer). Time is injected (ms) so it
/// unit-tests without timers.
///
/// p15 adds a second, lower-priority writer (pit-pet chatter, spec
/// tui-pit-pet): [`Notices::show_pet`] is the only entry point that checks
/// before overwriting — every pre-existing call goes through [`Notices::show`]
/// or [`Notices::show_typing_hint`], which are `NoticeKind::User` and always
/// win, exactly as they did before chatter existed.
#[derive(Default)]
pub struct Notices {
    text: Option<String>,
    expires_at_ms: i64,
    kind: NoticeKind,
}

impl Notices {
    /// Always wins, regardless of what (if anything) is currently showing —
    /// every call site that predates p15 uses this, so none of that
    /// behavior changes: a user notice can never be blocked.
    pub fn show(&mut self, text: impl Into<String>, ttl_ms: i64, now_ms: i64) {
        self.text = Some(text.into());
        self.expires_at_ms = now_ms + ttl_ms;
        self.kind = NoticeKind::User;
    }

    /// Garage-layer typing hint, rate-limited: while the hint is already up,
    /// further keypresses in the burst do NOT restart the timer.
    pub fn show_typing_hint(&mut self, now_ms: i64) {
        if self.text.as_deref() == Some(TYPING_HINT) && now_ms < self.expires_at_ms {
            return;
        }
        self.show(TYPING_HINT, 2500, now_ms);
    }

    /// Pit-pet chatter (spec tui-pit-pet "Species-voiced chatter"): refuses
    /// — leaving whatever is showing untouched — while a live `User` notice
    /// is up; a pet line freely replaces another pet line (or nothing).
    /// Returns whether it was shown, so the chatter scheduler knows the line
    /// wasn't dropped on the floor (it can retry, rather than treat the tick
    /// as having "said" a line it didn't).
    pub fn show_pet(&mut self, text: impl Into<String>, ttl_ms: i64, now_ms: i64) -> bool {
        if self.user_notice_active(now_ms) {
            return false;
        }
        self.text = Some(text.into());
        self.expires_at_ms = now_ms + ttl_ms;
        self.kind = NoticeKind::Pet;
        true
    }

    /// Drop a showing `Pet`-kind notice (spec tui-pit-pet "Chatter yields
    /// to attention": a chatter line must not linger once a session needs
    /// input). User notices are untouched. Returns true when one was cleared.
    pub fn clear_pet(&mut self) -> bool {
        if self.text.is_some() && self.kind == NoticeKind::Pet {
            self.text = None;
            return true;
        }
        false
    }

    /// True while a live (unexpired) `User`-kind notice is showing — the
    /// gate [`Notices::show_pet`] checks, exposed for the chatter scheduler
    /// to consult before even building a line.
    pub fn user_notice_active(&self, now_ms: i64) -> bool {
        self.kind == NoticeKind::User && self.text.is_some() && now_ms < self.expires_at_ms
    }

    /// Clear the current notice iff it starts with `prefix` (the Dart
    /// `_clearArmNotice`): a disarm must never wipe an unrelated notice.
    pub fn clear_prefix(&mut self, prefix: &str) {
        if self.text.as_deref().is_some_and(|t| t.starts_with(prefix)) {
            self.text = None;
        }
    }

    pub fn current(&self, now_ms: i64) -> Option<&str> {
        if now_ms >= self.expires_at_ms {
            return None;
        }
        self.text.as_deref()
    }

    /// The showing notice split by kind: `User` lines belong in the strip's
    /// right-aligned slot, `Pet` lines are drawn beside the pet's sprite.
    pub fn current_of(&self, kind: NoticeKind, now_ms: i64) -> Option<&str> {
        if self.kind != kind {
            return None;
        }
        self.current(now_ms)
    }

    /// Drop an expired notice; returns true when the visible state changed
    /// (the caller marks the frame dirty).
    pub fn tick(&mut self, now_ms: i64) -> bool {
        if self.text.is_some() && now_ms >= self.expires_at_ms {
            self.text = None;
            return true;
        }
        false
    }
}

/// First free `claude-N` label in the workspace (the default label family —
/// port of the Dart `_nextLabel`).
pub(super) fn next_label(state: &WallState, workspace: &str) -> String {
    let taken: HashSet<u32> = state
        .sessions
        .iter()
        .filter(|s| s.workspace == workspace)
        .filter_map(|s| {
            let digits = s.label.strip_prefix("claude-")?;
            if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
                return None;
            }
            digits.parse().ok()
        })
        .collect();
    let mut n = 1;
    while taken.contains(&n) {
        n += 1;
    }
    format!("claude-{n}")
}

/// A printable character (something the user plausibly meant as typing) —
/// controls and escape-derived characters don't trigger the typing hint.
pub(super) fn is_printable(key: &str) -> bool {
    key.chars()
        .next()
        .is_some_and(|c| c as u32 >= 0x20 && c != '\u{7f}')
}

/// What a garage key resolved to: IO the arming logic already produced
/// (the `X` `K` confirm), and the action the caller dispatches through its
/// one dispatcher (`App::dispatch_action`), same as a click does.
#[derive(Default)]
pub struct KeyOutcome {
    pub effects: Vec<Effect>,
    pub action: Option<(ActionId, ActionCtx)>,
}

impl KeyOutcome {
    fn effects(effects: Vec<Effect>) -> Self {
        KeyOutcome {
            effects,
            action: None,
        }
    }
}

/// Garage-layer key routing (spec tui-key-routing: "Garage-layer bindings" +
/// p8.1–p8.4, plus spec restart "TUI chords"). Pure — state transitions go
/// through the store, IO becomes returned [`Effect`]s, notices land in
/// [`Notices`] — so the armed `x`/`X`/`K` flows, the p16-restart `r` chords
/// and their exact strip wording unit-test without a daemon. Mirrors the
/// Dart `_onGarageKey` ordering, with the one-shot `r` prefix resolved ahead
/// of it (p16-restart, design.md D5).
#[derive(Default)]
pub struct GarageRouter {
    /// The `x` double-press close arming (3s window, keyed by session id).
    armed_close: ArmedClose,
    /// The `X` double-press workspace-remove arming (independent instance,
    /// keyed by workspace name — spec p8.3).
    armed_remove: ArmedAction,
    /// Sessions with a restore call in flight — guards duplicate restores
    /// (and wave 4 renders their "restoring…" placeholders from this).
    restoring: HashSet<String>,
    /// A spawn call in flight — guards concurrent spawns.
    spawning: bool,
    /// The `I` install (statusline + hooks) in flight — guards concurrent
    /// installs.
    installing: bool,
    /// The `r r` double-press restart arming (p16-restart) — an instance
    /// independent of `armed_close`, keyed by session id.
    armed_restart: ArmedRestart,
    /// The exact `r r` arm notice currently showing, so disarming clears
    /// its own line and nothing else: the two arm wordings differ (plain vs
    /// busy), so there is no shared prefix for `Notices::clear_prefix`.
    armed_restart_notice: Option<String>,
    /// True between the `r` prefix press and the very next key (p16-restart,
    /// design.md D5: a one-shot prefix, like `X` → `X`/`K`).
    restart_prefix: bool,
    /// The arm a plugin trigger left on a dangerous action (p24 confirm
    /// rule): independent of the native machines, armed only by
    /// [`ArmedAction::arm`], confirmed only by a human key or click.
    armed_plugin: ArmedAction,
    plugin_arm: Option<plugin_gate::PluginArm>,
    /// One-shot proof that a `confirmed` plugin source came from this
    /// router's own confirm path: (arm key, restart force).
    confirm_token: Option<(String, bool)>,
    pub notices: Notices,
}

impl GarageRouter {
    pub fn restoring(&self) -> &HashSet<String> {
        &self.restoring
    }

    pub fn spawn_settled(&mut self) {
        self.spawning = false;
    }

    pub fn restore_settled(&mut self, id: &str) {
        self.restoring.remove(id);
    }

    pub fn install_settled(&mut self) {
        self.installing = false;
    }

    /// Handle one garage-layer key (already reduced to its string form by
    /// [`garage_key_string`]) against the app's `keymap`. The garage layer consumes everything; the
    /// returned effects are the caller's IO.
    pub fn on_garage_key(
        &mut self,
        store: &mut WallStore,
        keymap: &Keymap,
        key: &str,
        now_ms: i64,
    ) -> KeyOutcome {
        let mut effects = Vec::new();
        // p16-restart (design.md D5): the `r` prefix is one-shot — the very
        // next key is the chord's second half, resolved BEFORE anything else
        // so `r a` / `r d` never reach the plain `a` / `d` bindings.
        if std::mem::take(&mut self.restart_prefix) {
            let action = self.restart_chord(keymap, key).map(|id| {
                let ctx = ActionCtx::from_focus(store.state(), ActionSource::Chord(key.to_owned()));
                (id, ctx)
            });
            return KeyOutcome { effects, action };
        }
        // A pending plugin arm (p24): `y` confirms it, any other key drops it.
        self.refresh_plugin_arm(now_ms);
        if self.plugin_arm_pending() {
            if key == "y" {
                match self.plugin_confirm_key(now_ms) {
                    plugin_gate::ConfirmKey::Run(action) => {
                        return KeyOutcome { effects, action: Some(*action) };
                    }
                    // A racing `y` is swallowed; the prompt stays.
                    plugin_gate::ConfirmKey::TooSoon => return KeyOutcome::effects(effects),
                    plugin_gate::ConfirmKey::Gone => {}
                }
            }
            self.disarm_plugin();
        }
        // Resolve the key to its action once; arming below is keyed on the
        // action (so `x`'s double-press is "session.close twice"), which is
        // byte-identical for the default keymap.
        let mut resolved = keymap.resolve(Layer::Garage, key);
        let mut action = resolved.map(|r| r.action);
        // p8.4: while an `X` remove arm is active, `K` confirms the removal
        // WITH session kill — checked BEFORE the disarm pass (K would
        // otherwise disarm the very arm it confirms). Outside a live arm `K`
        // falls through as an ordinary unbound key (typing hint).
        if action == Some(ids::WORKSPACE_REMOVE_KILL) {
            if self.kill_remove_pressed(store, now_ms, &mut effects) {
                return KeyOutcome::effects(effects);
            }
            resolved = None;
            action = None;
        }
        // Any key but `x` disarms a pending close (spec p8.1: "any other key
        // disarms"); same for `X` and a pending workspace removal (p8.3) —
        // the two arms are independent, so an `X` disarms a pending `x` and
        // vice versa. `K` only survives the remove arm via the branch above.
        if action != Some(ids::SESSION_CLOSE) {
            self.disarm_close();
        }
        if action != Some(ids::WORKSPACE_REMOVE) {
            self.disarm_remove();
        }
        // p16-restart: the `r r` arm survives only the prefix key itself —
        // the second `r` arrives through the chord branch above, so every
        // key that reaches here (including a bare `r` opening a fresh
        // prefix) leaves the arm alone or disarms it, same rule as `x`.
        if action != Some(ids::CHORD_RESTART) {
            self.disarm_restart();
        }
        // p16-restart: `r` alone is a prefix, not a command — consume it and
        // wait for the chord. The hint would bury a live `r r` arm notice
        // (which already says what the next press does), so it yields to it.
        if action == Some(ids::CHORD_RESTART) {
            self.restart_prefix = true;
            if self.armed_restart.armed_id().is_none() {
                self.notices.show(RESTART_PREFIX_HINT, 3000, now_ms);
            }
            return KeyOutcome::effects(effects);
        }
        let Some(resolved) = resolved else {
            // Unbound garage keys are consumed (never reach an agent), but
            // silence reads as a dead wall — show where the keys actually go.
            if is_printable(key) {
                self.notices.show_typing_hint(now_ms);
            }
            return KeyOutcome::effects(effects);
        };
        let ctx = ActionCtx::from_focus(store.state(), ActionSource::Key(key.to_owned()))
            .with_index(resolved.index);
        KeyOutcome {
            effects,
            action: Some((resolved.action, ctx)),
        }
    }

    /// Run one action against the store, whether a key or a click raised it
    /// (the caller's dispatcher, `App::dispatch_action`, adds the state the
    /// router does not own). The context is honoured: a click lands first
    /// (an engaged tile disengages), then actions that target a session
    /// focus `ctx.session` before they act. State transitions go through
    /// [`WallStore::dispatch`]; commands the store declines become
    /// [`Effect`]s or notices here.
    pub fn run_action(
        &mut self,
        store: &mut WallStore,
        id: ActionId,
        ctx: &ActionCtx,
        now_ms: i64,
        effects: &mut Vec<Effect>,
    ) {
        let from_click = matches!(ctx.source, ActionSource::Mouse { .. });
        if from_click && store.state().layer == KeyLayer::Engaged {
            store.disengage();
        }
        if matches!(id, ids::SESSION_FOCUS | ids::SESSION_ENGAGE | ids::SESSION_RESTORE) {
            if let Some(session) = &ctx.session {
                if from_click || store.state().focused_session_id.as_deref() != Some(session) {
                    store.focus_session(session);
                }
            }
        }
        match id {
            // `P` (spec tui-pit-pet "Opt-in roster cycled by `P`"): not a
            // `GarageCommand` — `pet.rs` (the render module) has no business
            // being a store dependency, so this stays here rather than
            // growing `state::store`'s command enum for one field's cycling.
            ids::PET_CYCLE_SPECIES => self.cycle_pet_pressed(store, now_ms),
            ids::SESSION_RESTORE => {
                if let Some(session) = ctx.session.clone() {
                    self.restore(session, effects);
                }
            }
            ids::SESSION_FOCUS => {} // the landing above did it
            ids::VIEW_FOCUS => {
                if let Some(view) = &ctx.view {
                    store.focus_view(view);
                }
            }
            // The restart ids run only from the `r` chord: `a` and `d` are
            // also plain garage keys, and a restart must never fire from a
            // click or a rebound key by accident.
            ids::SESSION_RESTART | ids::WORKSPACE_RESTART | ids::DAEMON_RESTART
                if !matches!(ctx.source, ActionSource::Chord(_)) => {}
            ids::SESSION_RESTART => self.restart_focused_pressed(store, now_ms, effects),
            ids::WORKSPACE_RESTART => self.restart_workspace_pressed(store, now_ms, effects),
            ids::DAEMON_RESTART => {
                self.notices.show("restarting the daemon…", 5000, now_ms);
                effects.push(Effect::RestartDaemon);
            }
            _ => self.run_command(store, id, ctx, now_ms, effects),
        }
    }

    /// The ids that map onto a [`GarageCommand`] (the store's transitions
    /// plus the effects it declines).
    fn run_command(
        &mut self,
        store: &mut WallStore,
        id: ActionId,
        ctx: &ActionCtx,
        now_ms: i64,
        effects: &mut Vec<Effect>,
    ) {
        let Some(command) = command_for_action(id, ctx.index) else {
            return;
        };
        // `d` needs to know afterward whether it detached (solo view) or
        // rejoined (default view) to show the right notice — special-cased
        // ahead of the generic dispatch below.
        if let GarageCommand::DetachFocused = command {
            self.detach_pressed(store, now_ms);
            return;
        }
        if store.dispatch(command) {
            return;
        }
        // Commands the store declined: effects (spawn/restore/close/quit),
        // the empty-jump notice, and Enter on a restorable tile (restore).
        match command {
            GarageCommand::Spawn { worktree } => self.spawn(store, worktree, effects),
            GarageCommand::Quit => effects.push(Effect::Quit),
            GarageCommand::Jump => {
                // Spec tui-triage: no-op with a brief strip notice.
                self.notices.show("no session needs you", 2000, now_ms);
            }
            GarageCommand::Engage => {
                // Engage declined: the focused tile is a restorable
                // placeholder → Enter restores it (spec p8.1).
                let restorable = store
                    .state()
                    .session_by_id(store.state().focused_session_id.as_deref())
                    .filter(|s| !s.live())
                    .map(|s| s.id.clone());
                if let Some(id) = restorable {
                    self.restore(id, effects);
                }
            }
            GarageCommand::RestoreAll => self.restore_all(store, now_ms, effects),
            GarageCommand::Close => self.close_pressed(store, now_ms, effects),
            GarageCommand::WorkspaceRemove => self.remove_pressed(store, now_ms, effects),
            GarageCommand::InstallStatusline => self.install_pressed(effects),
            GarageCommand::OpenWindow => self.open_window_pressed(store, now_ms, effects),
            _ => {}
        }
    }

    /// `n`/`N`: spawn into the focused workspace with a generated `claude-N`
    /// label (spec: "Spawn from the rail"); busy-guarded.
    fn spawn(&mut self, store: &WallStore, worktree: bool, effects: &mut Vec<Effect>) {
        if self.spawning {
            return;
        }
        let Some(workspace) = store.state().focused_workspace.clone() else {
            return;
        };
        self.spawning = true;
        let label = next_label(store.state(), &workspace);
        effects.push(Effect::Spawn {
            workspace,
            label,
            worktree,
        });
    }

    /// `I`: fire the install effect (statusline + hooks), busy-guarded
    /// against a second press before the first settles (spec
    /// tui-context-meters "Install affordance", spec tui-hooks-install).
    pub(super) fn install_pressed(&mut self, effects: &mut Vec<Effect>) {
        if self.installing {
            return;
        }
        self.installing = true;
        effects.push(Effect::Install);
    }

    /// `t`: open the focused session in its own OS terminal window (p12
    /// standalone-window — the `m` maximize's OS-window companion). Declined
    /// for a restorable/dead focus ("no live session to open") and off
    /// macOS entirely ("standalone windows: macOS only for now") — in
    /// neither case does an [`Effect::OpenWindow`] get built, so
    /// [`crate::ui::window_open`] never has to make that call itself.
    fn open_window_pressed(&mut self, store: &WallStore, now_ms: i64, effects: &mut Vec<Effect>) {
        let Some(session) = store
            .state()
            .session_by_id(store.state().focused_session_id.as_deref())
        else {
            return;
        };
        if !session.live() {
            self.notices.show("no live session to open", 2000, now_ms);
            return;
        }
        if !cfg!(target_os = "macos") {
            self.notices
                .show("standalone windows: macOS only for now", 3000, now_ms);
            return;
        }
        effects.push(Effect::OpenWindow {
            id: session.id.clone(),
            label: session.label.clone(),
        });
    }

    /// `P`: cycle the pit-pet roster off → Arthur → Papito → Segan → off
    /// (spec tui-pit-pet), persist the choice, and show the arrival notice
    /// (or "the strip is quiet again" for off). Resetting `PetSim`/ticking
    /// the sprite promptly is the caller's job (`App::handle_key` — it owns
    /// the sim, the router doesn't).
    fn cycle_pet_pressed(&mut self, store: &mut WallStore, now_ms: i64) {
        let Some((next, text)) = PetHost::cycle_choice(store.state().pet.as_deref()) else {
            self.notices.show("the pit pet is not in this build", 3000, now_ms);
            return;
        };
        store.set_pet(next);
        self.notices.show(text, 3000, now_ms);
    }

    /// Restore one session, deduplicated against in-flight restores.
    pub(super) fn restore(&mut self, id: String, effects: &mut Vec<Effect>) {
        if self.restoring.contains(&id) {
            return;
        }
        self.restoring.insert(id.clone());
        effects.push(Effect::Restore { id });
    }

    /// `R`: restore ALL restorable sessions in the focused workspace —
    /// parallel per-id calls, so one failure never blocks the rest.
    fn restore_all(&mut self, store: &WallStore, now_ms: i64, effects: &mut Vec<Effect>) {
        let Some(workspace) = store.state().focused_workspace.clone() else {
            return;
        };
        let ids: Vec<String> = restorable_session_ids(&store.state().sessions, &workspace)
            .into_iter()
            .filter(|id| !self.restoring.contains(id))
            .collect();
        if ids.is_empty() {
            self.notices
                .show(format!("nothing to restore in {workspace}"), 2000, now_ms);
            return;
        }
        for id in ids {
            self.restore(id, effects);
        }
    }

    /// `x`: armed double-press. First press arms (strip notice, 3s window);
    /// the second press on the same session within the window closes it —
    /// a live session via plain DELETE, a restorable one via `?meta=1`.
    fn close_pressed(&mut self, store: &WallStore, now_ms: i64, effects: &mut Vec<Effect>) {
        let Some(session) = store
            .state()
            .session_by_id(store.state().focused_session_id.as_deref())
        else {
            return;
        };
        let (id, label, live) = (session.id.clone(), session.label.clone(), session.live());
        if self.armed_close.press(&id, now_ms) {
            self.notices.clear_prefix("press x again to close ");
            effects.push(Effect::Close {
                id,
                label,
                meta_only: !live,
            });
        } else {
            self.notices
                .show(format!("press x again to close {label}"), 3000, now_ms);
        }
    }

    /// `d` (spec tui-views "Detach and rejoin"): dispatch the detach/rejoin
    /// transition, then show "detached &lt;label&gt;" iff it actually
    /// detached (landed on a non-default view) — a rejoin gets no notice,
    /// matching the design.md contract's harness-grepped wording.
    fn detach_pressed(&mut self, store: &mut WallStore, now_ms: i64) {
        let session = store
            .state()
            .session_by_id(store.state().focused_session_id.as_deref())
            .cloned();
        if !store.dispatch(GarageCommand::DetachFocused) {
            return;
        }
        let Some(session) = session else { return };
        if store.state().focused_view_name(&session.workspace) != views::DEFAULT_VIEW {
            self.notices.show(format!("detached {}", session.label), 2000, now_ms);
        }
    }

    fn disarm_close(&mut self) {
        if self.armed_close.armed_id().is_none() {
            return;
        }
        self.armed_close.disarm();
        self.notices.clear_prefix("press x again to close ");
    }

    /// `X`: armed double-press removal of the FOCUSED workspace (spec p8.3).
    /// Registered groups arm/confirm; a synthesized unregistered group has
    /// no registry entry to remove — explain instead of arming.
    fn remove_pressed(&mut self, store: &WallStore, now_ms: i64, effects: &mut Vec<Effect>) {
        let Some(group) = store
            .state()
            .group_by_name(store.state().focused_workspace.as_deref())
        else {
            return;
        };
        if !group.registered {
            self.notices.show(
                "already unregistered — sessions live in tmux; x closes them individually",
                5000,
                now_ms,
            );
            return;
        }
        let name = group.name.clone();
        let live = group.sessions.iter().filter(|s| s.live()).count();
        if self.armed_remove.press(&name, now_ms) {
            self.notices.clear_prefix("press X again to remove ");
            effects.push(Effect::RemoveWorkspace { name, kill: false });
        } else {
            // With live sessions the p8.4 `K` clause is appended
            // (workspace_remove.rs owns the wording; zero live omit it).
            self.notices
                .show(remove_arm_notice(&name, live), 3000, now_ms);
        }
    }

    /// `K` while the `X` remove arm is live (p8.4): confirm removal AND kill
    /// every live session. Returns false when nothing (unexpired) is armed,
    /// so the caller treats `K` as an ordinary unbound key. A refetch may
    /// have unregistered the target mid-arm — re-check before firing.
    fn kill_remove_pressed(
        &mut self,
        store: &WallStore,
        now_ms: i64,
        effects: &mut Vec<Effect>,
    ) -> bool {
        let Some(name) = confirm_kill_target(&mut self.armed_remove, now_ms) else {
            return false;
        };
        self.notices.clear_prefix("press X again to remove ");
        let registered = store
            .state()
            .group_by_name(Some(&name))
            .is_some_and(|g| g.registered);
        if registered {
            effects.push(Effect::RemoveWorkspace { name, kill: true });
        }
        true // stale arm: consumed as a no-op
    }

    fn disarm_remove(&mut self) {
        if self.armed_remove.armed_id().is_none() {
            return;
        }
        self.armed_remove.disarm();
        self.notices.clear_prefix("press X again to remove ");
    }

    /// The key after the `r` prefix (p16-restart, spec restart "TUI
    /// chords"). Anything that is not `r`/`a`/`d` cancels: the prefix is
    /// consumed, a pending `r r` arm disarms, and the key does NOT fall
    /// through to its own binding — a mistyped chord must never fire an
    /// unrelated command.
    fn restart_chord(&mut self, keymap: &Keymap, key: &str) -> Option<ActionId> {
        self.notices.clear_prefix(RESTART_PREFIX_HINT);
        let action = keymap.resolve(Layer::RestartChord, key).map(|r| r.action);
        if action.is_none() {
            self.disarm_restart();
        }
        action
    }

    /// `r r`: armed double-press restart of the focused session (spec
    /// restart "TUI chords"). The first press arms with the wording that
    /// promises the conversation resumes — or, on a `working`/`needs-input`
    /// session, warns that a second press restarts it anyway; the second
    /// press within the 3s window fires, forcing when the session was busy
    /// at arm time (design.md D2: the armed press IS the force).
    fn restart_focused_pressed(
        &mut self,
        store: &WallStore,
        now_ms: i64,
        effects: &mut Vec<Effect>,
    ) {
        let Some(session) = store
            .state()
            .session_by_id(store.state().focused_session_id.as_deref())
        else {
            return;
        };
        if !session.live() {
            // A restorable placeholder has no tmux pane to respawn — Enter
            // restores it instead.
            self.notices.show("no live session to restart", 2000, now_ms);
            return;
        }
        let busy = session.status == "working" || session.needs_input();
        // The display name, not the label: post-p13 that is what the tile
        // the user is looking at calls this session.
        let (id, name) = (session.id.clone(), session.display_name().to_owned());
        match self.armed_restart.press(&id, busy, now_ms) {
            Some(force) => {
                self.clear_restart_arm_notice();
                self.notices.show(
                    format!("restarting {name} — resumes the conversation"),
                    5000,
                    now_ms,
                );
                effects.push(Effect::RestartSession { id, name, force });
            }
            None => {
                let text = restart_arm_notice(&name, busy);
                self.notices.show(text.clone(), 3000, now_ms);
                self.armed_restart_notice = Some(text);
            }
        }
    }

    /// `r a`: restart every idle/done session of the focused workspace —
    /// one call per id (the daemon has no workspace filter), fired at once,
    /// no arming. Busy sessions were never targets, so a workspace with
    /// nothing idle says so instead of firing an empty round.
    fn restart_workspace_pressed(
        &mut self,
        store: &WallStore,
        now_ms: i64,
        effects: &mut Vec<Effect>,
    ) {
        let Some(workspace) = store.state().focused_workspace.clone() else {
            return;
        };
        let ids = store.restart_targets_for_workspace();
        if ids.is_empty() {
            self.notices.show(
                format!("nothing to restart in {workspace} (all busy or none)"),
                3000,
                now_ms,
            );
            return;
        }
        self.notices.show(
            format!("restarting {} sessions in {workspace}", ids.len()),
            5000,
            now_ms,
        );
        effects.push(Effect::RestartWorkspace { ids });
    }

    fn disarm_restart(&mut self) {
        if self.armed_restart.armed_id().is_none() {
            return;
        }
        self.armed_restart.disarm();
        self.clear_restart_arm_notice();
    }

    /// Clear the `r r` arm notice and only it — the plain and busy wordings
    /// share no prefix, so the line that was actually shown is remembered
    /// rather than guessed.
    fn clear_restart_arm_notice(&mut self) {
        if let Some(text) = self.armed_restart_notice.take() {
            self.notices.clear_prefix(&text);
        }
    }
}
