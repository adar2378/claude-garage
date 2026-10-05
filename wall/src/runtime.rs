//! Runtime: tokio; PTY readers and the SSE stream as tasks feeding ONE mpsc
//! event channel; a single state-owning loop applies events and draws —
//! "daemon is authoritative, render is pure", same shape as p8 (task 1.3).
//!
//! Wave 3 (task 3.3) adds the full three-layer key dispatch (spec
//! tui-key-routing):
//!   - garage layer: single-key app commands through the ported
//!     [`GarageCommand`] mapping, incl. every p8.1–p8.4 lifecycle key
//!     (`1-9 [ ] a A n N x X K w m R Enter ? q`), the armed `x`/`X`/`K`
//!     double-press flows and their exact Dart strip-notice wording, plus
//!     the p16-restart `r` prefix (`r r` / `r a` / `r d`);
//!   - engaged layer: every key re-encoded to raw bytes
//!     ([`crate::input::encode`]) and written to the focused [`TileClient`],
//!     except the reserved Ctrl+G disengage chord; `Event::Paste` is
//!     forwarded wrapped in bracketed-paste guards;
//!   - overlay layer: consumes everything, routed to a placeholder handler
//!     keyed by [`OverlayKind`] (wave 4 renders the overlays and replaces the
//!     placeholder bodies).
//!
//! Effects (daemon API calls) never run on the state loop: the garage router
//! is pure — it returns [`Effect`] values which are executed on blocking
//! tasks whose results (notices, refetched listings, settle markers) flow
//! back through the same event channel. The draw here is still a minimal
//! placeholder; the real UI surfaces are wave 4's caller of this loop.

use std::collections::HashSet;
use std::io::Write as _;
use std::ops::Range;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use crossterm::event::{
    Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::Line;
use ratatui::widgets::{Block, BorderType, Widget as _};
use serde_json::Value;
use tokio::sync::mpsc;

use crate::api::client::{ApiError, GarageClient};
use crate::api::models::{
    FinishAction, HooksInstallResult, RestartResponse, SessionInfo, UsageInfo, WorkspaceInfo,
    WorktreeRecord,
};
use crate::api::sse::SseClient;
use crate::input::encode::encode_key;
use crate::input::links::url_at;
use crate::input::paste::wrap_bracketed_paste;
use crate::state::armed_action::{ArmedAction, ArmedClose};
use crate::state::persistence::{self, WallFile};
use crate::state::salience::restorable_session_ids;
use crate::state::store::{
    garage_command_for, restart_arm_notice, restart_command_for, ArmedRestart, GarageCommand,
    WallStore, RESTART_PREFIX,
};
use crate::state::views::{self, ViewSummary};
use crate::state::wall_state::{KeyLayer, OverlayKind, WallSession, WallState};
use crate::state::workspace_remove::{confirm_kill_target, kill_remove_notice, remove_arm_notice};
use crate::ui::escalation::{EscalationPolicy, HEARTBEAT_INTERVAL_MS};
use crate::ui::help::render_help;
use crate::ui::hit_targets::{
    rail_target_at, tile_index_at, triage_row_index_at, view_picker_row_index_at, RailTarget,
};
use crate::ui::layout::{tile_inner, wall_layout, WallLayout};
use crate::ui::pet;
use crate::ui::rail::render_rail;
use crate::ui::registry::{FrozenTile, TileRegistry};
use crate::ui::scroll::{capture_frozen, page_lines, tmux_history_size, ScrollModel, WHEEL_LINES};
use crate::ui::strip::{render_strip, PetRender};
use crate::ui::theme::colors;
use crate::ui::tile::{render_centered_lines, render_tile, TileView};
use crate::ui::triage::{render_triage, triage_queue_rows, wrap_selection};
use crate::ui::view_picker::{picker_entries, render_view_picker, ViewPickerState};
use crate::ui::view_strip::render_view_strip;
use crate::ui::workspace_add::{render_workspace_add, PickOutcome, WorkspaceAddForm};
use crate::ui::worktree_finish::{
    finish_notice, render_worktree_finish, FinishChoice, WorktreeFinishState,
};

const FRAME_CAP: Duration = Duration::from_millis(33); // ~30fps render cap (not spec-mandated)

/// Elapsed timers and the done-fade must advance without input; 10s keeps
/// the "refresh ≤ 30s" contract with margin at negligible render cost.
const ELAPSED_TICK: Duration = Duration::from_secs(10);

/// Throttle for the frozen `+N lines` counter refresh (one tmux call per
/// frozen tile).
const FROZEN_COUNT_REFRESH: Duration = Duration::from_secs(1);

/// `GET /api/usage` poll cadence (spec tui-context-meters "Strip usage
/// chip": "refreshed at least every 60s"). No SSE push exists for
/// account-wide usage — polling (plus the once-at-startup fetch) is the
/// only source.
const USAGE_POLL_INTERVAL: Duration = Duration::from_secs(60);

/// Delay before the one-time statusline-install hint may appear (spec
/// tui-context-meters "Install affordance": "~30s after startup").
const INSTALL_HINT_DELAY: Duration = Duration::from_secs(30);

/// How long the install hint stays up if never dismissed by a keypress
/// (spec: "one-time" — it never reappears after this either).
const INSTALL_HINT_TTL_MS: i64 = 8000;

/// The install action's success wording (spec tui-context-meters "Install
/// affordance": exact strip-notice text, verbatim — harnesses grep it).
const INSTALL_STATUSLINE_SUCCESS: &str =
    "statusline feed installed — meters go live as agents work";

/// The one event channel every producer feeds (design.md: single
/// state-owning loop).
pub enum AppEvent {
    /// Terminal input (keys, resize, paste, mouse) from the crossterm reader
    /// task.
    Term(Event),
    /// A tile PTY produced output — render is dirty.
    TileOutput,
    Workspaces(Vec<WorkspaceInfo>),
    Sessions(Vec<SessionInfo>),
    /// `GET /api/usage` poll result (spec tui-context-meters "Strip usage
    /// chip").
    Usage(UsageInfo),
    /// SSE `status` event.
    Status {
        id: String,
        status: String,
        since: Option<i64>,
    },
    /// The daemon SSE stream's connect state changed (spec tui-pit-pet "One-
    /// row sprites and derived mood": "daemon SSE disconnected → box").
    /// Emitted only on transitions by `spawn_sse_task` — `true` on
    /// `on_connected`, `false` the moment it drops into the poll fallback.
    Connection(bool),
    /// A transient strip notice (fetch/effect errors, lifecycle results).
    Notice { text: String, ttl_ms: i64 },
    /// A spawn effect settled (success or failure) — clears the busy guard.
    SpawnSettled,
    /// A restore effect settled for this session id.
    RestoreSettled(String),
    /// The `I` install effect settled: both results (statusline first, then
    /// hooks — spec tui-hooks-install) in one event, so one notice and one
    /// busy guard.
    InstallSettled {
        statusline: Result<(), ApiError>,
        hooks: Result<HooksInstallResult, ApiError>,
    },
    /// A closed session left a worktree behind (the DELETE response's
    /// record) — opens the finish overlay (spec tui-worktree-finish).
    WorktreeClosed(WorktreeRecord),
    /// The finish overlay's merge/discard call settled: `error: None` moves
    /// on (closing the overlay when nothing else waits) with a strip notice;
    /// `Some` keeps the record on screen with the daemon's message inline.
    WorktreeFinishSettled {
        record: WorktreeRecord,
        action: FinishAction,
        error: Option<String>,
    },
    /// The add-workspace PUT settled: `error: None` closes the overlay and
    /// focuses the new workspace; `Some` keeps it open with the inline error.
    WorkspaceAddSettled {
        name: String,
        error: Option<String>,
    },
    /// The `w` overlay's Ctrl+O folder-picker call settled — applied to the
    /// form only while the overlay is still open and picking.
    PickDirectorySettled(PickOutcome),
    Quit,
}

pub type EventSender = mpsc::UnboundedSender<AppEvent>;

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// A tiny xorshift64 PRNG (design.md decision 2: `pet.rs` takes time and
/// randomness injected as `rng: &mut impl FnMut() -> f64`; this is the one
/// concrete source the runtime feeds it — no new crate for one PRNG). Not
/// cryptographic, not seeded for reproducibility across runs: the pet's
/// idle strolls and chatter picks only need to look unpredictable, not be.
fn xorshift_next(state: &mut u64) -> f64 {
    let mut x = *state;
    x ^= x << 13;
    x ^= x >> 7;
    x ^= x << 17;
    *state = x;
    // Top 53 bits → [0.0, 1.0), matching the precision of an f64 mantissa.
    (x >> 11) as f64 * (1.0 / (1u64 << 53) as f64)
}

// ── Strip notices ─────────────────────────────────────────────────────────

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

// ── Effects (the caller-side of store-declined commands) ─────────────────

/// Daemon API calls the garage router asks the loop to perform — the store
/// never does IO, and neither does the router: effects run on blocking tasks
/// and report back through the event channel.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Effect {
    /// `n`/`N`: `POST /api/sessions`.
    Spawn {
        workspace: String,
        label: String,
        worktree: bool,
    },
    /// Enter on a restorable tile / one `R` target:
    /// `POST /api/sessions/restore {id}`.
    Restore { id: String },
    /// Confirmed `x`-`x`: `DELETE /api/sessions/<id>` (`?meta=1` for a
    /// restorable placeholder).
    Close {
        id: String,
        label: String,
        meta_only: bool,
    },
    /// Confirmed `X`-`X` (registry-only) or `X` then `K` (`kill: true`,
    /// `?sessions=kill`).
    RemoveWorkspace { name: String, kill: bool },
    /// The `w` overlay's validated submit: `PUT /api/workspaces {name, dir}`.
    AddWorkspace { name: String, dir: String },
    /// The `w` overlay's Ctrl+O: `POST /api/pick-directory` (native macOS
    /// folder dialog). Runs on its own blocking task like every effect, so
    /// the up-to-120 s dialog never stalls other effects.
    PickDirectory,
    /// `q`: orderly quit.
    Quit,
    /// `I`: `POST /api/statusline/install` then `POST /api/hooks/install`
    /// on one blocking task (spec tui-context-meters "Install affordance",
    /// spec tui-hooks-install).
    Install,
    /// The finish overlay's `m` / confirmed `d d`: `POST
    /// /api/worktrees/finish` with the held record (spec
    /// tui-worktree-finish). Keep never becomes an effect.
    FinishWorktree {
        record: WorktreeRecord,
        action: FinishAction,
    },
    /// `t`: launch a detached OS terminal window attached to the focused
    /// LIVE session (macOS only — the router declines before this is ever
    /// built off macOS; see [`crate::ui::window_open`]).
    OpenWindow { id: String, label: String },
    /// Confirmed `r r` (p16-restart): `POST /api/sessions/restart {id,
    /// force}`. `name` is captured at press time so the result notice names
    /// what the user was looking at, even if the tile's title moved on
    /// while the call was in flight.
    RestartSession {
        id: String,
        name: String,
        force: bool,
    },
    /// `r a`: one restart call per idle/done session of the focused
    /// workspace (the daemon has no workspace filter), aggregated into one
    /// result notice.
    RestartWorkspace { ids: Vec<String> },
    /// `r d`: `POST /api/daemon/restart` — tmux sessions untouched; the SSE
    /// task's existing reconnect handles the dropped connection.
    RestartDaemon,
}

// ── Garage-layer router ──────────────────────────────────────────────────

/// First free `claude-N` label in the workspace (the default label family —
/// port of the Dart `_nextLabel`).
fn next_label(state: &WallState, workspace: &str) -> String {
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
fn is_printable(key: &str) -> bool {
    key.chars()
        .next()
        .is_some_and(|c| c as u32 >= 0x20 && c != '\u{7f}')
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
    /// [`garage_key_string`]). The garage layer consumes everything; the
    /// returned effects are the caller's IO.
    pub fn on_garage_key(
        &mut self,
        store: &mut WallStore,
        key: &str,
        now_ms: i64,
    ) -> Vec<Effect> {
        let mut effects = Vec::new();
        // p16-restart (design.md D5): the `r` prefix is one-shot — the very
        // next key is the chord's second half, resolved BEFORE anything else
        // so `r a` / `r d` never reach the plain `a` / `d` bindings.
        if std::mem::take(&mut self.restart_prefix) {
            self.restart_chord(store, key, now_ms, &mut effects);
            return effects;
        }
        // p8.4: while an `X` remove arm is active, `K` confirms the removal
        // WITH session kill — checked BEFORE the disarm pass (K would
        // otherwise disarm the very arm it confirms). Outside a live arm `K`
        // falls through as an ordinary unbound key (typing hint).
        if key == "K" && self.kill_remove_pressed(store, now_ms, &mut effects) {
            return effects;
        }
        // Any key but `x` disarms a pending close (spec p8.1: "any other key
        // disarms"); same for `X` and a pending workspace removal (p8.3) —
        // the two arms are independent, so an `X` disarms a pending `x` and
        // vice versa. `K` only survives the remove arm via the branch above.
        if key != "x" {
            self.disarm_close();
        }
        if key != "X" {
            self.disarm_remove();
        }
        // p16-restart: the `r r` arm survives only the prefix key itself —
        // the second `r` arrives through the chord branch above, so every
        // key that reaches here (including a bare `r` opening a fresh
        // prefix) leaves the arm alone or disarms it, same rule as `x`.
        if key != RESTART_PREFIX {
            self.disarm_restart();
        }
        // p16-restart: `r` alone is a prefix, not a command — consume it and
        // wait for the chord. The hint would bury a live `r r` arm notice
        // (which already says what the next press does), so it yields to it.
        if key == RESTART_PREFIX {
            self.restart_prefix = true;
            if self.armed_restart.armed_id().is_none() {
                self.notices.show(RESTART_PREFIX_HINT, 3000, now_ms);
            }
            return effects;
        }
        // `P` (spec tui-pit-pet "Opt-in roster cycled by `P`"): not a
        // `GarageCommand` — `pet.rs` (the render module) has no business
        // being a store dependency, so this stays here rather than growing
        // `state::store`'s command enum for one field's cycling.
        if key == "P" {
            self.cycle_pet_pressed(store, now_ms);
            return effects;
        }
        let Some(command) = garage_command_for(key) else {
            // Unbound garage keys are consumed (never reach an agent), but
            // silence reads as a dead wall — show where the keys actually go.
            if is_printable(key) {
                self.notices.show_typing_hint(now_ms);
            }
            return effects;
        };
        // `d` needs to know afterward whether it detached (solo view) or
        // rejoined (default view) to show the right notice — special-cased
        // ahead of the generic dispatch below.
        if let GarageCommand::DetachFocused = command {
            self.detach_pressed(store, now_ms);
            return effects;
        }
        if store.dispatch(command) {
            return effects;
        }
        // Commands the store declined: effects (spawn/restore/close/quit),
        // the empty-jump notice, and Enter on a restorable tile (restore).
        match command {
            GarageCommand::Spawn { worktree } => self.spawn(store, worktree, &mut effects),
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
                    self.restore(id, &mut effects);
                }
            }
            GarageCommand::RestoreAll => self.restore_all(store, now_ms, &mut effects),
            GarageCommand::Close => self.close_pressed(store, now_ms, &mut effects),
            GarageCommand::WorkspaceRemove => self.remove_pressed(store, now_ms, &mut effects),
            GarageCommand::InstallStatusline => self.install_pressed(&mut effects),
            GarageCommand::OpenWindow => self.open_window_pressed(store, now_ms, &mut effects),
            _ => {}
        }
        effects
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
    fn install_pressed(&mut self, effects: &mut Vec<Effect>) {
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
        let current = store
            .state()
            .pet
            .as_deref()
            .and_then(pet::Species::from_str);
        let next = pet::Species::cycle(current);
        store.set_pet(next.map(|s| s.as_str().to_owned()));
        let text = match next {
            Some(species) => format!("{} is on the wall", species.label()),
            None => "the strip is quiet again".to_owned(),
        };
        self.notices.show(text, 3000, now_ms);
    }

    /// A click on a restorable placeholder restores it (Enter's landing,
    /// spec p8.1) — same in-flight dedup as the key path.
    pub fn restore_clicked(&mut self, id: String) -> Vec<Effect> {
        let mut effects = Vec::new();
        self.restore(id, &mut effects);
        effects
    }

    /// Restore one session, deduplicated against in-flight restores.
    fn restore(&mut self, id: String, effects: &mut Vec<Effect>) {
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
    fn restart_chord(
        &mut self,
        store: &WallStore,
        key: &str,
        now_ms: i64,
        effects: &mut Vec<Effect>,
    ) {
        self.notices.clear_prefix(RESTART_PREFIX_HINT);
        match restart_command_for(key) {
            Some(GarageCommand::RestartFocused) => {
                self.restart_focused_pressed(store, now_ms, effects)
            }
            Some(GarageCommand::RestartWorkspace) => {
                self.restart_workspace_pressed(store, now_ms, effects)
            }
            Some(GarageCommand::RestartDaemon) => {
                self.notices.show("restarting the daemon…", 5000, now_ms);
                effects.push(Effect::RestartDaemon);
            }
            _ => self.disarm_restart(),
        }
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

// ── Key-event reduction ──────────────────────────────────────────────────

/// Reduce a crossterm key event to the garage layer's string form (the
/// Dart handler's `event.character`, Enter normalized to `"\r"`).
/// `None` = nothing typeable (consumed silently — Ctrl/Alt-modified keys,
/// Esc, arrows … are all unbound in the garage layer).
pub fn garage_key_string(key: &KeyEvent) -> Option<String> {
    match key.code {
        KeyCode::Enter => Some("\r".to_owned()),
        // p10 (spec tui-views "View strip and group frame"): Tab cycles the
        // focused workspace's views — the wave-1b reservation this fills in
        // (`garage_command_for` already maps `"\t"` to `CycleView`).
        KeyCode::Tab => Some("\t".to_owned()),
        KeyCode::Char(c)
            if !key
                .modifiers
                .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
        {
            Some(c.to_string())
        }
        _ => None,
    }
}

/// What an engaged-layer key event does (spec: "Engage and disengage" +
/// "Verbatim byte re-encoding").
#[derive(Debug, PartialEq, Eq)]
pub enum EngagedInput {
    /// The reserved Ctrl+G disengage chord — never forwarded.
    Disengage,
    /// Raw bytes for the focused tile's PTY (Esc included — Esc is always
    /// forwarded while engaged, never a layer control).
    Write(Vec<u8>),
    /// Nothing forwardable (bare modifier press etc).
    Nothing,
}

pub fn engaged_input_for(key: &KeyEvent) -> EngagedInput {
    if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('g') {
        return EngagedInput::Disengage;
    }
    match encode_key(key) {
        Some(bytes) if !bytes.is_empty() => EngagedInput::Write(bytes),
        _ => EngagedInput::Nothing,
    }
}

/// Overlay-layer key routing (task 4.3/4.4): the overlay layer consumes
/// everything, keyed by [`OverlayKind`]. Help: Esc/`?`/`q` close. Triage
/// queue: `j`/`k` (and arrows) move the selection with wrap, Enter closes
/// and jump-engages the selected session, Esc closes. Add-workspace: the
/// text field owns the keys — Enter submits (validation via the injected
/// `dir_exists`, so this routes pure in tests), Ctrl+O opens the native
/// folder picker (ignored while busy or already picking), Esc cancels and
/// resets the form. Returned effects are the caller's IO (the PUT / the
/// picker call).
pub fn handle_overlay_key(
    store: &mut WallStore,
    key: &KeyEvent,
    queue_selection: &mut usize,
    form: &mut WorkspaceAddForm,
    view_picker: &mut ViewPickerState,
    home: &str,
    dir_exists: &dyn Fn(&str) -> bool,
) -> (Vec<Effect>, Option<String>) {
    let Some(kind) = store.state().overlay else {
        return (Vec::new(), None);
    };
    let mut effects = Vec::new();
    let mut notice = None;
    let plain_char = match key.code {
        KeyCode::Char(c)
            if !key
                .modifiers
                .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
        {
            Some(c)
        }
        _ => None,
    };
    match kind {
        OverlayKind::Help => {
            if key.code == KeyCode::Esc || matches!(plain_char, Some('?' | 'q')) {
                store.close_overlay();
            }
        }
        OverlayKind::TriageQueue => {
            let len = triage_queue_rows(&store.state().sessions).len();
            match (key.code, plain_char) {
                (KeyCode::Esc, _) => store.close_overlay(),
                (KeyCode::Down, _) | (_, Some('j')) => {
                    *queue_selection = wrap_selection(*queue_selection, 1, len);
                }
                (KeyCode::Up, _) | (_, Some('k')) => {
                    *queue_selection = wrap_selection(*queue_selection, -1, len);
                }
                (KeyCode::Enter, _) => {
                    // Close first, then jump — jump_to_session refuses stale
                    // rows (gone / no longer blocked), same as the Dart flow.
                    store.close_overlay();
                    let target = triage_queue_rows(&store.state().sessions)
                        .get((*queue_selection).min(len.saturating_sub(1)))
                        .map(|s| s.id.clone());
                    if let Some(id) = target {
                        store.jump_to_session(&id);
                    }
                }
                _ => {}
            }
        }
        OverlayKind::WorkspaceAdd => match key.code {
            KeyCode::Esc => {
                form.reset();
                store.close_overlay();
            }
            KeyCode::Backspace => form.backspace(),
            KeyCode::Enter => {
                let existing: Vec<String> = store
                    .state()
                    .workspaces
                    .iter()
                    .map(|w| w.name.clone())
                    .chain(store.state().groups.iter().map(|g| g.name.clone()))
                    .collect();
                if let Some((name, dir)) = form.submit(home, &existing, dir_exists) {
                    effects.push(Effect::AddWorkspace { name, dir });
                }
            }
            // Before the plain-char arm: Ctrl+O must never type an `o`.
            KeyCode::Char('o' | 'O') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                if form.start_pick() {
                    effects.push(Effect::PickDirectory);
                }
            }
            KeyCode::Char(c)
                if !key
                    .modifiers
                    .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
            {
                form.insert_char(c);
            }
            _ => {}
        },
        OverlayKind::ViewPicker => {
            notice = handle_view_picker_key(store, key, plain_char, view_picker);
        }
        // Routed by `App::handle_key` to [`handle_worktree_finish_key`]: the
        // armed discard needs the clock, which this router never sees.
        OverlayKind::WorktreeFinish => {}
    }
    (effects, notice)
}

/// The view-picker's selectable rows for the CURRENTLY focused
/// workspace/session (spec tui-views "Move to a group"): every other view of
/// the workspace plus the trailing "new group…" row — the focused session's
/// own current view is excluded (see [`picker_entries`]). Shared by the key,
/// click, and render paths so all three can never disagree about what's
/// selectable — the row a keypress or click resolves to is always the same
/// row the modal draws.
fn view_picker_entries(store: &WallStore) -> Vec<String> {
    let Some(workspace) = store.state().focused_workspace.clone() else {
        return picker_entries(&[], "");
    };
    let current_view = store.state().focused_view_name(&workspace).to_owned();
    let view_names: Vec<String> =
        store.state().views_for(&workspace).into_iter().map(|v| v.name).collect();
    picker_entries(&view_names, &current_view)
}

/// `D` picker key routing (spec tui-views "Move to a group"): `j`/`k`/arrows
/// move the selection, Enter on a view moves the focused session there
/// (closing the overlay and returning the "moved <label> → <view>" strip
/// notice), Enter on the trailing "new group…" row switches to its
/// text-input sub-mode (reusing the workspace_add field pattern) whose own
/// Enter creates/moves into that name; Esc cancels from either sub-mode.
fn handle_view_picker_key(
    store: &mut WallStore,
    key: &KeyEvent,
    plain_char: Option<char>,
    view_picker: &mut ViewPickerState,
) -> Option<String> {
    let entries = view_picker_entries(store);

    if view_picker.new_group_input.is_some() {
        match key.code {
            KeyCode::Esc => {
                view_picker.reset();
                store.close_overlay();
            }
            KeyCode::Backspace => view_picker.backspace(),
            KeyCode::Enter => {
                let name = view_picker.new_group_input.clone().unwrap_or_default();
                let trimmed = name.trim().to_owned();
                view_picker.reset();
                if trimmed.is_empty() {
                    store.close_overlay();
                } else {
                    return move_to_view_and_notice(store, &trimmed);
                }
            }
            KeyCode::Char(c) if plain_char == Some(c) => view_picker.insert_char(c),
            _ => {}
        }
        return None;
    }

    // The trailing "new group…" row is always the last entry (`picker_entries`
    // guarantees it — never empty even with zero other views).
    let entries_len = entries.len();
    match (key.code, plain_char) {
        (KeyCode::Esc, _) => {
            view_picker.reset();
            store.close_overlay();
        }
        (KeyCode::Down, _) | (_, Some('j')) => {
            view_picker.selected = wrap_selection(view_picker.selected, 1, entries_len);
        }
        (KeyCode::Up, _) | (_, Some('k')) => {
            view_picker.selected = wrap_selection(view_picker.selected, -1, entries_len);
        }
        (KeyCode::Enter, _) => {
            let sel = view_picker.selected.min(entries_len.saturating_sub(1));
            if sel == entries_len - 1 {
                view_picker.new_group_input = Some(String::new());
            } else if let Some(name) = entries.get(sel).cloned() {
                view_picker.reset();
                return move_to_view_and_notice(store, &name);
            }
        }
        _ => {}
    }
    None
}

/// Close the picker, move the focused session into `view_name`, and return
/// the strip notice on success (spec tui-views design.md: "moved <label> →
/// <view>" — kept stable for e2e greps).
fn move_to_view_and_notice(store: &mut WallStore, view_name: &str) -> Option<String> {
    let label = store
        .state()
        .session_by_id(store.state().focused_session_id.as_deref())
        .map(|s| s.label.clone());
    store.close_overlay();
    if store.move_focused_to_view(view_name) {
        label.map(|label| format!("moved {label} → {view_name}"))
    } else {
        None
    }
}

/// Finish-overlay key routing (spec tui-worktree-finish): `m` merges, `d`
/// arms and a second `d` within the window discards, `k`/Esc keeps (no
/// request; the overlay closes once no record waits). Everything else is
/// consumed — the overlay is modal — and disarms a pending discard. While a
/// request is in flight every choice is ignored.
fn handle_worktree_finish_key(
    store: &mut WallStore,
    key: &KeyEvent,
    finish: &mut WorktreeFinishState,
    now_ms: i64,
) -> Vec<Effect> {
    let plain_char = match key.code {
        KeyCode::Char(c)
            if !key
                .modifiers
                .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
        {
            Some(c)
        }
        _ => None,
    };
    let choice = match (key.code, plain_char) {
        (KeyCode::Esc, _) | (_, Some('k')) => finish.keep(),
        (_, Some('m')) => finish.merge(),
        (_, Some('d')) => finish.discard(now_ms),
        _ => {
            finish.other_key();
            FinishChoice::Nothing
        }
    };
    match choice {
        FinishChoice::Run(record, action) => vec![Effect::FinishWorktree { record, action }],
        FinishChoice::Kept { close: true } => {
            store.close_overlay();
            Vec::new()
        }
        FinishChoice::Kept { close: false } | FinishChoice::Nothing => Vec::new(),
    }
}

/// Queue a closed session's worktree record and bring the finish overlay up
/// (spec tui-worktree-finish). The DELETE settles asynchronously, so the
/// wall may have moved on: an engaged tile is disengaged and any other
/// overlay is closed — the record is the only handle on the worktree and
/// must not wait behind them. Returns true when another overlay was
/// displaced (the caller resets that overlay's state).
fn show_worktree_finish(
    store: &mut WallStore,
    finish: &mut WorktreeFinishState,
    record: WorktreeRecord,
) -> bool {
    finish.push(record);
    let mut displaced = false;
    match (store.state().layer, store.state().overlay) {
        (KeyLayer::Overlay, Some(OverlayKind::WorktreeFinish)) => return false,
        (KeyLayer::Overlay, _) => {
            store.close_overlay();
            displaced = true;
        }
        (KeyLayer::Engaged, _) => store.disengage(),
        (KeyLayer::Garage, _) => {}
    }
    store.open_overlay(OverlayKind::WorktreeFinish);
    displaced
}

/// Apply a finish call's settle: success returns the strip notice and
/// closes the overlay once no record waits; a failure keeps the overlay
/// open with the daemon's message inline (no notice).
fn worktree_finish_settled(
    store: &mut WallStore,
    finish: &mut WorktreeFinishState,
    record: &WorktreeRecord,
    action: FinishAction,
    error: Option<String>,
) -> Option<String> {
    let failed = error.is_some();
    let close = finish.settled(error);
    if close && store.state().overlay == Some(OverlayKind::WorktreeFinish) {
        store.close_overlay();
    }
    (!failed).then(|| finish_notice(record, action))
}

// ── Latency instrumentation ──────────────────────────────────────────────

/// `GARAGE_TUI_KEYLOG=<file>` appends `<epoch_us> <layer> <desc>` per
/// handled key — the same first-field contract as the Dart TUI's keylog and
/// the e2e harnesses.
struct KeyLog(Option<std::fs::File>);

impl KeyLog {
    fn new() -> KeyLog {
        KeyLog(std::env::var("GARAGE_TUI_KEYLOG").ok().and_then(|p| {
            std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(p)
                .ok()
        }))
    }

    fn log(&mut self, layer: &str, desc: &str) {
        if let Some(f) = self.0.as_mut() {
            let us = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|d| d.as_micros())
                .unwrap_or(0);
            let _ = writeln!(f, "{us} {layer} {desc}");
            let _ = f.flush();
        }
    }
}

// ── Effect execution ─────────────────────────────────────────────────────

fn send_notice(tx: &EventSender, text: String, ttl_ms: i64) {
    let _ = tx.send(AppEvent::Notice { text, ttl_ms });
}

/// The Dart failure wording: an API-level error carries the daemon's
/// message (`"<prefix>: <message>"`); a transport failure is just the
/// prefix.
fn failure_notice(prefix: &str, e: &ApiError) -> String {
    match e {
        ApiError::Status { message, .. } => format!("{prefix}: {message}"),
        ApiError::Transport(_) => prefix.to_owned(),
    }
}

/// The one strip line for `I` (spec tui-hooks-install): the hooks result,
/// then the statusline result. Each side is reported on its own — a failure
/// names which install failed and its error without hiding the other — and
/// an idempotent hooks re-run reads "already installed", never as an error.
/// The statusline success keeps its exact wording.
fn install_notice(
    statusline: &Result<(), ApiError>,
    hooks: &Result<HooksInstallResult, ApiError>,
) -> String {
    let hooks = match hooks {
        Ok(result) if result.already_installed => "hooks already installed".to_owned(),
        Ok(_) => "hooks installed".to_owned(),
        Err(e) => failure_notice("hooks install failed", e),
    };
    let statusline = match statusline {
        Ok(()) => INSTALL_STATUSLINE_SUCCESS.to_owned(),
        Err(e) => failure_notice("statusline install failed", e),
    };
    format!("{hooks} · {statusline}")
}

/// p16-restart: the strip line for ONE `r r` restart's response. Every
/// branch names an outcome — a response that restarted nothing still says
/// so, rather than leaving the strip silent about a key the user pressed.
fn restart_notice(name: &str, response: &RestartResponse) -> String {
    if let Some(entry) = response.restarted.first() {
        return if entry.resumed {
            format!("{name} restarted")
        } else {
            format!("{name} restarted fresh (no conversation to resume)")
        };
    }
    if let Some(entry) = response.failed.first() {
        return format!("{name}: {}", entry.error);
    }
    if let Some(entry) = response.skipped.first() {
        return format!("{name} skipped — {}", entry.status);
    }
    format!("{name}: the daemon reported no outcome")
}

/// p16-restart: the aggregated `r a` result line. Skips and failures are
/// named only when there are any — the targets were filtered to idle/done
/// before the calls went out, so "skipped 0" is the normal case and would
/// be pure noise; a skip that does appear is a session that went busy
/// between the keypress and the call.
fn restart_all_notice(restarted: usize, skipped: usize, failures: &[String]) -> String {
    let mut parts = vec![format!("restarted {restarted}")];
    if skipped > 0 {
        parts.push(format!("skipped {skipped} (working/waiting)"));
    }
    if let Some(first) = failures.first() {
        parts.push(format!("failed {}: {first}", failures.len()));
    }
    parts.join(" · ")
}

/// Refetch both listings and feed them into the channel (the Dart
/// `_refetchSessions`: workspaces too, so worktree flags never derive from a
/// stale registered dir).
fn fetch_and_send(client: &GarageClient, tx: &EventSender) {
    if let Ok(workspaces) = client.fetch_workspaces() {
        let _ = tx.send(AppEvent::Workspaces(workspaces));
    }
    if let Ok(sessions) = client.fetch_sessions() {
        let _ = tx.send(AppEvent::Sessions(sessions));
    }
}

/// Where a bracketed paste goes (see [`App::handle_paste`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PasteTarget {
    /// Engaged: the focused tile's PTY.
    Tile,
    /// The `w` overlay's path field.
    WorkspaceField,
    /// Consumed — garage layer and every other overlay.
    Drop,
}

/// Pure paste routing: only the engaged layer and the `w` overlay's text
/// field accept pastes; nothing pasted ever reaches the key dispatch.
fn paste_target(layer: KeyLayer, overlay: Option<OverlayKind>) -> PasteTarget {
    match (layer, overlay) {
        (KeyLayer::Engaged, _) => PasteTarget::Tile,
        (KeyLayer::Overlay, Some(OverlayKind::WorkspaceAdd)) => PasteTarget::WorkspaceField,
        _ => PasteTarget::Drop,
    }
}

/// Run one router effect on a blocking task; results flow back through the
/// channel. `Effect::Quit` is the loop's own concern and never lands here.
fn run_effect(effect: Effect, tx: &EventSender, base_url: &str) {
    let tx = tx.clone();
    let base_url = base_url.to_owned();
    tokio::task::spawn_blocking(move || {
        let client = GarageClient::new(Some(base_url));
        match effect {
            Effect::Spawn {
                workspace,
                label,
                worktree,
            } => {
                if let Err(e) = client.spawn_session(&workspace, &label, worktree) {
                    send_notice(&tx, failure_notice("spawn failed", &e), 5000);
                }
                fetch_and_send(&client, &tx);
                let _ = tx.send(AppEvent::SpawnSettled);
            }
            Effect::Restore { id } => {
                match client.restore_session(&id) {
                    Ok(None) => {}
                    Ok(Some(reason)) => send_notice(&tx, format!("restore failed: {reason}"), 5000),
                    Err(e) => send_notice(&tx, failure_notice("restore failed", &e), 5000),
                }
                fetch_and_send(&client, &tx);
                let _ = tx.send(AppEvent::RestoreSettled(id));
            }
            Effect::Close {
                id,
                label,
                meta_only,
            } => {
                match client.delete_session(&id, meta_only) {
                    Ok(Some(record)) => {
                        // Spec tui-worktree-finish: the worktree outlives the
                        // session — hand its record to the finish overlay.
                        send_notice(&tx, format!("closed {label}"), 2000);
                        let _ = tx.send(AppEvent::WorktreeClosed(record));
                    }
                    Ok(None) => send_notice(&tx, format!("closed {label}"), 2000),
                    Err(e) => send_notice(&tx, failure_notice("close failed", &e), 5000),
                }
                fetch_and_send(&client, &tx);
            }
            Effect::RemoveWorkspace { name, kill: false } => {
                match client.remove_workspace(&name, false) {
                    Ok(_) => send_notice(
                        &tx,
                        format!("removed workspace {name} — its sessions keep running"),
                        5000,
                    ),
                    Err(e) => send_notice(&tx, failure_notice("remove failed", &e), 5000),
                }
                fetch_and_send(&client, &tx);
            }
            Effect::RemoveWorkspace { name, kill: true } => {
                match client.remove_workspace(&name, true) {
                    Ok(body) => send_notice(&tx, kill_remove_notice(&name, body.as_ref()), 5000),
                    Err(e) => send_notice(&tx, failure_notice("remove failed", &e), 5000),
                }
                fetch_and_send(&client, &tx);
            }
            Effect::AddWorkspace { name, dir } => match client.put_workspace(&name, &dir) {
                Ok(()) => {
                    // Refetch BEFORE settling so the new group exists when
                    // the settle handler focuses it.
                    fetch_and_send(&client, &tx);
                    let _ = tx.send(AppEvent::WorkspaceAddSettled { name, error: None });
                }
                Err(e) => {
                    let error = match &e {
                        ApiError::Status { message, .. } => message.clone(),
                        ApiError::Transport(_) => "could not register workspace".to_owned(),
                    };
                    let _ = tx.send(AppEvent::WorkspaceAddSettled {
                        name,
                        error: Some(error),
                    });
                }
            },
            Effect::PickDirectory => {
                // Blocks this task (not the loop, not other effects) for as
                // long as the dialog is open.
                let outcome = match client.pick_directory() {
                    Ok(Some(dir)) => PickOutcome::Picked(dir),
                    Ok(None) => PickOutcome::Cancelled,
                    Err(ApiError::Status { status: 501, .. }) => PickOutcome::Unsupported,
                    Err(_) => PickOutcome::Failed,
                };
                let _ = tx.send(AppEvent::PickDirectorySettled(outcome));
            }
            Effect::Install => {
                // Statusline first, then hooks — both always run, so one
                // failing never hides the other's result.
                let statusline = client.install_statusline();
                let hooks = client.install_hooks();
                let _ = tx.send(AppEvent::InstallSettled { statusline, hooks });
            }
            Effect::FinishWorktree { record, action } => {
                let error = match client.finish_worktree(&record, action) {
                    Ok(()) => None,
                    Err(ApiError::Status { status, message }) if message.trim().is_empty() => {
                        Some(format!("{} failed ({status})", action.as_str()))
                    }
                    Err(ApiError::Status { message, .. }) => Some(message),
                    Err(ApiError::Transport(_)) => {
                        Some("could not reach the daemon — try again or keep".to_owned())
                    }
                };
                let _ = tx.send(AppEvent::WorktreeFinishSettled { record, action, error });
            }
            Effect::OpenWindow { id, label } => match crate::ui::window_open::launch(&id) {
                Ok(()) => send_notice(&tx, format!("opened {label} in a new window"), 3000),
                Err(e) => send_notice(&tx, format!("open window failed: {e}"), 5000),
            },
            Effect::RestartSession { id, name, force } => {
                match client.restart_session(&id, force) {
                    Ok(response) => send_notice(&tx, restart_notice(&name, &response), 5000),
                    Err(e) => send_notice(&tx, failure_notice("restart failed", &e), 5000),
                }
                // The pane is new: title, status and context all moved.
                fetch_and_send(&client, &tx);
            }
            Effect::RestartWorkspace { ids } => {
                let mut restarted = 0usize;
                let mut skipped = 0usize;
                let mut failures: Vec<String> = Vec::new();
                // Sequential, one id per call (design.md: the daemon has no
                // workspace filter) — a workspace holds at most a handful of
                // sessions, and one failure must never stop the rest.
                for id in &ids {
                    match client.restart_session(id, false) {
                        Ok(response) => {
                            restarted += response.restarted.len();
                            skipped += response.skipped.len();
                            failures.extend(response.failed.iter().map(|f| f.error.clone()));
                        }
                        Err(e) => failures.push(failure_notice("restart failed", &e)),
                    }
                }
                send_notice(&tx, restart_all_notice(restarted, skipped, &failures), 5000);
                fetch_and_send(&client, &tx);
            }
            Effect::RestartDaemon => match client.restart_daemon() {
                // The successor's pid is the daemon's own bookkeeping; the
                // wall's side of the handoff is already told by the SSE
                // task's `AppEvent::Connection` transitions (the pet boxes
                // on the drop, unboxes on the reconnect), so a success adds
                // no notice of its own.
                Ok(_pid) => {}
                Err(e) => send_notice(&tx, failure_notice("daemon restart failed", &e), 5000),
            },
            Effect::Quit => unreachable!("Quit is handled by the state loop"),
        }
    });
}

// ── Runtime assembly ─────────────────────────────────────────────────────

/// Build the runtime, spawn the producer tasks, and run the state loop until
/// quit. The terminal has already been entered by the caller (main).
pub fn run(terminal: &mut crate::term::WallTerminal, base_url: &str) -> std::io::Result<()> {
    let rt = tokio::runtime::Runtime::new()?;
    let stop = Arc::new(AtomicBool::new(false));
    let (tx, rx) = mpsc::unbounded_channel::<AppEvent>();
    let client_id = format!("garage-wall-{}", std::process::id());

    spawn_input_task(&rt, tx.clone(), stop.clone());
    spawn_fetch_task(&rt, tx.clone(), base_url.to_owned(), client_id.clone());
    spawn_sse_task(&rt, tx.clone(), base_url.to_owned(), stop.clone());
    spawn_usage_task(&rt, tx.clone(), base_url.to_owned(), stop.clone());
    spawn_signal_task(&rt, tx.clone());

    let result = rt.block_on(state_loop(terminal, tx.clone(), rx, base_url, &client_id));
    stop.store(true, Ordering::Relaxed);
    // Report invisible on the way out (the Dart `_quit` heartbeat stop) —
    // bounded by the shutdown timeout below, never blocking the exit.
    {
        let base_url = base_url.to_owned();
        rt.spawn_blocking(move || {
            let _ = GarageClient::new(Some(base_url)).post_visibility(&client_id, false);
        });
    }
    // Don't wait for blocked producer tasks (the SSE read can sit inside its
    // read timeout); everything is stop-flagged and owns no cleanup.
    rt.shutdown_timeout(Duration::from_millis(500));
    result
}

/// Terminal input → channel. Poll-based so the `stop` flag is honored
/// promptly at shutdown.
fn spawn_input_task(rt: &tokio::runtime::Runtime, tx: EventSender, stop: Arc<AtomicBool>) {
    rt.spawn_blocking(move || {
        while !stop.load(Ordering::Relaxed) {
            match crossterm::event::poll(Duration::from_millis(100)) {
                Ok(true) => match crossterm::event::read() {
                    Ok(ev) => {
                        if tx.send(AppEvent::Term(ev)).is_err() {
                            break;
                        }
                    }
                    Err(_) => break,
                },
                Ok(false) => {}
                Err(_) => break,
            }
        }
    });
}

/// Initial workspaces + sessions fetch, plus the visible=true report (the
/// full visibility heartbeat is wave 4's escalation module).
fn spawn_fetch_task(
    rt: &tokio::runtime::Runtime,
    tx: EventSender,
    base_url: String,
    client_id: String,
) {
    rt.spawn_blocking(move || {
        let client = GarageClient::new(Some(base_url));
        let _ = client.post_visibility(&client_id, true);
        match client.fetch_workspaces() {
            Ok(workspaces) => {
                let _ = tx.send(AppEvent::Workspaces(workspaces));
            }
            Err(e) => send_notice(&tx, format!("workspaces fetch failed: {e}"), 5000),
        }
        match client.fetch_sessions() {
            Ok(sessions) => {
                let _ = tx.send(AppEvent::Sessions(sessions));
            }
            Err(e) => send_notice(&tx, format!("sessions fetch failed: {e}"), 5000),
        }
    });
}

/// `GET /api/usage`: once at startup, then every [`USAGE_POLL_INTERVAL`]
/// (spec tui-context-meters "Strip usage chip") — no SSE push exists for
/// account-wide usage, so polling is the only source. Stops promptly on
/// `stop` rather than sleeping through the full interval at shutdown.
fn spawn_usage_task(
    rt: &tokio::runtime::Runtime,
    tx: EventSender,
    base_url: String,
    stop: Arc<AtomicBool>,
) {
    rt.spawn(async move {
        while !stop.load(Ordering::Relaxed) {
            let base_url = base_url.clone();
            let tx = tx.clone();
            let _ = tokio::task::spawn_blocking(move || {
                let client = GarageClient::new(Some(base_url));
                if let Ok(usage) = client.fetch_usage() {
                    let _ = tx.send(AppEvent::Usage(usage));
                }
            })
            .await;
            tokio::time::sleep(USAGE_POLL_INTERVAL).await;
        }
    });
}

/// SSE subscription task: `status` events flow straight into the channel; a
/// `sessions` ping (and the poll fallback while disconnected) triggers a
/// refetch whose result flows into the same channel.
fn spawn_sse_task(
    rt: &tokio::runtime::Runtime,
    tx: EventSender,
    base_url: String,
    stop: Arc<AtomicBool>,
) {
    rt.spawn_blocking(move || {
        let client = GarageClient::new(Some(base_url.clone()));
        let mut sse = SseClient::new(base_url);
        let refetch = {
            let tx = tx.clone();
            move |client: &GarageClient| {
                if let Ok(sessions) = client.fetch_sessions() {
                    let _ = tx.send(AppEvent::Sessions(sessions));
                }
            }
        };
        let on_event = {
            let tx = tx.clone();
            let client = &client;
            let refetch = refetch.clone();
            move |event: crate::api::sse::SseEvent| match event.event.as_str() {
                "status" => {
                    if let Ok(v) = serde_json::from_str::<Value>(&event.data) {
                        if let Some(id) = v.get("id").and_then(Value::as_str) {
                            let status = v
                                .get("status")
                                .and_then(Value::as_str)
                                .unwrap_or("idle")
                                .to_owned();
                            let needs_input = status == "needs-input";
                            let _ = tx.send(AppEvent::Status {
                                id: id.to_owned(),
                                status,
                                since: v.get("since").and_then(Value::as_i64),
                            });
                            // The status event carries no notification text;
                            // a needs-input transition's `message` only
                            // exists on the listing, so pull it — the triage
                            // queue must show the daemon's question (spec
                            // tui-triage "Question text shown").
                            if needs_input {
                                refetch(client);
                            }
                        }
                    }
                }
                "sessions" => refetch(client),
                _ => {}
            }
        };
        let on_poll = {
            let client = &client;
            move || refetch(client)
        };
        // `AppEvent::Connection` transitions only (spec tui-pit-pet: the
        // pet boxes on an observed drop, not on every internal reconnect
        // tick) — `last_live` collapses `SseReconnectMachine`'s repeated
        // connect/disconnect calls (one per attempt/poll cycle) down to an
        // edge so the state loop isn't spammed with redundant events.
        let last_live = std::cell::Cell::new(None::<bool>);
        let on_connection = move |live: bool| {
            if last_live.get() != Some(live) {
                last_live.set(Some(live));
                let _ = tx.send(AppEvent::Connection(live));
            }
        };
        sse.run(stop, on_event, on_poll, on_connection);
    });
}

/// SIGTERM → orderly quit (emergency kill contract: the terminal is restored
/// and tiles detach before masters close).
fn spawn_signal_task(rt: &tokio::runtime::Runtime, tx: EventSender) {
    rt.spawn(async move {
        if let Ok(mut sigterm) =
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
        {
            sigterm.recv().await;
            let _ = tx.send(AppEvent::Quit);
        }
    });
}

/// Everything the state loop owns.
struct App {
    store: WallStore,
    router: GarageRouter,
    registry: TileRegistry,
    escalation: EscalationPolicy,
    keylog: KeyLog,
    tx: EventSender,
    base_url: String,
    /// Selection index into the triage queue rows; reset on every open.
    queue_selection: usize,
    /// The `w` overlay's text field + inline error + busy guard.
    ws_form: WorkspaceAddForm,
    /// The `D` overlay's selection + "new group…" text-input sub-mode.
    view_picker: ViewPickerState,
    /// The worktree-finish overlay's queued records, busy flag, inline
    /// error and armed discard (p17).
    wt_finish: WorktreeFinishState,
    /// Rects of the last drawn frame — the mouse router reads the SAME
    /// geometry the paint used. `None` until the first frame.
    layout: Option<WallLayout>,
    /// Strip badge columns from the last frame (absolute), when rendered.
    badge_cols: Option<Range<u16>>,
    /// View-strip click targets from the last frame (absolute columns paired
    /// with the view name they focus) — empty when the strip isn't shown.
    view_strip_targets: Vec<(Range<u16>, String)>,
    /// Modal rects from the last frame, per open overlay.
    triage_rect: Option<Rect>,
    ws_add_rect: Option<Rect>,
    view_picker_rect: Option<Rect>,

    // ── p15: pit pet (spec tui-pit-pet) ───────────────────────────────────
    /// Motion state; reset to `default()` whenever `P` changes the species
    /// (a fresh pet starts centered, not wherever the last one wandered).
    pet_sim: pet::PetSim,
    /// Chatter's own rate-limit/dedup state — never reset by `P` (the spec
    /// doesn't ask for it, and it'd only let a species-swap dodge the
    /// floor).
    pet_chatter: pet::Chatter,
    /// Next epoch ms the 300 ms tick is due; `0` so a freshly-enabled pet
    /// renders on the very next loop pass instead of waiting out a stale
    /// deadline from before it was off.
    pet_next_tick_ms: i64,
    /// This frame's sprite, built by [`App::pet_tick`] — `None` whenever the
    /// pet is off (design.md decision 3: "off → the branch is one `if`").
    pet_render: Option<PetRender>,
    /// `GARAGE_PET_ASCII=1`, read once at construction (spec risk note: the
    /// pup's face glyphs have an ASCII fallback for fonts that mangle them).
    pet_ascii: bool,
    /// Epoch ms the wall started — `ChatterCtx::uptime_ms`'s zero point.
    started_ms: i64,
    /// The effective mood from the last tick — used only to detect a fresh
    /// transition INTO `Celebrate` (`ChatterCtx::just_celebrated`); the
    /// authoritative mood for click routing.
    pet_mood: pet::Mood,
    /// `daemon_live` as of the last tick — used only to detect a box → live
    /// edge (`ChatterCtx::just_back_live`).
    prev_daemon_live: bool,
    /// Strip-absolute pet columns from the last frame (like `badge_cols`),
    /// when the pet rendered.
    pet_cols: Option<Range<u16>>,
    /// The filler width the last-drawn strip had — the room `pet_tick`'s
    /// next `PetSim::step` has to work with. `0` until the first frame
    /// (falls back to a conservative default — see `pet_tick`).
    last_strip_filler: u16,
    /// xorshift64 state feeding every `rng: &mut impl FnMut() -> f64` the
    /// pet module needs (design.md decision 2: time/randomness injected).
    pet_rng: u64,
    /// `(minute-bucket, hour)` — `ChatterCtx::local_hour` is shelled out to
    /// `date +%H` (no time crate in this workspace — see Cargo.toml) at most
    /// once a minute, not once per 300 ms tick.
    local_hour_cache: Option<(i64, u8)>,
}

impl App {
    /// Apply one event; true = quit.
    fn apply(&mut self, ev: AppEvent) -> bool {
        match ev {
            AppEvent::Quit => return true,
            AppEvent::TileOutput => {}
            AppEvent::Workspaces(w) => self.store.workspaces_fetched(w),
            AppEvent::Sessions(s) => self.store.sessions_fetched(s),
            AppEvent::Usage(u) => self.store.usage_fetched(u),
            AppEvent::Status { id, status, since } => {
                self.store.status_changed(&id, &status, since);
            }
            AppEvent::Connection(live) => self.store.connection(live),
            AppEvent::Notice { text, ttl_ms } => {
                self.router.notices.show(text, ttl_ms, now_ms());
            }
            AppEvent::SpawnSettled => self.router.spawn_settled(),
            AppEvent::RestoreSettled(id) => self.router.restore_settled(&id),
            AppEvent::InstallSettled { statusline, hooks } => {
                self.router.install_settled();
                let failed = statusline.is_err() || hooks.is_err();
                self.router.notices.show(
                    install_notice(&statusline, &hooks),
                    if failed { 8000 } else { 5000 },
                    now_ms(),
                );
            }
            AppEvent::WorktreeClosed(record) => {
                if show_worktree_finish(&mut self.store, &mut self.wt_finish, record) {
                    self.ws_form.reset();
                    self.view_picker.reset();
                }
            }
            AppEvent::WorktreeFinishSettled { record, action, error } => {
                let notice = worktree_finish_settled(
                    &mut self.store,
                    &mut self.wt_finish,
                    &record,
                    action,
                    error,
                );
                if let Some(text) = notice {
                    self.router.notices.show(text, 5000, now_ms());
                }
            }
            AppEvent::WorkspaceAddSettled { name, error } => {
                let failed = error.is_some();
                self.ws_form.settled(error);
                if !failed {
                    if self.store.state().overlay == Some(OverlayKind::WorkspaceAdd) {
                        self.store.close_overlay();
                    }
                    self.store.focus_workspace_named(&name);
                    self.router.notices.show(
                        format!("workspace {name} added — press n to spawn a session"),
                        5000,
                        now_ms(),
                    );
                }
            }
            AppEvent::PickDirectorySettled(outcome) => {
                // Closed meanwhile → `reset` already cleared `picking`, and
                // `pick_settled` ignores it; the overlay check covers any
                // other way the overlay went away.
                if self.store.state().overlay == Some(OverlayKind::WorkspaceAdd) {
                    self.ws_form.pick_settled(outcome);
                }
            }
            AppEvent::Term(Event::Resize(_, _)) => {} // layout recomputes per frame
            AppEvent::Term(Event::Paste(text)) => self.handle_paste(&text),
            AppEvent::Term(Event::Mouse(mouse)) => self.handle_mouse(&mouse),
            AppEvent::Term(Event::Key(key)) => {
                if key.kind != KeyEventKind::Release {
                    // Any keypress dismisses the install hint forever this
                    // run (spec tui-context-meters "Install affordance") —
                    // checked before ordinary key handling so it dismisses
                    // regardless of layer, even when the key goes on to do
                    // something else (e.g. `I` itself: the hint clears here,
                    // then the install action below shows its own notice).
                    if self.router.notices.current(now_ms()) == Some(crate::ui::strip::STATUSLINE_HINT) {
                        self.router.notices.clear_prefix(crate::ui::strip::STATUSLINE_HINT);
                    }
                    if self.handle_key(&key) {
                        return true;
                    }
                }
            }
            AppEvent::Term(_) => {}
        }
        false
    }

    /// Bracketed paste (crossterm `Event::Paste`), routed by [`paste_target`]:
    /// while engaged, forward to the focused tile wrapped in the paste guards
    /// (spec: "Paste forwarding"), snapping a frozen view live first like any
    /// PTY write; in the `w` overlay, into the path field (cleaned — see
    /// [`WorkspaceAddForm::insert_paste`]); everywhere else consumed —
    /// pasted text must never fire app commands.
    fn handle_paste(&mut self, text: &str) {
        let state = self.store.state();
        match paste_target(state.layer, state.overlay) {
            PasteTarget::Tile => {
                if let Some(id) = state.focused_session_id.clone() {
                    self.registry.set_frozen(&id, None);
                    self.registry.write(&id, &wrap_bracketed_paste(text));
                }
            }
            PasteTarget::WorkspaceField => self.ws_form.insert_paste(text),
            PasteTarget::Drop => {}
        }
    }

    /// The three-layer dispatch; true = quit.
    fn handle_key(&mut self, key: &KeyEvent) -> bool {
        match self.store.state().layer {
            KeyLayer::Overlay => {
                self.keylog
                    .log("overlay", &format!("{:?} {:?}", key.code, key.modifiers));
                if self.store.state().overlay == Some(OverlayKind::WorktreeFinish) {
                    let effects = handle_worktree_finish_key(
                        &mut self.store,
                        key,
                        &mut self.wt_finish,
                        now_ms(),
                    );
                    self.run_effects(effects);
                    return false;
                }
                let home = std::env::var("HOME").unwrap_or_default();
                let (effects, notice) = handle_overlay_key(
                    &mut self.store,
                    key,
                    &mut self.queue_selection,
                    &mut self.ws_form,
                    &mut self.view_picker,
                    &home,
                    &|path| std::path::Path::new(path).is_dir(),
                );
                if let Some(text) = notice {
                    self.router.notices.show(text, 2000, now_ms());
                }
                self.run_effects(effects);
                false
            }
            KeyLayer::Engaged => {
                self.keylog
                    .log("engaged", &format!("{:?} {:?}", key.code, key.modifiers));
                self.handle_engaged_key(key);
                false
            }
            KeyLayer::Garage => {
                self.keylog
                    .log("garage", &format!("{:?} {:?}", key.code, key.modifiers));
                let Some(key_str) = garage_key_string(key) else {
                    return false; // consumed: garage typing never reaches an agent
                };
                // Fresh overlay state on every open (the Dart `A`/`w`
                // branches — reset happens even if the dispatch declines).
                if key_str == "A" {
                    self.queue_selection = 0;
                }
                if key_str == "w" {
                    self.ws_form.reset();
                }
                if key_str == "D" {
                    self.view_picker.reset();
                }
                // `P` (spec tui-pit-pet): a fresh pet starts its motion sim
                // from scratch and renders on the very next tick, rather
                // than carrying over the outgoing (or newly-arriving)
                // species' position/frame counters. `GarageRouter` owns the
                // choice + persistence + notice; it doesn't own the sim, so
                // the reset happens here by comparing before/after.
                let pet_before = self.store.state().pet.clone();
                let effects = self
                    .router
                    .on_garage_key(&mut self.store, &key_str, now_ms());
                if self.store.state().pet != pet_before {
                    self.pet_sim = pet::PetSim::default();
                    self.pet_next_tick_ms = 0;
                }
                self.run_effects(effects)
            }
        }
    }

    /// Run router effects; true = quit requested.
    fn run_effects(&mut self, effects: Vec<Effect>) -> bool {
        let mut quit = false;
        for effect in effects {
            if matches!(effect, Effect::Quit) {
                quit = true;
            } else {
                run_effect(effect, &self.tx, &self.base_url);
            }
        }
        quit
    }

    // ── Engaged layer: scrollback + verbatim passthrough (task 4.5) ──────

    /// While engaged, every key is consumed here — Ctrl+G disengages
    /// (deliberately NOT snapping a frozen peek live — it may outlive
    /// engagement, same as the unengaged wheel peek), Shift+PageUp/Down
    /// drives the frozen capture-pane view, End while frozen snaps live,
    /// and everything else re-encodes into the PTY, snapping live first.
    fn handle_engaged_key(&mut self, key: &KeyEvent) {
        let Some(id) = self.store.state().focused_session_id.clone() else {
            return;
        };
        match engaged_input_for(key) {
            EngagedInput::Disengage => {
                self.store.disengage();
                return;
            }
            EngagedInput::Write(_) | EngagedInput::Nothing => {}
        }
        let shift = key.modifiers.contains(KeyModifiers::SHIFT);
        if shift && key.code == KeyCode::PageUp {
            let page = self.page_for(&id);
            self.scroll_tile(&id, page);
            return;
        }
        if shift && key.code == KeyCode::PageDown {
            let page = self.page_for(&id);
            self.scroll_tile(&id, -page);
            return;
        }
        // End while frozen: snap back to live, consumed (a live End still
        // reaches the app through encode_key below).
        if key.code == KeyCode::End && self.registry.frozen(&id).is_some() {
            self.registry.set_frozen(&id, None);
            return;
        }
        if let EngagedInput::Write(bytes) = engaged_input_for(key) {
            // Any key that reaches the PTY snaps the tile back to live first
            // (spec tui-scrollback "Typing snaps to live").
            self.registry.set_frozen(&id, None);
            self.registry.write(&id, &bytes);
        }
    }

    fn page_for(&self, id: &str) -> i64 {
        page_lines(self.registry.size_of(id).map_or(24, |(_, rows)| rows))
    }

    /// Scroll a tile by `lines` (+up / −down): freeze on the first up-scroll
    /// (capture-pane at the absolute `#{history_size}` anchor), page within
    /// the frozen view, return to live at the tail. tmux copy-mode is never
    /// entered — everything is capture-pane + local render.
    fn scroll_tile(&mut self, id: &str, lines: i64) {
        let (cols, rows) = self.registry.size_of(id).unwrap_or((80, 24));
        match self.registry.frozen(id).map(|f| f.model) {
            None if lines > 0 => {
                let Some(hist) = tmux_history_size(id) else {
                    return;
                };
                if let Some(model) = ScrollModel::freeze(hist, lines) {
                    self.recapture(id, model, cols, rows);
                }
            }
            None => {} // wheel-down while live is a no-op
            Some(model) if lines > 0 => {
                self.recapture(id, model.scroll_up(lines), cols, rows);
            }
            Some(model) => match model.scroll_down(-lines) {
                None => self.registry.set_frozen(id, None), // back to live
                Some(next) => self.recapture(id, next, cols, rows),
            },
        }
    }

    fn recapture(&mut self, id: &str, model: ScrollModel, cols: u16, rows: u16) {
        if let Some(parser) = capture_frozen(id, model.top_abs(), cols, rows) {
            let new_lines = tmux_history_size(id).map_or(0, |h| model.new_lines(h));
            self.registry.set_frozen(
                id,
                Some(FrozenTile {
                    model,
                    parser,
                    new_lines,
                    counted_at: Instant::now(),
                }),
            );
        }
    }

    // ── Mouse routing (task 4.4; spec tui-key-routing "or clicking a
    // tile", tui-triage "or clicking the strip badge", tui-scrollback
    // "Mouse wheel scrolling") ───────────────────────────────────────────

    fn handle_mouse(&mut self, mouse: &MouseEvent) {
        match mouse.kind {
            MouseEventKind::Down(MouseButton::Left) => {
                self.handle_click(mouse.column, mouse.row);
            }
            MouseEventKind::ScrollUp => self.handle_wheel(mouse.column, mouse.row, true),
            MouseEventKind::ScrollDown => self.handle_wheel(mouse.column, mouse.row, false),
            _ => {}
        }
    }

    /// The session id under a grid point, honoring maximize (a maximized
    /// tile covers the whole grid, so every grid click/wheel targets it).
    fn tile_at(&self, col: u16, row: u16) -> Option<(String, Rect)> {
        let layout = self.layout.as_ref()?;
        if !layout.grid.contains((col, row).into()) {
            return None;
        }
        let ids = &self.store.state().gridded_session_ids;
        let index = match layout.maximized {
            Some(i) => i,
            None => tile_index_at(
                i32::from(col - layout.grid.x),
                i32::from(row - layout.grid.y),
                ids.len(),
                i32::from(layout.grid.width),
                i32::from(layout.grid.height),
            )?,
        };
        Some((ids.get(index)?.clone(), *layout.tiles.get(index)?))
    }

    fn handle_click(&mut self, col: u16, row: u16) {
        let Some(layout) = self.layout.clone() else {
            return;
        };
        // Overlay layer: the whole screen is the overlay's (a barrier —
        // clicks never reach the wall underneath).
        if let Some(kind) = self.store.state().overlay {
            match kind {
                OverlayKind::Help => self.store.close_overlay(), // any click dismisses
                OverlayKind::TriageQueue => {
                    let inside = self
                        .triage_rect
                        .is_some_and(|r| r.contains((col, row).into()));
                    if !inside {
                        self.store.close_overlay(); // outside-modal click = Esc
                        return;
                    }
                    let local = i32::from(row - self.triage_rect.unwrap().y);
                    let ids: Vec<String> = triage_queue_rows(&self.store.state().sessions)
                        .iter()
                        .map(|s| s.id.clone())
                        .collect();
                    // A queue row click selects and jump-engages (Enter's
                    // landing); border/padding/footer clicks do nothing.
                    if let Some(i) = triage_row_index_at(local, ids.len()) {
                        self.queue_selection = i;
                        self.store.close_overlay();
                        self.store.jump_to_session(&ids[i]);
                    }
                }
                OverlayKind::WorkspaceAdd => {
                    let inside = self
                        .ws_add_rect
                        .is_some_and(|r| r.contains((col, row).into()));
                    if !inside {
                        self.ws_form.reset();
                        self.store.close_overlay();
                    }
                }
                OverlayKind::ViewPicker => self.handle_view_picker_click(col, row),
                // Modal: clicks anywhere are swallowed — an outside click
                // must not quietly "keep" a worktree the user never saw.
                OverlayKind::WorktreeFinish => {}
            }
            return;
        }
        // View-strip click: focus that view (spec tui-views "View strip and
        // group frame" — task 3.3 "view-strip click focuses that view").
        if layout.view_strip.height > 0 && row == layout.view_strip.y {
            let target = self
                .view_strip_targets
                .iter()
                .find(|(r, _)| r.contains(&col))
                .map(|(_, name)| name.clone());
            if let Some(name) = target {
                if self.store.state().layer == KeyLayer::Engaged {
                    self.store.disengage();
                }
                self.store.focus_view(&name);
            }
            return;
        }
        // Click on a URL in a tile's grid text opens it (spec
        // tui-key-routing "Click opens links") — our mouse capture starves
        // the outer terminal of clicks, so the wall must be the linkifier.
        // Checked before focus/engage: a click ON a link means "open this",
        // not "type here". Everything else falls through unchanged.
        if let Some((id, rect)) = self.tile_at(col, row) {
            let inner = Rect {
                x: rect.x + 1,
                y: rect.y + 1,
                width: rect.width.saturating_sub(2),
                height: rect.height.saturating_sub(2),
            };
            if inner.contains((col, row).into()) {
                if let Some(line) = self.registry.row_text(&id, row - inner.y) {
                    if let Some(url) = url_at(&line, usize::from(col - inner.x)) {
                        let _ = std::process::Command::new("open").arg(&url).spawn();
                        self.router.notices.show(format!("opened {url}"), 2000, now_ms());
                        return;
                    }
                }
            }
        }
        // Grid click: focus AND engage (same landing as focus+Enter); a
        // click on a restorable placeholder restores it; while engaged a
        // click on a different tile migrates via explicit disengage → focus
        // → engage (engagement never transfers silently).
        if let Some((id, _)) = self.tile_at(col, row) {
            let live = self
                .store
                .state()
                .session_by_id(Some(&id))
                .is_some_and(WallSession::live);
            let engaged = self.store.state().layer == KeyLayer::Engaged;
            if !live {
                if engaged {
                    self.store.disengage();
                }
                self.store.focus_session(&id);
                let effects = self.router.restore_clicked(id);
                self.run_effects(effects);
                return;
            }
            if engaged {
                if self.store.state().focused_session_id.as_deref() == Some(id.as_str()) {
                    return; // click on the engaged tile itself: nothing extra
                }
                self.store.disengage();
            }
            self.store.focus_session(&id);
            self.store.engage();
            return;
        }
        // Rail click: header focuses the workspace, session row focuses the
        // session — never engages; engagement drops back to garage first.
        if layout.rail.contains((col, row).into()) {
            let target = rail_target_at(
                &self.store.state().groups,
                i32::from(row - layout.rail.y),
            );
            if let Some(target) = target {
                if self.store.state().layer == KeyLayer::Engaged {
                    self.store.disengage();
                }
                match target {
                    RailTarget::Workspace(index) => self.store.focus_workspace(index),
                    RailTarget::Session(id) => self.store.focus_session(&id),
                }
            }
            return;
        }
        // Pet click (spec tui-pit-pet "Click routing"): checked BEFORE the
        // badge so an alert pet sitting near the badge never gets swallowed
        // by it.
        if row >= layout.strip.y && self.pet_cols.as_ref().is_some_and(|r| r.contains(&col)) {
            self.handle_pet_click(now_ms());
            return;
        }
        // Strip badge click: open the triage queue (the `A` binding).
        if row >= layout.strip.y && self.badge_cols.as_ref().is_some_and(|r| r.contains(&col)) {
            if self.store.state().layer == KeyLayer::Engaged {
                self.store.disengage();
            }
            self.queue_selection = 0;
            self.store.open_overlay(OverlayKind::TriageQueue);
        }
    }

    /// A click on the pet's strip columns (spec tui-pit-pet "Click
    /// routing"): while alert, the same jump `a` performs; otherwise a
    /// petting interaction (a short celebrate + a species-voiced reply).
    fn handle_pet_click(&mut self, now: i64) {
        if self.store.state().layer == KeyLayer::Engaged {
            self.store.disengage();
        }
        if self.pet_mood == pet::Mood::Alert {
            if !self.store.jump_to_longest_waiting() {
                // Same wording as the `a` key's decline (spec tui-triage).
                self.router.notices.show("no session needs you", 2000, now);
            }
            return;
        }
        let Some(species) = self
            .store
            .state()
            .pet
            .as_deref()
            .and_then(pet::Species::from_str)
        else {
            return; // race: `P` turned it off between paint and click
        };
        self.pet_sim.pet(now);
        // Petting is a celebrate the sim will report next tick; pre-set the
        // mood so it is not mistaken for a blocked→clear transition (which
        // would trigger a Proud line over the petting reply).
        self.pet_mood = pet::Mood::Celebrate;
        let mut rng_state = self.pet_rng;
        let mut rng = || xorshift_next(&mut rng_state);
        let line = pet::petting_line(species, &mut rng);
        self.pet_rng = rng_state;
        if let Some(line) = line {
            self.router.notices.show_pet(line, 2500, now);
        }
    }

    // ── p15: pit pet tick (spec tui-pit-pet) ──────────────────────────────

    /// `ChatterCtx::local_hour`, shelled out to `date +%H` at most once a
    /// minute (no time crate in this workspace's `Cargo.toml`; task 4.3
    /// explicitly calls for the `date` fallback, cached, "not per tick").
    /// Falls back to noon (never late-night, never usage-adjacent) if the
    /// shell-out ever fails — chatter staying silent on a broken clock beats
    /// it misfiring the late-night lines all day.
    fn local_hour(&mut self, now: i64) -> u8 {
        let minute_bucket = now.div_euclid(60_000);
        if let Some((bucket, hour)) = self.local_hour_cache {
            if bucket == minute_bucket {
                return hour;
            }
        }
        let hour = std::process::Command::new("date")
            .arg("+%H")
            .output()
            .ok()
            .filter(|o| o.status.success())
            .and_then(|o| String::from_utf8(o.stdout).ok())
            .and_then(|s| s.trim().parse::<u8>().ok())
            .unwrap_or(12);
        self.local_hour_cache = Some((minute_bucket, hour));
        hour
    }

    /// The 300 ms pet tick (spec tui-pit-pet "Species-true one-row motion":
    /// "Motion SHALL run on a 300 ms tick while the pet is on"). Off → one
    /// early-return `if` and nothing else runs (design.md decision 3). On →
    /// steps the sim, updates `pet_render`, and runs the chatter scheduler
    /// in the same beat (task 4.3). Returns whether the caller should mark
    /// the frame dirty.
    fn pet_tick(&mut self, now: i64) -> bool {
        let Some(species) = self
            .store
            .state()
            .pet
            .as_deref()
            .and_then(pet::Species::from_str)
        else {
            // Off: nothing to render, nothing to say — cheap by construction.
            return self.pet_render.take().is_some();
        };
        if now < self.pet_next_tick_ms {
            return false;
        }
        self.pet_next_tick_ms = now + 300;

        let (daemon_live, blocked, any_working, session_count, usage_pct_max) = {
            let state = self.store.state();
            let usage_pct_max = [state.usage.five_hour.as_ref(), state.usage.seven_day.as_ref()]
                .into_iter()
                .flatten()
                .map(|w| w.used_percentage.min(u32::from(u8::MAX)) as u8)
                .max();
            (
                state.daemon_live,
                state.blocked_count(),
                state.sessions.iter().any(|s| s.status == "working"),
                state.sessions.len(),
                usage_pct_max,
            )
        };
        let mood_in = pet::mood(daemon_live, blocked > 0, any_working, session_count);
        // `last_strip_filler` is `0` before the first frame is ever drawn —
        // a conservative fallback keeps the very first tick from computing
        // against a zero-width filler and hiding the pet before it's ever
        // been seen.
        // Minus the strip's two gutter columns (see `strip_line`), so the sim's
        // clamp and the paint's clamp agree.
        let filler_width = if self.last_strip_filler > 0 { self.last_strip_filler.saturating_sub(2) } else { 20 };

        let mut rng_state = self.pet_rng;
        let mut rng = || xorshift_next(&mut rng_state);
        let result = self.pet_sim.step(
            species,
            mood_in,
            blocked,
            session_count,
            filler_width,
            now,
            self.pet_ascii,
            &mut rng,
        );
        self.pet_rng = rng_state;

        let just_celebrated = self.pet_mood != pet::Mood::Celebrate && result.mood == pet::Mood::Celebrate;
        let just_back_live = !self.prev_daemon_live && daemon_live;
        self.pet_mood = result.mood;
        self.prev_daemon_live = daemon_live;

        let was_none = self.pet_render.is_none();
        self.pet_render = Some(PetRender {
            text: result.sprite.text,
            bang: result.sprite.bang,
            bang_lit: result.bang_lit,
            x: result.x,
            dim: matches!(result.mood, pet::Mood::Sleep | pet::Mood::Box),
            say: None, // filled at draw time from the live pet notice
        });
        let mut dirty = result.changed || was_none;

        if result.mood == pet::Mood::Alert && self.router.notices.clear_pet() {
            dirty = true;
        }
        let ctx = pet::ChatterCtx {
            now_ms: now,
            uptime_ms: now - self.started_ms,
            local_hour: self.local_hour(now),
            usage_pct_max,
            mood: result.mood,
            just_celebrated,
            just_back_live,
            alert: result.mood == pet::Mood::Alert,
            user_notice_active: self.router.notices.user_notice_active(now),
        };
        let mut rng_state = self.pet_rng;
        let mut rng = || xorshift_next(&mut rng_state);
        let line = self.pet_chatter.next_line(species, &ctx, &mut rng);
        self.pet_rng = rng_state;
        if let Some(line) = line {
            if self.router.notices.show_pet(line, 4000, now) {
                dirty = true;
            }
        }

        dirty
    }

    /// A click inside the `D` view-picker overlay (task 3.3 "picker overlay
    /// clicks"): outside the modal is Esc; inside, while the "new group…"
    /// text field owns the keys, clicks do nothing (same as
    /// `workspace_add`'s field); otherwise a row click selects — the
    /// trailing "new group…" row switches to that sub-mode, any other row
    /// moves the focused session there and shows the "moved" notice.
    fn handle_view_picker_click(&mut self, col: u16, row: u16) {
        let Some(rect) = self.view_picker_rect else { return };
        if !rect.contains((col, row).into()) {
            self.view_picker.reset();
            self.store.close_overlay();
            return;
        }
        if self.view_picker.new_group_input.is_some() {
            return;
        }
        let entries = view_picker_entries(&self.store);
        let entries_len = entries.len();
        let local = i32::from(row - rect.y);
        let Some(i) = view_picker_row_index_at(local, entries_len) else {
            return;
        };
        if i == entries_len - 1 {
            self.view_picker.selected = i;
            self.view_picker.new_group_input = Some(String::new());
            return;
        }
        let Some(name) = entries.get(i).cloned() else {
            return;
        };
        let label = self
            .store
            .state()
            .session_by_id(self.store.state().focused_session_id.as_deref())
            .map(|s| s.label.clone());
        self.store.close_overlay();
        if self.store.move_focused_to_view(&name) {
            if let Some(label) = label {
                self.router.notices.show(format!("moved {label} → {name}"), 2000, now_ms());
            }
        }
        self.view_picker.reset();
    }

    /// Wheel over a tile's terminal area. A frozen view owns the wheel
    /// regardless of engagement; an engaged LIVE tile forwards the SGR wheel
    /// sequence with tile-local 1-based coords (Claude Code enables mouse
    /// reporting and scrolls its own alternate screen); an unengaged live
    /// tile opens the frozen peek on wheel-up WITHOUT engaging.
    fn handle_wheel(&mut self, col: u16, row: u16, up: bool) {
        if self.store.state().overlay.is_some() {
            return;
        }
        let Some((id, rect)) = self.tile_at(col, row) else {
            return;
        };
        // Tile-local coords inside the border; border wheel does nothing
        // (the Dart WheelRegion wrapped only the terminal child).
        let inner = Rect {
            x: rect.x + 1,
            y: rect.y + 1,
            width: rect.width.saturating_sub(2),
            height: rect.height.saturating_sub(2),
        };
        if !inner.contains((col, row).into()) {
            return;
        }
        if self.registry.frozen(&id).is_some() {
            self.scroll_tile(&id, if up { WHEEL_LINES } else { -WHEEL_LINES });
            return;
        }
        let live = self
            .store
            .state()
            .session_by_id(Some(&id))
            .is_some_and(WallSession::live);
        let engaged_here = self.store.state().layer == KeyLayer::Engaged
            && self.store.state().focused_session_id.as_deref() == Some(id.as_str());
        if engaged_here && live {
            let seq = format!(
                "\x1b[<{};{};{}M",
                if up { 64 } else { 65 },
                col - inner.x + 1,
                row - inner.y + 1
            );
            self.registry.write(&id, seq.as_bytes());
            return;
        }
        if live && up {
            self.scroll_tile(&id, WHEEL_LINES); // frozen peek, layer untouched
        }
    }

    // ── Downstream of state changes ──────────────────────────────────────

    /// Runs after every drained event batch (and on ticks): reconcile the
    /// PTY registry against the gridded live set, drop engagement whose
    /// tile is dead (keys must never black-hole), and emit escalation
    /// (bell/OSC title) for the new snapshot.
    fn after_events(&mut self) -> Option<String> {
        let live_gridded: Vec<String> = {
            let state = self.store.state();
            state
                .gridded_session_ids
                .iter()
                .filter(|id| state.session_by_id(Some(id)).is_some_and(WallSession::live))
                .cloned()
                .collect()
        };
        self.registry.sync(&live_gridded);
        self.registry.tick(Instant::now());
        if self.store.state().layer == KeyLayer::Engaged {
            let dead = self
                .store
                .state()
                .focused_session_id
                .as_deref()
                .is_some_and(|id| self.registry.dead_reason(id).is_some());
            if dead {
                self.store.disengage();
            }
        }
        self.escalation.update(&self.store.state().sessions)
    }

    /// Refresh the frozen `+N lines` counters (throttled tmux calls).
    /// Returns true when any visible counter changed.
    fn refresh_frozen_counts(&mut self) -> bool {
        self.registry.refresh_frozen_counts(FROZEN_COUNT_REFRESH)
    }

    // ── Draw (task 4.1–4.5: the real wall) ───────────────────────────────

    fn draw_frame(&mut self, frame: &mut ratatui::Frame, now_ms: i64) {
        let area = frame.area();
        let (ids, maximized_index) = {
            let state = self.store.state();
            let ids = state.gridded_session_ids.clone();
            let maximized = state
                .maximized_session_id
                .as_deref()
                .and_then(|m| ids.iter().position(|id| id == m));
            (ids, maximized)
        };
        // Views (spec tui-views "View strip and group frame"): the strip
        // shows only at 2+ views; the frame only around a 2+-session
        // focused view — both precomputed here so `wall_layout` stays pure
        // geometry with no state dependency of its own.
        let focused_workspace = self.store.state().focused_workspace.clone();
        let views: Vec<ViewSummary> = focused_workspace
            .as_deref()
            .map(|w| self.store.state().views_for(w))
            .unwrap_or_default();
        let focused_view_name = focused_workspace
            .as_deref()
            .map(|w| self.store.state().focused_view_name(w).to_owned())
            .unwrap_or_default();
        let show_view_strip = views.len() >= 2;
        let framed = views
            .iter()
            .find(|v| v.name == focused_view_name)
            .is_some_and(|v| v.session_ids.len() >= 2);

        let layout = wall_layout(area, ids.len(), maximized_index, show_view_strip, framed);

        // PTY size tracks the tile (TIOCSWINSZ) on every layout pass —
        // startup, grid reshape, maximize both ways, terminal resize.
        for (i, id) in ids.iter().enumerate() {
            let (cols, rows) = tile_inner(layout.tiles[i]);
            self.registry.apply_size(id, cols, rows);
        }

        let buf = frame.buffer_mut();
        render_rail(buf, layout.rail, self.store.state(), now_ms);

        if show_view_strip {
            let ranges = render_view_strip(buf, layout.view_strip, &views, &focused_view_name);
            self.view_strip_targets =
                views.iter().zip(ranges).map(|(v, r)| (r, v.name.clone())).collect();
        } else {
            self.view_strip_targets.clear();
        }
        if let Some(frame_rect) = layout.group_frame {
            Block::bordered()
                .border_type(BorderType::Rounded)
                .border_style(Style::default().fg(colors::FAINT))
                .render(frame_rect, buf);
        }

        if self.store.state().groups.is_empty() {
            // Zero workspaces: onboarding panel (never amber).
            render_centered_lines(
                buf,
                layout.grid,
                vec![
                    Line::styled(
                        "no workspaces yet — press w to add one",
                        Style::default().fg(colors::FG),
                    ),
                    Line::styled(
                        "every Claude session in it appears here live",
                        Style::default().fg(colors::FAINT),
                    ),
                ],
            );
        } else if ids.is_empty() {
            render_centered_lines(
                buf,
                layout.grid,
                vec![
                    Line::styled(
                        "no sessions in this workspace",
                        Style::default().fg(colors::FG),
                    ),
                    Line::styled(
                        "press n for a session, N for a worktree session",
                        Style::default().fg(colors::FAINT),
                    ),
                ],
            );
        } else {
            for (i, id) in ids.iter().enumerate() {
                // A maximized tile covers the whole grid: siblings stay
                // mounted (parsers keep reading, PTYs stay sized) but are
                // not painted and receive no input.
                if maximized_index.is_some() && maximized_index != Some(i) {
                    continue;
                }
                self.draw_tile(buf, layout.tiles[i], id, now_ms);
            }
        }

        let strip_cols = render_strip(
            buf,
            layout.strip,
            self.store.state(),
            self.router.notices.current_of(NoticeKind::User, now_ms),
            self.pet_render.clone().map(|mut p| {
                p.say = self
                    .router
                    .notices
                    .current_of(NoticeKind::Pet, now_ms)
                    .map(str::to_owned);
                p
            }),
        );
        self.badge_cols = strip_cols.badge;
        self.pet_cols = strip_cols.pet;
        self.last_strip_filler = strip_cols.filler_width;

        self.triage_rect = None;
        self.ws_add_rect = None;
        self.view_picker_rect = None;
        match self.store.state().overlay {
            Some(OverlayKind::Help) => render_help(buf, area),
            Some(OverlayKind::TriageQueue) => {
                let rows = triage_queue_rows(&self.store.state().sessions);
                self.triage_rect =
                    Some(render_triage(buf, area, &rows, self.queue_selection, now_ms));
            }
            Some(OverlayKind::WorkspaceAdd) => {
                self.ws_add_rect = Some(render_workspace_add(buf, area, &self.ws_form));
            }
            Some(OverlayKind::ViewPicker) => {
                let view_names: Vec<String> = views.iter().map(|v| v.name.clone()).collect();
                let entries = picker_entries(&view_names, &focused_view_name);
                self.view_picker_rect = Some(render_view_picker(
                    buf,
                    area,
                    &entries,
                    self.view_picker.selected,
                    self.view_picker.new_group_input.as_deref(),
                ));
            }
            Some(OverlayKind::WorktreeFinish) => {
                render_worktree_finish(buf, area, &self.wt_finish, now_ms);
            }
            None => {}
        }
        self.layout = Some(layout);
    }

    fn draw_tile(&self, buf: &mut ratatui::buffer::Buffer, rect: Rect, id: &str, now_ms: i64) {
        let state = self.store.state();
        let Some(session) = state.session_by_id(Some(id)) else {
            return; // refetch race; the next frame fixes it
        };
        let focused = state.focused_session_id.as_deref() == Some(id);
        let frozen = self.registry.frozen(id);
        let view = TileView {
            session,
            focused,
            engaged: focused && state.layer == KeyLayer::Engaged,
            restoring: self.router.restoring().contains(id),
            dead_reason: self.registry.dead_reason(id),
            frozen_new_lines: frozen.map(|f| f.new_lines),
            now_ms,
        };
        if let Some(frozen) = frozen {
            render_tile(buf, rect, &view, Some(frozen.parser.screen()));
        } else if let Some(client) = self.registry.client(id) {
            let parser = client.parser().lock().unwrap();
            render_tile(buf, rect, &view, Some(parser.screen()));
        } else {
            render_tile(buf, rect, &view, None);
        }
    }
}

/// Snapshot the two fields `wall.json` persists (spec tui-views "View
/// persistence" + tui-pit-pet "Opt-in roster cycled by `P`") into a
/// [`WallFile`] ready for [`persistence::save`]. `daemon_live` and every
/// other transient field never reach disk.
fn wall_file_snapshot(store: &WallStore) -> WallFile {
    WallFile { views: store.state().views.clone(), pet: store.state().pet.clone() }
}

/// Whether the one-time statusline-install hint should appear now (spec
/// tui-context-meters "Install affordance"): not shown yet this run, the
/// startup delay has elapsed, and no session carries statusline-sourced
/// context. Pure — split out from the loop below so the gating logic
/// unit-tests without real timers.
fn should_show_install_hint(
    already_shown: bool,
    elapsed_since_startup: Duration,
    sessions: &[WallSession],
) -> bool {
    !already_shown
        && elapsed_since_startup >= INSTALL_HINT_DELAY
        && !sessions.iter().any(WallSession::has_statusline_context)
}

/// The single state-owning loop: drains the channel, applies events, runs
/// the downstream reconciliation (registry / escalation / heartbeat), and
/// draws when dirty, frame-capped like the spike.
async fn state_loop(
    terminal: &mut crate::term::WallTerminal,
    tx: EventSender,
    mut rx: mpsc::UnboundedReceiver<AppEvent>,
    base_url: &str,
    client_id: &str,
) -> std::io::Result<()> {
    let started_ms = now_ms();
    // `GARAGE_PET_CHATTER_MS` (task 4.3, E2E-only): lowers `Chatter`'s
    // rate-limit floor so a harness doesn't have to wait 3 real minutes for
    // a line. Absent in every normal run — `Chatter::default()`'s floor is
    // the spec's `CHATTER_FLOOR_MS`.
    let pet_chatter = std::env::var("GARAGE_PET_CHATTER_MS")
        .ok()
        .and_then(|s| s.parse::<i64>().ok())
        .map(pet::Chatter::with_floor_ms)
        .unwrap_or_default();
    let mut app = App {
        store: WallStore::new(),
        router: GarageRouter::default(),
        registry: TileRegistry::new(tx.clone()),
        escalation: EscalationPolicy::default(),
        keylog: KeyLog::new(),
        tx,
        base_url: base_url.to_owned(),
        queue_selection: 0,
        ws_form: WorkspaceAddForm::default(),
        view_picker: ViewPickerState::default(),
        wt_finish: WorktreeFinishState::default(),
        layout: None,
        badge_cols: None,
        view_strip_targets: Vec::new(),
        triage_rect: None,
        ws_add_rect: None,
        view_picker_rect: None,
        pet_sim: pet::PetSim::default(),
        pet_chatter,
        pet_next_tick_ms: 0,
        pet_render: None,
        pet_ascii: std::env::var("GARAGE_PET_ASCII").ok().as_deref() == Some("1"),
        started_ms,
        pet_mood: pet::Mood::Sleep,
        prev_daemon_live: true, // matches WallState::initial()'s daemon_live default
        pet_cols: None,
        last_strip_filler: 0,
        pet_rng: (started_ms as u64) | 1, // xorshift64 needs a non-zero seed
        local_hour_cache: None,
    };
    // View persistence (spec tui-views "View persistence"): load whatever
    // wall.json holds, then debounce a save (≥500ms after the last
    // views_revision bump) so a burst of `d`/`D`/Tab presses writes once,
    // not once per keystroke; a final synchronous save on every exit path
    // below never leaves a just-made change unpersisted.
    //
    // The load itself is deferred to right after the FIRST `Sessions` fetch
    // lands (not issued eagerly here) — `load_views` prunes its assignments
    // against `state().sessions`, which is empty on a brand-new `WallStore`,
    // so calling it before any session exists would prune every loaded
    // assignment on the spot and silently lose the whole file.
    let views_path = persistence::wall_json_path();
    let mut pending_file = Some(persistence::load(&views_path));
    let mut saved_views_revision = app.store.views_revision();
    let mut views_dirty_since: Option<Instant> = None;
    const VIEWS_SAVE_DEBOUNCE: Duration = Duration::from_millis(500);

    let mut dirty = true;
    let mut last_render = Instant::now() - FRAME_CAP;
    let mut last_elapsed_tick = Instant::now();
    let mut last_heartbeat = Instant::now();
    let startup = Instant::now();
    let mut install_hint_shown = false;

    loop {
        let now = now_ms();
        if app.router.notices.tick(now) {
            dirty = true;
        }
        // Elapsed timers / done-fade advance without input.
        if last_elapsed_tick.elapsed() >= ELAPSED_TICK {
            last_elapsed_tick = Instant::now();
            dirty = true;
        }
        // Visibility heartbeat (spec tui-triage: daemon-side notifications
        // stay suppressed while the TUI runs).
        if last_heartbeat.elapsed() >= Duration::from_millis(HEARTBEAT_INTERVAL_MS) {
            last_heartbeat = Instant::now();
            let base_url = base_url.to_owned();
            let client_id = client_id.to_owned();
            tokio::task::spawn_blocking(move || {
                let _ = GarageClient::new(Some(base_url)).post_visibility(&client_id, true);
            });
        }
        // One-time statusline-install hint (spec tui-context-meters
        // "Install affordance"): dim, never repeats after it's shown once —
        // dismissal on any keypress is handled in `App::apply`.
        if should_show_install_hint(install_hint_shown, startup.elapsed(), &app.store.state().sessions)
        {
            install_hint_shown = true;
            app.router.notices.show(crate::ui::strip::STATUSLINE_HINT, INSTALL_HINT_TTL_MS, now);
            dirty = true;
        }
        // Reattach retries / staggered attaches fire from the tick too, so
        // they never wait on an input event.
        if app.registry.tick(Instant::now()) {
            dirty = true;
        }
        if app.refresh_frozen_counts() {
            dirty = true;
        }
        // Pit pet (spec tui-pit-pet): a 300 ms tick while a species is
        // selected, one cheap `if` otherwise (design.md decision 3).
        if app.pet_tick(now) {
            dirty = true;
        }
        // Debounced wall.json save (spec tui-views "View persistence"):
        // start the timer the moment views_revision moves, fire once it's
        // sat still for VIEWS_SAVE_DEBOUNCE — a burst of view mutations
        // resets the timer on every bump rather than writing on each one.
        if app.store.views_revision() != saved_views_revision {
            views_dirty_since.get_or_insert_with(Instant::now);
        }
        if let Some(since) = views_dirty_since {
            if since.elapsed() >= VIEWS_SAVE_DEBOUNCE {
                saved_views_revision = app.store.views_revision();
                views_dirty_since = None;
                let file = wall_file_snapshot(&app.store);
                let path = views_path.clone();
                tokio::task::spawn_blocking(move || {
                    let _ = persistence::save(&path, &file);
                });
            }
        }
        if dirty && last_render.elapsed() >= FRAME_CAP {
            terminal.draw(|f| app.draw_frame(f, now_ms()))?;
            last_render = Instant::now();
            dirty = false;
        }
        let timeout = if dirty {
            FRAME_CAP.saturating_sub(last_render.elapsed())
        } else {
            Duration::from_millis(250)
        };
        let first = match tokio::time::timeout(timeout, rx.recv()).await {
            Err(_) => continue, // timeout: loop back to tick + render
            Ok(None) => {
                let _ = persistence::save(&views_path, &wall_file_snapshot(&app.store));
                return Ok(());
            }
            Ok(Some(ev)) => ev,
        };
        // Drain everything pending; handle in arrival order.
        let mut events = vec![first];
        while let Ok(ev) = rx.try_recv() {
            events.push(ev);
        }
        for ev in events {
            let was_sessions = matches!(ev, AppEvent::Sessions(_));
            dirty = true;
            if app.apply(ev) {
                let _ = persistence::save(&views_path, &wall_file_snapshot(&app.store));
                return Ok(());
            }
            // The first Sessions fetch has now populated state().sessions —
            // safe to load the persisted views (see the comment above). The
            // pet choice needs no such pruning (it isn't session-shaped), so
            // it's applied in the same beat, right alongside `load_views`.
            if was_sessions {
                if let Some(file) = pending_file.take() {
                    app.store.load_views(file.views);
                    app.store.set_pet(file.pet);
                }
            }
        }
        // Downstream of the batch: registry sync, dead-tile disengage,
        // escalation. The bell/title bytes go straight to the terminal
        // between frames — same thread as the draw, so no interleaving.
        if let Some(seq) = app.after_events() {
            let mut out = std::io::stdout();
            let _ = out.write_all(seq.as_bytes());
            let _ = out.flush();
        }
    }
}

#[cfg(test)]
mod tests {
    //! Wave-3 key-routing tests (spec tui-key-routing incl. p8.1–p8.4):
    //! the garage router's armed `x`/`X`/`K` flows with the exact Dart
    //! strip-notice wording (the e2e harnesses grep these strings verbatim),
    //! the lifecycle effects, the engaged-layer encode path with the Ctrl+G
    //! disengage, and the overlay placeholder routing. The store transitions
    //! themselves are covered by `state/store.rs`; these tests cover the
    //! caller-side behavior the Dart TUI kept in `bin/garage_tui.dart`.
    use super::*;
    use crate::api::models::{SessionInfo, WorkspaceInfo};

    fn ws(name: &str) -> WorkspaceInfo {
        WorkspaceInfo {
            name: name.to_owned(),
            dir: Some(format!("/repos/{name}/{name}")),
            branch: None,
        }
    }

    fn si(workspace: &str, label: &str) -> SessionInfo {
        SessionInfo {
            id: format!("garage/{workspace}/{label}"),
            workspace: workspace.to_owned(),
            label: label.to_owned(),
            dir: Some(format!("/repos/{workspace}/{workspace}")),
            attached: false,
            status: "working".to_owned(),
            since: Some(1000),
            message: None,
            branch: None,
            restorable: false,
            title: None,
            context: None,
        }
    }

    fn si_blocked(workspace: &str, label: &str, since: i64) -> SessionInfo {
        SessionInfo {
            status: "needs-input".to_owned(),
            since: Some(since),
            ..si(workspace, label)
        }
    }

    fn si_status(workspace: &str, label: &str, status: &str) -> SessionInfo {
        SessionInfo {
            status: status.to_owned(),
            ..si(workspace, label)
        }
    }

    fn si_restorable(workspace: &str, label: &str) -> SessionInfo {
        SessionInfo {
            status: "restorable".to_owned(),
            since: None,
            restorable: true,
            ..si(workspace, label)
        }
    }

    fn id(workspace: &str, label: &str) -> String {
        format!("garage/{workspace}/{label}")
    }

    fn store_with(workspaces: Vec<WorkspaceInfo>, sessions: Vec<SessionInfo>) -> WallStore {
        let mut store = WallStore::new();
        store.workspaces_fetched(workspaces);
        store.sessions_fetched(sessions);
        store
    }

    fn key_at(
        router: &mut GarageRouter,
        store: &mut WallStore,
        key: &str,
        now: i64,
    ) -> Vec<Effect> {
        router.on_garage_key(store, key, now)
    }

    // ── armed x close (p8.1) ────────────────────────────────────────────

    #[test]
    fn x_arms_with_the_exact_strip_wording_then_a_second_x_closes() {
        let mut store = store_with(vec![ws("a")], vec![si("a", "ghost")]);
        let mut router = GarageRouter::default();
        assert_eq!(key_at(&mut router, &mut store, "x", 1000), vec![]);
        assert_eq!(
            router.notices.current(1001),
            Some("press x again to close ghost"),
            "harness-grepped wording"
        );
        assert_eq!(
            key_at(&mut router, &mut store, "x", 2000),
            vec![Effect::Close {
                id: id("a", "ghost"),
                label: "ghost".to_owned(),
                meta_only: false,
            }]
        );
        assert_eq!(
            router.notices.current(2001),
            None,
            "confirming clears the arm notice"
        );
    }

    #[test]
    fn x_on_a_restorable_placeholder_deletes_meta_only() {
        let mut store = store_with(vec![ws("a")], vec![si_restorable("a", "dead")]);
        let mut router = GarageRouter::default();
        key_at(&mut router, &mut store, "x", 1000);
        assert_eq!(
            key_at(&mut router, &mut store, "x", 2000),
            vec![Effect::Close {
                id: id("a", "dead"),
                label: "dead".to_owned(),
                meta_only: true,
            }]
        );
    }

    #[test]
    fn any_other_key_disarms_the_pending_close_and_clears_its_notice() {
        let mut store = store_with(vec![ws("a")], vec![si("a", "ghost")]);
        let mut router = GarageRouter::default();
        key_at(&mut router, &mut store, "x", 1000);
        // 'z' is unbound: it disarms AND raises the typing hint.
        key_at(&mut router, &mut store, "z", 1100);
        assert_eq!(router.notices.current(1101), Some(TYPING_HINT));
        // The next x must re-arm, not confirm.
        assert_eq!(key_at(&mut router, &mut store, "x", 1200), vec![]);
        assert_eq!(
            router.notices.current(1201),
            Some("press x again to close ghost")
        );
    }

    #[test]
    fn an_expired_x_arm_re_arms_instead_of_confirming() {
        let mut store = store_with(vec![ws("a")], vec![si("a", "ghost")]);
        let mut router = GarageRouter::default();
        key_at(&mut router, &mut store, "x", 1000);
        assert_eq!(
            key_at(&mut router, &mut store, "x", 4001),
            vec![],
            "window expired"
        );
        assert_eq!(key_at(&mut router, &mut store, "x", 4500).len(), 1);
    }

    #[test]
    fn x_with_no_focused_session_is_a_silent_no_op() {
        let mut store = store_with(vec![ws("a")], vec![]);
        let mut router = GarageRouter::default();
        assert_eq!(key_at(&mut router, &mut store, "x", 1000), vec![]);
        assert_eq!(router.notices.current(1001), None);
    }

    // ── armed X workspace removal (p8.3) + K kill confirm (p8.4) ────────

    #[test]
    fn shift_x_arm_notice_carries_the_k_clause_with_live_sessions() {
        let mut store = store_with(vec![ws("proj")], vec![si("proj", "a"), si("proj", "b")]);
        let mut router = GarageRouter::default();
        assert_eq!(key_at(&mut router, &mut store, "X", 1000), vec![]);
        assert_eq!(
            router.notices.current(1001),
            Some("press X again to remove proj (sessions keep running) · K to also kill its 2 sessions")
        );
    }

    #[test]
    fn shift_x_arm_notice_omits_the_k_clause_with_zero_live_sessions() {
        let mut store = store_with(vec![ws("proj")], vec![si_restorable("proj", "dead")]);
        let mut router = GarageRouter::default();
        key_at(&mut router, &mut store, "X", 1000);
        assert_eq!(
            router.notices.current(1001),
            Some("press X again to remove proj (sessions keep running)")
        );
    }

    #[test]
    fn x_x_confirm_is_registry_only_never_sessions_kill() {
        let mut store = store_with(vec![ws("proj")], vec![si("proj", "a")]);
        let mut router = GarageRouter::default();
        key_at(&mut router, &mut store, "X", 1000);
        assert_eq!(
            key_at(&mut router, &mut store, "X", 2000),
            vec![Effect::RemoveWorkspace {
                name: "proj".to_owned(),
                kill: false,
            }]
        );
    }

    #[test]
    fn k_within_the_arm_window_confirms_the_kill_remove() {
        let mut store = store_with(vec![ws("proj")], vec![si("proj", "a"), si("proj", "b")]);
        let mut router = GarageRouter::default();
        key_at(&mut router, &mut store, "X", 1000);
        assert_eq!(
            key_at(&mut router, &mut store, "K", 2500),
            vec![Effect::RemoveWorkspace {
                name: "proj".to_owned(),
                kill: true,
            }]
        );
        assert_eq!(router.notices.current(2501), None, "arm notice cleared");
    }

    #[test]
    fn bare_k_outside_an_arm_stays_an_ordinary_unbound_key() {
        let mut store = store_with(vec![ws("proj")], vec![si("proj", "a")]);
        let mut router = GarageRouter::default();
        assert_eq!(key_at(&mut router, &mut store, "K", 1000), vec![]);
        assert_eq!(
            router.notices.current(1001),
            Some(TYPING_HINT),
            "typing hint, never an arm"
        );
    }

    #[test]
    fn k_after_the_window_expired_confirms_nothing_and_never_re_arms() {
        let mut store = store_with(vec![ws("proj")], vec![si("proj", "a")]);
        let mut router = GarageRouter::default();
        key_at(&mut router, &mut store, "X", 1000);
        assert_eq!(key_at(&mut router, &mut store, "K", 4001), vec![]);
        // The stale arm is gone: an X now re-arms from scratch.
        assert_eq!(key_at(&mut router, &mut store, "X", 4100), vec![]);
        assert!(router
            .notices
            .current(4101)
            .is_some_and(|n| n.starts_with("press X again to remove proj")));
    }

    #[test]
    fn k_on_an_arm_whose_workspace_became_unregistered_is_a_consumed_no_op() {
        let mut store = store_with(vec![ws("proj")], vec![si("proj", "a")]);
        let mut router = GarageRouter::default();
        key_at(&mut router, &mut store, "X", 1000);
        // A refetch unregistered `proj` mid-arm (sessions stay live).
        store.workspaces_fetched(vec![]);
        assert_eq!(
            key_at(&mut router, &mut store, "K", 2000),
            vec![],
            "stale arm: no API call"
        );
    }

    #[test]
    fn shift_x_on_an_unregistered_group_explains_instead_of_arming() {
        // Registry-only removal happened: sessions live on, group synthesized.
        let mut store = store_with(vec![], vec![si("proj", "a")]);
        let mut router = GarageRouter::default();
        assert!(!store.state().groups[0].registered);
        assert_eq!(key_at(&mut router, &mut store, "X", 1000), vec![]);
        assert_eq!(
            router.notices.current(1001),
            Some("already unregistered — sessions live in tmux; x closes them individually")
        );
        // And a K afterwards is unbound (nothing armed).
        assert_eq!(key_at(&mut router, &mut store, "K", 1100), vec![]);
    }

    #[test]
    fn x_and_shift_x_cross_disarm_each_other() {
        let mut store = store_with(vec![ws("proj")], vec![si("proj", "a")]);
        let mut router = GarageRouter::default();
        key_at(&mut router, &mut store, "x", 1000); // close armed
        key_at(&mut router, &mut store, "X", 1100); // arms removal, disarms close
        assert!(router
            .notices
            .current(1101)
            .is_some_and(|n| n.starts_with("press X again to remove proj")));
        // x now re-arms (the X disarmed it) AND disarms the removal.
        assert_eq!(key_at(&mut router, &mut store, "x", 1200), vec![]);
        assert_eq!(
            router.notices.current(1201),
            Some("press x again to close a")
        );
        // The x disarmed the removal: this X re-arms instead of confirming.
        assert_eq!(key_at(&mut router, &mut store, "X", 1300), vec![]);
        assert!(router
            .notices
            .current(1301)
            .is_some_and(|n| n.starts_with("press X again to remove proj")));
    }

    // ── p16-restart: the `r` prefix chords (spec restart "TUI chords") ──

    #[test]
    fn r_opens_the_prefix_with_a_hint_and_r_r_arms_then_restarts() {
        let mut store = store_with(vec![ws("a")], vec![si_status("a", "one", "idle")]);
        let mut router = GarageRouter::default();
        assert_eq!(key_at(&mut router, &mut store, "r", 1000), vec![]);
        assert_eq!(router.notices.current(1001), Some(RESTART_PREFIX_HINT));
        assert_eq!(key_at(&mut router, &mut store, "r", 1100), vec![]);
        assert_eq!(
            router.notices.current(1101),
            Some("restart one? r r again — resumes the conversation")
        );
        // Second `r r` inside the window fires, unforced (the session is idle).
        assert_eq!(key_at(&mut router, &mut store, "r", 1200), vec![]);
        assert_eq!(
            key_at(&mut router, &mut store, "r", 1300),
            vec![Effect::RestartSession {
                id: id("a", "one"),
                name: "one".to_owned(),
                force: false,
            }]
        );
        assert_eq!(
            router.notices.current(1301),
            Some("restarting one — resumes the conversation")
        );
    }

    #[test]
    fn r_r_on_a_working_session_warns_and_the_second_press_forces() {
        // `si` defaults to `working`.
        let mut store = store_with(vec![ws("a")], vec![si("a", "busy")]);
        let mut router = GarageRouter::default();
        key_at(&mut router, &mut store, "r", 1000);
        key_at(&mut router, &mut store, "r", 1100);
        assert_eq!(
            router.notices.current(1101),
            Some("busy is working — r r again to restart anyway")
        );
        key_at(&mut router, &mut store, "r", 1200);
        assert_eq!(
            key_at(&mut router, &mut store, "r", 1300),
            vec![Effect::RestartSession {
                id: id("a", "busy"),
                name: "busy".to_owned(),
                force: true,
            }]
        );
    }

    #[test]
    fn an_expired_r_r_arm_re_arms_instead_of_restarting() {
        let mut store = store_with(vec![ws("a")], vec![si_status("a", "one", "idle")]);
        let mut router = GarageRouter::default();
        key_at(&mut router, &mut store, "r", 1000);
        key_at(&mut router, &mut store, "r", 1100);
        key_at(&mut router, &mut store, "r", 5000);
        assert_eq!(
            key_at(&mut router, &mut store, "r", 5100),
            vec![],
            "the 3s window expired — this re-arms"
        );
        assert_eq!(
            router.notices.current(5101),
            Some("restart one? r r again — resumes the conversation")
        );
    }

    #[test]
    fn any_other_key_after_r_cancels_the_prefix_and_is_swallowed() {
        let mut store = store_with(vec![ws("a")], vec![si_status("a", "one", "idle")]);
        let mut router = GarageRouter::default();
        key_at(&mut router, &mut store, "r", 1000);
        // `x` after the prefix neither closes nor arms a close: the chord
        // cancelled and ate the key.
        assert_eq!(key_at(&mut router, &mut store, "x", 1100), vec![]);
        assert_eq!(router.notices.current(1101), None, "the hint is cleared");
        assert_eq!(key_at(&mut router, &mut store, "x", 1200), vec![]);
        assert_eq!(
            router.notices.current(1201),
            Some("press x again to close one"),
            "the close arms from scratch"
        );
    }

    #[test]
    fn a_cancelled_chord_disarms_a_pending_r_r() {
        let mut store = store_with(vec![ws("a")], vec![si_status("a", "one", "idle")]);
        let mut router = GarageRouter::default();
        key_at(&mut router, &mut store, "r", 1000);
        key_at(&mut router, &mut store, "r", 1100); // armed
        key_at(&mut router, &mut store, "r", 1200);
        key_at(&mut router, &mut store, "z", 1300); // cancels the chord
        assert_eq!(router.notices.current(1301), None, "the arm notice went too");
        // The next full chord must re-arm, not fire.
        key_at(&mut router, &mut store, "r", 1400);
        assert_eq!(key_at(&mut router, &mut store, "r", 1500), vec![]);
    }

    #[test]
    fn r_r_on_a_restorable_placeholder_explains_instead_of_restarting() {
        let mut store = store_with(vec![ws("a")], vec![si_restorable("a", "dead")]);
        let mut router = GarageRouter::default();
        key_at(&mut router, &mut store, "r", 1000);
        assert_eq!(key_at(&mut router, &mut store, "r", 1100), vec![]);
        assert_eq!(
            router.notices.current(1101),
            Some("no live session to restart")
        );
    }

    #[test]
    fn r_a_restarts_the_workspaces_idle_and_done_sessions_never_the_busy_ones() {
        let mut store = store_with(
            vec![ws("a")],
            vec![
                si_status("a", "idle-one", "idle"),
                si("a", "busy"),
                si_blocked("a", "blocked", 900),
                si_status("a", "done-one", "done"),
                si_restorable("a", "dead"),
            ],
        );
        let mut router = GarageRouter::default();
        let focused = store.state().focused_session_id.clone();
        key_at(&mut router, &mut store, "r", 1000);
        assert_eq!(
            key_at(&mut router, &mut store, "a", 1100),
            vec![Effect::RestartWorkspace {
                ids: vec![id("a", "idle-one"), id("a", "done-one")],
            }]
        );
        assert_eq!(
            router.notices.current(1101),
            Some("restarting 2 sessions in a")
        );
        // The `a` jump never ran: it would have engaged the blocked session.
        assert_eq!(store.state().layer, KeyLayer::Garage);
        assert_eq!(store.state().focused_session_id, focused);
    }

    #[test]
    fn r_a_with_no_idle_session_says_so_and_fires_nothing() {
        let mut store = store_with(vec![ws("a")], vec![si("a", "busy")]);
        let mut router = GarageRouter::default();
        key_at(&mut router, &mut store, "r", 1000);
        assert_eq!(key_at(&mut router, &mut store, "a", 1100), vec![]);
        assert_eq!(
            router.notices.current(1101),
            Some("nothing to restart in a (all busy or none)")
        );
    }

    #[test]
    fn r_d_restarts_the_daemon_at_once() {
        let mut store = store_with(vec![ws("a")], vec![si("a", "one")]);
        let mut router = GarageRouter::default();
        key_at(&mut router, &mut store, "r", 1000);
        assert_eq!(
            key_at(&mut router, &mut store, "d", 1100),
            vec![Effect::RestartDaemon]
        );
        assert_eq!(router.notices.current(1101), Some("restarting the daemon…"));
        // The prefix is one-shot: a bare `d` afterwards is the ordinary
        // detach binding again, never a second daemon restart.
        assert_eq!(key_at(&mut router, &mut store, "d", 1200), vec![]);
    }

    #[test]
    fn the_restart_response_names_every_outcome() {
        use crate::api::models::{FailedEntry, RestartedEntry, SkippedEntry};
        let resumed = RestartResponse {
            restarted: vec![RestartedEntry {
                id: id("a", "one"),
                resumed: true,
            }],
            ..RestartResponse::default()
        };
        assert_eq!(restart_notice("one", &resumed), "one restarted");
        let fresh = RestartResponse {
            restarted: vec![RestartedEntry {
                id: id("a", "one"),
                resumed: false,
            }],
            ..RestartResponse::default()
        };
        assert_eq!(
            restart_notice("one", &fresh),
            "one restarted fresh (no conversation to resume)"
        );
        let failed = RestartResponse {
            failed: vec![FailedEntry {
                id: id("a", "one"),
                error: "no server running".to_owned(),
            }],
            ..RestartResponse::default()
        };
        assert_eq!(restart_notice("one", &failed), "one: no server running");
        let skipped = RestartResponse {
            skipped: vec![SkippedEntry {
                id: id("a", "one"),
                status: "working".to_owned(),
            }],
            ..RestartResponse::default()
        };
        assert_eq!(restart_notice("one", &skipped), "one skipped — working");
        assert_eq!(
            restart_notice("one", &RestartResponse::default()),
            "one: the daemon reported no outcome",
            "an empty response is still reported, never silent"
        );
    }

    #[test]
    fn the_workspace_restart_line_names_skips_and_failures_only_when_there_are_any() {
        assert_eq!(restart_all_notice(3, 0, &[]), "restarted 3");
        assert_eq!(
            restart_all_notice(2, 1, &[]),
            "restarted 2 · skipped 1 (working/waiting)"
        );
        assert_eq!(
            restart_all_notice(1, 1, &["tmux: no such pane".to_owned()]),
            "restarted 1 · skipped 1 (working/waiting) · failed 1: tmux: no such pane"
        );
    }

    // ── restore (p8.1: Enter on restorable, R restore-all) ──────────────

    #[test]
    fn enter_on_a_restorable_tile_restores_it_once() {
        let mut store = store_with(vec![ws("a")], vec![si_restorable("a", "dead")]);
        let mut router = GarageRouter::default();
        assert_eq!(
            key_at(&mut router, &mut store, "\r", 1000),
            vec![Effect::Restore { id: id("a", "dead") }]
        );
        assert_eq!(store.state().layer, KeyLayer::Garage, "engage still declined");
        // A second Enter while the restore is in flight must not duplicate.
        assert_eq!(key_at(&mut router, &mut store, "\r", 1100), vec![]);
        // Settling releases the guard.
        router.restore_settled(&id("a", "dead"));
        assert_eq!(key_at(&mut router, &mut store, "\r", 1200).len(), 1);
    }

    #[test]
    fn enter_on_a_live_tile_engages_no_effects() {
        let mut store = store_with(vec![ws("a")], vec![si("a", "one")]);
        let mut router = GarageRouter::default();
        assert_eq!(key_at(&mut router, &mut store, "\r", 1000), vec![]);
        assert_eq!(store.state().layer, KeyLayer::Engaged);
    }

    #[test]
    fn r_restores_every_restorable_session_in_the_focused_workspace() {
        let mut store = store_with(
            vec![ws("a"), ws("b")],
            vec![
                si("a", "live"),
                si_restorable("a", "r1"),
                si_restorable("b", "other"),
                si_restorable("a", "r2"),
            ],
        );
        let mut router = GarageRouter::default();
        assert_eq!(
            key_at(&mut router, &mut store, "R", 1000),
            vec![
                Effect::Restore { id: id("a", "r1") },
                Effect::Restore { id: id("a", "r2") },
            ],
            "parallel per-id calls, focused workspace only"
        );
    }

    #[test]
    fn r_with_nothing_restorable_shows_the_notice() {
        let mut store = store_with(vec![ws("a")], vec![si("a", "one")]);
        let mut router = GarageRouter::default();
        assert_eq!(key_at(&mut router, &mut store, "R", 1000), vec![]);
        assert_eq!(router.notices.current(1001), Some("nothing to restore in a"));
    }

    // ── spawn (n / N) ───────────────────────────────────────────────────

    #[test]
    fn n_spawns_with_the_first_free_claude_n_label() {
        let mut store = store_with(
            vec![ws("a")],
            vec![si("a", "claude-1"), si("a", "claude-3")],
        );
        let mut router = GarageRouter::default();
        assert_eq!(
            key_at(&mut router, &mut store, "n", 1000),
            vec![Effect::Spawn {
                workspace: "a".to_owned(),
                label: "claude-2".to_owned(),
                worktree: false,
            }]
        );
    }

    #[test]
    fn shift_n_spawns_a_worktree_session() {
        let mut store = store_with(vec![ws("a")], vec![si("a", "one")]);
        let mut router = GarageRouter::default();
        assert_eq!(
            key_at(&mut router, &mut store, "N", 1000),
            vec![Effect::Spawn {
                workspace: "a".to_owned(),
                label: "claude-1".to_owned(),
                worktree: true,
            }]
        );
    }

    #[test]
    fn spawn_is_busy_guarded_until_settled() {
        let mut store = store_with(vec![ws("a")], vec![si("a", "one")]);
        let mut router = GarageRouter::default();
        assert_eq!(key_at(&mut router, &mut store, "n", 1000).len(), 1);
        assert_eq!(
            key_at(&mut router, &mut store, "n", 1100),
            vec![],
            "in flight"
        );
        router.spawn_settled();
        assert_eq!(key_at(&mut router, &mut store, "n", 1200).len(), 1);
    }

    // ── p11: install statusline (spec tui-context-meters "Install
    // affordance") ──────────────────────────────────────────────────────

    #[test]
    fn shift_i_fires_the_install_effect_busy_guarded_until_settled() {
        let mut store = store_with(vec![ws("a")], vec![si("a", "one")]);
        let mut router = GarageRouter::default();
        assert_eq!(key_at(&mut router, &mut store, "I", 1000), vec![Effect::Install]);
        assert_eq!(
            key_at(&mut router, &mut store, "I", 1100),
            vec![],
            "in flight"
        );
        router.install_settled();
        assert_eq!(key_at(&mut router, &mut store, "I", 1200), vec![Effect::Install]);
    }

    #[test]
    fn install_statusline_notice_strings() {
        assert_eq!(
            INSTALL_STATUSLINE_SUCCESS,
            "statusline feed installed — meters go live as agents work"
        );
        assert_eq!(
            failure_notice("statusline install failed", &ApiError::Transport("x".into())),
            "statusline install failed"
        );
        assert_eq!(
            failure_notice(
                "statusline install failed",
                &ApiError::Status { status: 500, message: "boom".into() }
            ),
            "statusline install failed: boom"
        );
    }

    // ── p17: `I` installs hooks too (spec tui-hooks-install) ─────────────

    fn hooks(already_installed: bool) -> Result<HooksInstallResult, ApiError> {
        Ok(HooksInstallResult { already_installed })
    }

    fn status_err(status: u16, message: &str) -> ApiError {
        ApiError::Status { status, message: message.to_owned() }
    }

    #[test]
    fn install_notice_both_installed() {
        assert_eq!(
            install_notice(&Ok(()), &hooks(false)),
            "hooks installed · statusline feed installed — meters go live as agents work"
        );
    }

    #[test]
    fn install_notice_hooks_already_installed_is_not_an_error() {
        assert_eq!(
            install_notice(&Ok(()), &hooks(true)),
            "hooks already installed · statusline feed installed — meters go live as agents \
             work"
        );
    }

    #[test]
    fn install_notice_names_the_failed_side_and_still_reports_the_other() {
        let corrupt = "~/.claude/settings.json is not valid JSON";
        assert_eq!(
            install_notice(&Ok(()), &Err(status_err(422, corrupt))),
            format!(
                "hooks install failed: {corrupt} · statusline feed installed — meters go live \
                 as agents work"
            )
        );
        assert_eq!(
            install_notice(&Err(status_err(500, "boom")), &hooks(false)),
            "hooks installed · statusline install failed: boom"
        );
        assert_eq!(
            install_notice(&Err(ApiError::Transport("x".into())), &hooks(true)),
            "hooks already installed · statusline install failed"
        );
        assert_eq!(
            install_notice(&Err(status_err(500, "a")), &Err(status_err(422, "b"))),
            "hooks install failed: b · statusline install failed: a"
        );
    }

    // ── p12: standalone window ("t" opens the focused session in its own
    // OS terminal — an effect the store always declines) ──────────────────

    #[test]
    fn t_with_no_focused_session_is_a_silent_no_op() {
        let mut store = store_with(vec![ws("a")], vec![]);
        let mut router = GarageRouter::default();
        assert_eq!(key_at(&mut router, &mut store, "t", 1000), vec![]);
        assert_eq!(router.notices.current(1001), None);
    }

    #[test]
    fn t_on_a_restorable_session_shows_no_live_session_notice() {
        let mut store = store_with(vec![ws("a")], vec![si_restorable("a", "dead")]);
        let mut router = GarageRouter::default();
        assert_eq!(key_at(&mut router, &mut store, "t", 1000), vec![]);
        assert_eq!(
            router.notices.current(1001),
            Some("no live session to open")
        );
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn t_on_a_live_session_fires_open_window() {
        let mut store = store_with(vec![ws("a")], vec![si("a", "one")]);
        let mut router = GarageRouter::default();
        assert_eq!(
            key_at(&mut router, &mut store, "t", 1000),
            vec![Effect::OpenWindow {
                id: id("a", "one"),
                label: "one".to_owned(),
            }]
        );
    }

    #[cfg(not(target_os = "macos"))]
    #[test]
    fn t_off_macos_shows_macos_only_notice_even_for_a_live_session() {
        let mut store = store_with(vec![ws("a")], vec![si("a", "one")]);
        let mut router = GarageRouter::default();
        assert_eq!(key_at(&mut router, &mut store, "t", 1000), vec![]);
        assert_eq!(
            router.notices.current(1001),
            Some("standalone windows: macOS only for now")
        );
    }

    #[test]
    fn t_open_window_success_and_failure_notices() {
        assert_eq!(
            format!("opened {} in a new window", "one"),
            "opened one in a new window"
        );
        let err = std::io::Error::other("boom");
        assert_eq!(
            format!("open window failed: {err}"),
            "open window failed: boom"
        );
    }

    #[test]
    fn install_hint_condition_gates_on_shown_delay_and_statusline_sessions() {
        use crate::api::models::ContextInfo;

        let none_yet = vec![WallSession::from_info(&si("a", "one"), None)];
        assert!(
            !should_show_install_hint(false, Duration::from_secs(29), &none_yet),
            "too early"
        );
        assert!(should_show_install_hint(false, Duration::from_secs(30), &none_yet));
        assert!(
            !should_show_install_hint(true, Duration::from_secs(60), &none_yet),
            "already shown this run — never again"
        );

        let transcript_only = vec![WallSession::from_info(
            &SessionInfo {
                context: Some(ContextInfo { used_percentage: 10, source: "transcript".into() }),
                ..si("a", "one")
            },
            None,
        )];
        assert!(
            should_show_install_hint(false, Duration::from_secs(60), &transcript_only),
            "transcript-sourced context doesn't count as statusline-sourced"
        );

        let with_statusline = vec![WallSession::from_info(
            &SessionInfo {
                context: Some(ContextInfo { used_percentage: 10, source: "statusline".into() }),
                ..si("a", "one")
            },
            None,
        )];
        assert!(!should_show_install_hint(false, Duration::from_secs(60), &with_statusline));
    }

    #[test]
    fn any_keypress_dismisses_the_install_hint_forever_this_run() {
        let mut router = GarageRouter::default();
        router.notices.show(crate::ui::strip::STATUSLINE_HINT, 8000, 1000);
        assert_eq!(router.notices.current(1001), Some(crate::ui::strip::STATUSLINE_HINT));
        // The dismissal check runtime.rs's `App::apply` runs before ordinary
        // key handling — exercised directly here against `Notices` since
        // building a full `App` needs a terminal/registry this module
        // doesn't stand up in tests.
        if router.notices.current(1001) == Some(crate::ui::strip::STATUSLINE_HINT) {
            router.notices.clear_prefix(crate::ui::strip::STATUSLINE_HINT);
        }
        assert_eq!(router.notices.current(1001), None, "any keypress clears it");
    }

    #[test]
    fn next_label_ignores_foreign_and_malformed_labels() {
        let store = store_with(
            vec![ws("a"), ws("b")],
            vec![
                si("b", "claude-1"),    // other workspace
                si("a", "claude-x"),    // not the family
                si("a", "claude-1a"),   // trailing junk
                si("a", "api-fix"),
            ],
        );
        assert_eq!(next_label(store.state(), "a"), "claude-1");
    }

    // ── the rest of the garage bindings ─────────────────────────────────

    #[test]
    fn q_quits_via_the_effect() {
        let mut store = store_with(vec![ws("a")], vec![si("a", "one")]);
        let mut router = GarageRouter::default();
        assert_eq!(
            key_at(&mut router, &mut store, "q", 1000),
            vec![Effect::Quit]
        );
    }

    #[test]
    fn a_with_nothing_blocked_shows_the_no_session_notice() {
        let mut store = store_with(vec![ws("a")], vec![si("a", "one")]);
        let mut router = GarageRouter::default();
        assert_eq!(key_at(&mut router, &mut store, "a", 1000), vec![]);
        assert_eq!(router.notices.current(1001), Some("no session needs you"));
    }

    #[test]
    fn a_with_a_blocked_session_jumps_engaged_no_effects() {
        let mut store = store_with(
            vec![ws("a"), ws("b")],
            vec![si("a", "one"), si_blocked("b", "old", 100)],
        );
        let mut router = GarageRouter::default();
        assert_eq!(key_at(&mut router, &mut store, "a", 1000), vec![]);
        assert_eq!(store.state().layer, KeyLayer::Engaged);
        assert_eq!(store.state().focused_session_id, Some(id("b", "old")));
    }

    #[test]
    fn unbound_printables_raise_the_typing_hint_rate_limited() {
        let mut store = store_with(vec![ws("a")], vec![si("a", "one")]);
        let mut router = GarageRouter::default();
        key_at(&mut router, &mut store, "z", 1000);
        assert_eq!(router.notices.current(1001), Some(TYPING_HINT));
        // Burst typing must not restart the timer: still expires at 3500.
        key_at(&mut router, &mut store, "y", 2000);
        assert_eq!(router.notices.current(3499), Some(TYPING_HINT));
        assert_eq!(router.notices.current(3501), None);
    }

    #[test]
    fn notices_expire_via_tick() {
        let mut notices = Notices::default();
        notices.show("closed ghost", 2000, 1000);
        assert_eq!(notices.current(2999), Some("closed ghost"));
        assert!(!notices.tick(2000), "not expired yet — no change");
        assert!(notices.tick(3000), "expired — visible state changed");
        assert_eq!(notices.current(3000), None);
    }

    // ── pit-pet chatter vs. user notices (spec tui-pit-pet "Species-voiced
    // chatter": "SHALL NOT replace a non-pet notice that is still showing")

    #[test]
    fn show_pet_is_refused_over_a_live_user_notice() {
        let mut notices = Notices::default();
        notices.show("press x again to close", 3000, 1000);
        assert!(!notices.show_pet("drink some water", 4000, 1500));
        // The user notice is untouched, not clobbered nor extended.
        assert_eq!(notices.current(1500), Some("press x again to close"));
    }

    #[test]
    fn show_pet_is_allowed_once_the_user_notice_has_expired() {
        let mut notices = Notices::default();
        notices.show("press x again to close", 1000, 1000); // expires at 2000
        assert!(notices.show_pet("drink some water", 4000, 2500));
        assert_eq!(notices.current(2500), Some("drink some water"));
    }

    #[test]
    fn a_user_notice_always_overrides_a_live_pet_notice_immediately() {
        let mut notices = Notices::default();
        assert!(notices.show_pet("drink some water", 4000, 1000));
        notices.show(TYPING_HINT, 2500, 1500);
        assert_eq!(notices.current(1500), Some(TYPING_HINT));
    }

    #[test]
    fn a_pet_notice_freely_replaces_another_pet_notice() {
        let mut notices = Notices::default();
        assert!(notices.show_pet("drink some water", 4000, 1000));
        assert!(notices.show_pet("proud of you", 4000, 1200));
        assert_eq!(notices.current(1200), Some("proud of you"));
    }

    #[test]
    fn user_notice_active_tracks_kind_and_expiry() {
        let mut notices = Notices::default();
        assert!(!notices.user_notice_active(1000), "nothing showing yet");
        notices.show("press x again to close", 1000, 1000); // expires at 2000
        assert!(notices.user_notice_active(1500));
        assert!(!notices.user_notice_active(2000), "expired");
        notices.show_pet("drink some water", 4000, 2000);
        assert!(!notices.user_notice_active(2500), "a pet notice is not a user notice");
    }

    // ── key-event reduction ─────────────────────────────────────────────

    #[test]
    fn garage_key_string_normalizes_enter_and_passes_plain_chars() {
        let plain =
            |code| garage_key_string(&KeyEvent::new(code, KeyModifiers::NONE));
        assert_eq!(plain(KeyCode::Enter).as_deref(), Some("\r"));
        assert_eq!(plain(KeyCode::Char('x')).as_deref(), Some("x"));
        // Shifted capitals keep their character (the X/N/A/R/K bindings).
        assert_eq!(
            garage_key_string(&KeyEvent::new(KeyCode::Char('X'), KeyModifiers::SHIFT)).as_deref(),
            Some("X")
        );
        assert_eq!(
            garage_key_string(&KeyEvent::new(KeyCode::Char('?'), KeyModifiers::SHIFT)).as_deref(),
            Some("?")
        );
    }

    #[test]
    fn garage_key_string_maps_tab_to_cycle_view_the_wave_1b_follow_up() {
        assert_eq!(
            garage_key_string(&KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE)).as_deref(),
            Some("\t")
        );
        assert_eq!(
            garage_command_for(&garage_key_string(&KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE)).unwrap()),
            Some(GarageCommand::CycleView)
        );
    }

    #[test]
    fn ctrl_and_alt_modified_keys_are_consumed_without_reaching_the_bindings() {
        // Mirrors the Dart parser's character-null control events: consumed,
        // no disarm, no typing hint, never an agent byte.
        assert_eq!(
            garage_key_string(&KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL)),
            None
        );
        assert_eq!(
            garage_key_string(&KeyEvent::new(KeyCode::Char('x'), KeyModifiers::ALT)),
            None
        );
        assert_eq!(
            garage_key_string(&KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)),
            None
        );
    }

    // ── engaged layer ───────────────────────────────────────────────────

    #[test]
    fn ctrl_g_disengages_and_is_never_forwarded() {
        assert_eq!(
            engaged_input_for(&KeyEvent::new(KeyCode::Char('g'), KeyModifiers::CONTROL)),
            EngagedInput::Disengage
        );
    }

    #[test]
    fn esc_is_always_forwarded_while_engaged() {
        // Spec scenario "Esc reaches Claude Code": 0x1b, layer untouched.
        assert_eq!(
            engaged_input_for(&KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)),
            EngagedInput::Write(vec![0x1b])
        );
    }

    #[test]
    fn engaged_bytes_match_the_spec_scenarios() {
        let cases: [(KeyCode, KeyModifiers, &[u8]); 4] = [
            (KeyCode::Char('c'), KeyModifiers::CONTROL, b"\x03"),
            (KeyCode::Right, KeyModifiers::ALT, b"\x1b[1;3C"),
            (KeyCode::BackTab, KeyModifiers::SHIFT, b"\x1b[Z"),
            (KeyCode::Char('h'), KeyModifiers::NONE, b"h"),
        ];
        for (code, mods, expected) in cases {
            assert_eq!(
                engaged_input_for(&KeyEvent::new(code, mods)),
                EngagedInput::Write(expected.to_vec()),
                "{code:?} {mods:?}"
            );
        }
    }

    #[test]
    fn a_bare_modifier_press_forwards_nothing() {
        use crossterm::event::ModifierKeyCode;
        assert_eq!(
            engaged_input_for(&KeyEvent::new(
                KeyCode::Modifier(ModifierKeyCode::LeftControl),
                KeyModifiers::CONTROL
            )),
            EngagedInput::Nothing
        );
    }

    // ── overlay layer (task 4.3/4.4: real handlers keyed by OverlayKind) ─

    /// Route one overlay key with a permissive dir_exists (tests that need
    /// validation inject their own).
    fn overlay_key(
        store: &mut WallStore,
        selection: &mut usize,
        form: &mut WorkspaceAddForm,
        key: KeyEvent,
    ) -> Vec<Effect> {
        let mut view_picker = ViewPickerState::default();
        handle_overlay_key(store, &key, selection, form, &mut view_picker, "/home/me", &|_| true).0
    }

    fn plain(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    #[test]
    fn question_mark_toggles_help_and_esc_or_q_also_close_it() {
        for closer in [
            KeyEvent::new(KeyCode::Char('?'), KeyModifiers::SHIFT),
            plain(KeyCode::Char('q')),
            plain(KeyCode::Esc),
        ] {
            let mut store = store_with(vec![ws("a")], vec![si("a", "one")]);
            let mut router = GarageRouter::default();
            key_at(&mut router, &mut store, "?", 1000);
            assert_eq!(store.state().overlay, Some(OverlayKind::Help));
            let (mut sel, mut form) = (0, WorkspaceAddForm::default());
            overlay_key(&mut store, &mut sel, &mut form, closer);
            assert_eq!(store.state().layer, KeyLayer::Garage);
            assert_eq!(store.state().overlay, None);
        }
    }

    #[test]
    fn help_overlay_consumes_other_keys_without_closing() {
        let mut store = store_with(vec![ws("a")], vec![si("a", "one")]);
        store.open_overlay(OverlayKind::Help);
        let (mut sel, mut form) = (0, WorkspaceAddForm::default());
        overlay_key(&mut store, &mut sel, &mut form, plain(KeyCode::Char('x')));
        assert_eq!(store.state().overlay, Some(OverlayKind::Help));
    }

    #[test]
    fn queue_j_k_wrap_the_selection_and_esc_closes() {
        let mut store = store_with(
            vec![ws("a")],
            vec![
                si_blocked("a", "b1", 100),
                si_blocked("a", "b2", 200),
                si("a", "live"),
            ],
        );
        store.open_overlay(OverlayKind::TriageQueue);
        let (mut sel, mut form) = (0, WorkspaceAddForm::default());
        overlay_key(&mut store, &mut sel, &mut form, plain(KeyCode::Char('j')));
        assert_eq!(sel, 1);
        overlay_key(&mut store, &mut sel, &mut form, plain(KeyCode::Char('j')));
        assert_eq!(sel, 0, "wraps past the end");
        overlay_key(&mut store, &mut sel, &mut form, plain(KeyCode::Char('k')));
        assert_eq!(sel, 1, "wraps backwards too");
        // Garage keys must not leak through the overlay layer.
        overlay_key(&mut store, &mut sel, &mut form, plain(KeyCode::Char('q')));
        assert_eq!(store.state().overlay, Some(OverlayKind::TriageQueue));
        overlay_key(&mut store, &mut sel, &mut form, plain(KeyCode::Esc));
        assert_eq!(store.state().layer, KeyLayer::Garage);
    }

    #[test]
    fn queue_enter_jump_engages_the_selected_session() {
        let mut store = store_with(
            vec![ws("a"), ws("b")],
            vec![si("a", "one"), si_blocked("b", "old", 100), si_blocked("a", "young", 900)],
        );
        store.open_overlay(OverlayKind::TriageQueue);
        let (mut sel, mut form) = (1, WorkspaceAddForm::default());
        // Row 0 = b/old (longest waiting), row 1 = a/young.
        overlay_key(&mut store, &mut sel, &mut form, plain(KeyCode::Enter));
        assert_eq!(store.state().overlay, None);
        assert_eq!(store.state().layer, KeyLayer::Engaged);
        assert_eq!(store.state().focused_session_id, Some(id("a", "young")));
    }

    #[test]
    fn queue_enter_on_an_empty_queue_just_closes() {
        let mut store = store_with(vec![ws("a")], vec![si("a", "one")]);
        store.open_overlay(OverlayKind::TriageQueue);
        let (mut sel, mut form) = (0, WorkspaceAddForm::default());
        overlay_key(&mut store, &mut sel, &mut form, plain(KeyCode::Enter));
        assert_eq!(store.state().overlay, None);
        assert_eq!(store.state().layer, KeyLayer::Garage);
    }

    #[test]
    fn workspace_add_types_into_the_field_and_esc_cancels() {
        let mut store = store_with(vec![ws("a")], vec![si("a", "one")]);
        store.open_overlay(OverlayKind::WorkspaceAdd);
        let (mut sel, mut form) = (0, WorkspaceAddForm::default());
        for c in ['~', '/', 'q', 'x'] {
            overlay_key(&mut store, &mut sel, &mut form, plain(KeyCode::Char(c)));
        }
        assert_eq!(form.input, "~/qx", "q types — it never quits the overlay");
        overlay_key(&mut store, &mut sel, &mut form, plain(KeyCode::Backspace));
        assert_eq!(form.input, "~/q");
        overlay_key(&mut store, &mut sel, &mut form, plain(KeyCode::Esc));
        assert_eq!(store.state().layer, KeyLayer::Garage);
        assert_eq!(form.input, "", "Esc resets the form");
    }

    #[test]
    fn workspace_add_enter_submits_the_validated_path_as_an_effect() {
        let mut store = store_with(vec![ws("a")], vec![si("a", "one")]);
        store.open_overlay(OverlayKind::WorkspaceAdd);
        let (mut sel, mut form) = (0, WorkspaceAddForm::default());
        form.input = "~/dev/proj".to_owned();
        let effects = overlay_key(&mut store, &mut sel, &mut form, plain(KeyCode::Enter));
        assert_eq!(
            effects,
            vec![Effect::AddWorkspace {
                name: "proj".to_owned(),
                dir: "/home/me/dev/proj".to_owned(),
            }]
        );
        assert!(form.busy, "PUT in flight");
        assert_eq!(
            store.state().overlay,
            Some(OverlayKind::WorkspaceAdd),
            "stays open until the settle"
        );
    }

    #[test]
    fn workspace_add_validation_failure_keeps_the_overlay_open_with_the_error() {
        let mut store = store_with(vec![ws("a")], vec![si("a", "one")]);
        store.open_overlay(OverlayKind::WorkspaceAdd);
        let (mut sel, mut form) = (0, WorkspaceAddForm::default());
        form.input = "/nope".to_owned();
        let mut view_picker = ViewPickerState::default();
        let (effects, _) = handle_overlay_key(
            &mut store,
            &plain(KeyCode::Enter),
            &mut sel,
            &mut form,
            &mut view_picker,
            "/h",
            &|_| false,
        );
        assert!(effects.is_empty());
        assert_eq!(form.error.as_deref(), Some("no such directory: /nope"));
        assert_eq!(store.state().overlay, Some(OverlayKind::WorkspaceAdd));
    }

    #[test]
    fn workspace_add_ctrl_o_fires_the_picker_once_and_blocks_enter() {
        let mut store = store_with(vec![ws("a")], vec![si("a", "one")]);
        store.open_overlay(OverlayKind::WorkspaceAdd);
        let (mut sel, mut form) = (0, WorkspaceAddForm::default());
        form.input = "~/dev/proj".to_owned();
        let ctrl_o = KeyEvent::new(KeyCode::Char('o'), KeyModifiers::CONTROL);
        let effects = overlay_key(&mut store, &mut sel, &mut form, ctrl_o);
        assert_eq!(effects, vec![Effect::PickDirectory]);
        assert!(form.picking);
        assert_eq!(form.input, "~/dev/proj", "Ctrl+O never types an o");
        // While the dialog is open: a second Ctrl+O and Enter are no-ops.
        assert!(overlay_key(&mut store, &mut sel, &mut form, ctrl_o).is_empty());
        assert!(overlay_key(&mut store, &mut sel, &mut form, plain(KeyCode::Enter)).is_empty());
        assert!(!form.busy);
        // Esc still closes, and clears the in-flight pick.
        overlay_key(&mut store, &mut sel, &mut form, plain(KeyCode::Esc));
        assert_eq!(store.state().overlay, None);
        assert!(!form.picking);
    }

    #[test]
    fn workspace_add_ctrl_o_is_ignored_while_registering() {
        let mut store = store_with(vec![ws("a")], vec![si("a", "one")]);
        store.open_overlay(OverlayKind::WorkspaceAdd);
        let (mut sel, mut form) = (0, WorkspaceAddForm::default());
        form.busy = true;
        let ctrl_o = KeyEvent::new(KeyCode::Char('o'), KeyModifiers::CONTROL);
        assert!(overlay_key(&mut store, &mut sel, &mut form, ctrl_o).is_empty());
        assert!(!form.picking);
    }

    #[test]
    fn pastes_route_only_to_the_tile_or_the_workspace_field() {
        assert_eq!(paste_target(KeyLayer::Engaged, None), PasteTarget::Tile);
        assert_eq!(
            paste_target(KeyLayer::Overlay, Some(OverlayKind::WorkspaceAdd)),
            PasteTarget::WorkspaceField
        );
        for overlay in [
            OverlayKind::Help,
            OverlayKind::TriageQueue,
            OverlayKind::ViewPicker,
            OverlayKind::WorktreeFinish,
        ] {
            assert_eq!(
                paste_target(KeyLayer::Overlay, Some(overlay)),
                PasteTarget::Drop,
                "{overlay:?} must swallow pastes"
            );
        }
        assert_eq!(paste_target(KeyLayer::Garage, None), PasteTarget::Drop);
    }

    #[test]
    fn a_paste_in_the_workspace_overlay_lands_in_the_field() {
        // The WorkspaceField branch of handle_paste, end to end on the form.
        let mut store = store_with(vec![ws("a")], vec![si("a", "one")]);
        store.open_overlay(OverlayKind::WorkspaceAdd);
        let mut form = WorkspaceAddForm::default();
        let state = store.state();
        assert_eq!(
            paste_target(state.layer, state.overlay),
            PasteTarget::WorkspaceField
        );
        form.insert_paste("/Users/me/dev/My\\ Proj\n");
        assert_eq!(form.input, "/Users/me/dev/My Proj");
    }

    #[test]
    fn workspace_add_name_collides_against_existing_groups() {
        let mut store = store_with(vec![ws("proj")], vec![]);
        store.open_overlay(OverlayKind::WorkspaceAdd);
        let (mut sel, mut form) = (0, WorkspaceAddForm::default());
        form.input = "/x/proj".to_owned();
        let effects = overlay_key(&mut store, &mut sel, &mut form, plain(KeyCode::Enter));
        assert_eq!(
            effects,
            vec![Effect::AddWorkspace {
                name: "proj-2".to_owned(),
                dir: "/x/proj".to_owned(),
            }]
        );
    }

    // ── p10 views: d/D strip notices (design.md "detached <label>" /
    // "moved <label> → <view>" — kept stable for e2e greps) ────────────────

    #[test]
    fn d_detaching_shows_the_detached_notice() {
        let mut store = store_with(vec![ws("a")], vec![si("a", "one"), si("a", "two")]);
        store.focus_session(&id("a", "two"));
        let mut router = GarageRouter::default();
        key_at(&mut router, &mut store, "d", 1000);
        assert_eq!(router.notices.current(1001), Some("detached two"));
    }

    #[test]
    fn d_rejoining_shows_no_notice() {
        let mut store = store_with(vec![ws("a")], vec![si("a", "one"), si("a", "two")]);
        store.focus_session(&id("a", "two"));
        let mut router = GarageRouter::default();
        key_at(&mut router, &mut store, "d", 1000); // detach: notice shown, ttl 2000ms
        key_at(&mut router, &mut store, "d", 3500); // rejoin, well after that notice expired
        assert_eq!(router.notices.current(3600), None, "a rejoin raises no notice of its own");
    }

    #[test]
    fn d_with_no_focused_session_is_a_silent_no_op() {
        let mut store = store_with(vec![ws("a")], vec![]);
        let mut router = GarageRouter::default();
        key_at(&mut router, &mut store, "d", 1000);
        assert_eq!(router.notices.current(1001), None);
    }

    #[test]
    fn shift_d_opens_the_picker_overlay() {
        let mut store = store_with(vec![ws("a")], vec![si("a", "one")]);
        let mut router = GarageRouter::default();
        key_at(&mut router, &mut store, "D", 1000);
        assert_eq!(store.state().overlay, Some(OverlayKind::ViewPicker));
    }

    #[test]
    fn picker_entries_exclude_the_focused_sessions_current_view() {
        // Exactly the reported repro shape: two sessions, `d` detaches the
        // focused one into its own solo view, then `D` must never offer that
        // solo view as a destination for itself.
        let mut store = store_with(vec![ws("a")], vec![si("a", "one"), si("a", "two")]);
        store.focus_session(&id("a", "one"));
        store.detach_focused(); // "one" -> its own solo view "one"; "two" stays in "main"
        assert_eq!(
            view_picker_entries(&store),
            ["main", "new group…"],
            "not [\"main\", \"one\", \"new group…\"]"
        );

        // Cross-check the other side of the same pair: focused back on
        // "main" (holding "two"), "main" itself must now be the excluded one.
        store.focus_view_of(&id("a", "two"));
        assert_eq!(view_picker_entries(&store), ["one", "new group…"]);
    }

    #[test]
    fn picker_enter_on_an_existing_view_moves_and_shows_the_moved_notice() {
        let mut store =
            store_with(vec![ws("a")], vec![si("a", "one"), si("a", "two"), si("a", "three")]);
        store.focus_session(&id("a", "two"));
        store.detach_focused(); // "two" -> solo view "two"
        store.focus_view_of(&id("a", "three"));
        store.open_view_picker();

        // Focused session "three" sits in "main" — its own current view, so
        // "main" is excluded. Entries are ["two", "new group…"]; select
        // "two" (index 0).
        let mut view_picker = ViewPickerState { selected: 0, ..Default::default() };
        let notice =
            handle_view_picker_key(&mut store, &plain(KeyCode::Enter), None, &mut view_picker);
        assert_eq!(notice.as_deref(), Some("moved three → two"));
        assert_eq!(store.state().overlay, None, "picker closes on selection");
        let view = store
            .state()
            .views_for("a")
            .into_iter()
            .find(|v| v.name == "two")
            .unwrap();
        assert_eq!(view.session_ids, [id("a", "two"), id("a", "three")]);
    }

    #[test]
    fn picker_new_group_flow_creates_a_named_group_and_shows_the_moved_notice() {
        let mut store = store_with(vec![ws("a")], vec![si("a", "one"), si("a", "two")]);
        store.focus_session(&id("a", "one"));
        store.open_view_picker();

        // "one" and "two" are both still in "main" — the focused session's
        // own current view, so it's excluded, leaving just ["new group…"].
        let mut view_picker = ViewPickerState { selected: 0, ..Default::default() };
        assert_eq!(
            handle_view_picker_key(&mut store, &plain(KeyCode::Enter), None, &mut view_picker),
            None,
            "switches to the text-input sub-mode; nothing moved yet"
        );
        assert!(view_picker.new_group_input.is_some());

        for c in "backend".chars() {
            handle_view_picker_key(
                &mut store,
                &KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE),
                Some(c),
                &mut view_picker,
            );
        }
        let notice =
            handle_view_picker_key(&mut store, &plain(KeyCode::Enter), None, &mut view_picker);
        assert_eq!(notice.as_deref(), Some("moved one → backend"));
        assert_eq!(store.state().focused_view_name("a"), "backend");
    }

    #[test]
    fn picker_esc_cancels_from_either_mode_without_moving_anything() {
        let mut store = store_with(vec![ws("a")], vec![si("a", "one"), si("a", "two")]);
        store.focus_session(&id("a", "one"));
        store.open_view_picker();
        let mut view_picker = ViewPickerState::default();

        handle_view_picker_key(&mut store, &plain(KeyCode::Esc), None, &mut view_picker);
        assert_eq!(store.state().overlay, None);
        assert_eq!(store.state().views_for("a").len(), 1, "nothing moved");

        store.open_view_picker();
        view_picker.new_group_input = Some("partial".to_owned());
        handle_view_picker_key(&mut store, &plain(KeyCode::Esc), None, &mut view_picker);
        assert_eq!(store.state().overlay, None);
        assert!(view_picker.new_group_input.is_none());
    }

    // ── p17: worktree finish overlay (spec tui-worktree-finish) ─────────

    fn wt_record(branch: &str) -> WorktreeRecord {
        WorktreeRecord {
            path: format!("/w/{branch}"),
            branch: branch.to_owned(),
            repo_dir: "/r".to_owned(),
            target: Some("main".to_owned()),
        }
    }

    fn finish_open(store: &mut WallStore, branch: &str) -> WorktreeFinishState {
        let mut finish = WorktreeFinishState::default();
        assert!(!show_worktree_finish(store, &mut finish, wt_record(branch)));
        assert_eq!(store.state().overlay, Some(OverlayKind::WorktreeFinish));
        finish
    }

    fn finish_key(
        store: &mut WallStore,
        finish: &mut WorktreeFinishState,
        c: char,
        now_ms: i64,
    ) -> Vec<Effect> {
        handle_worktree_finish_key(store, &plain(KeyCode::Char(c)), finish, now_ms)
    }

    #[test]
    fn worktree_close_opens_the_finish_overlay_even_from_engaged_or_another_overlay() {
        let mut store = store_with(vec![ws("a")], vec![si("a", "one")]);
        assert!(store.engage());
        let mut finish = WorktreeFinishState::default();
        assert!(!show_worktree_finish(&mut store, &mut finish, wt_record("garage/x")));
        assert_eq!(store.state().layer, KeyLayer::Overlay);
        assert_eq!(store.state().overlay, Some(OverlayKind::WorktreeFinish));

        let mut store = store_with(vec![ws("a")], vec![si("a", "one")]);
        store.open_overlay(OverlayKind::WorkspaceAdd);
        let mut finish = WorktreeFinishState::default();
        assert!(show_worktree_finish(&mut store, &mut finish, wt_record("garage/x")), "displaced");
        assert_eq!(store.state().overlay, Some(OverlayKind::WorktreeFinish));
    }

    #[test]
    fn a_second_worktree_close_queues_behind_the_first() {
        let mut store = store_with(vec![ws("a")], vec![si("a", "one")]);
        let mut finish = finish_open(&mut store, "garage/a");
        assert!(!show_worktree_finish(&mut store, &mut finish, wt_record("garage/b")));
        assert_eq!(finish.current().unwrap().branch, "garage/a", "first stays on screen");
        assert_eq!(finish_key(&mut store, &mut finish, 'k', 1000), vec![]);
        assert_eq!(store.state().overlay, Some(OverlayKind::WorktreeFinish), "b still waits");
        assert_eq!(finish.current().unwrap().branch, "garage/b");
        assert_eq!(finish_key(&mut store, &mut finish, 'k', 1100), vec![]);
        assert_eq!(store.state().overlay, None);
    }

    #[test]
    fn m_merges_once_and_ignores_choices_while_busy() {
        let mut store = store_with(vec![ws("a")], vec![si("a", "one")]);
        let mut finish = finish_open(&mut store, "garage/x");
        assert_eq!(
            finish_key(&mut store, &mut finish, 'm', 1000),
            vec![Effect::FinishWorktree {
                record: wt_record("garage/x"),
                action: FinishAction::Merge,
            }]
        );
        for c in ['m', 'd', 'k'] {
            assert_eq!(finish_key(&mut store, &mut finish, c, 1100), vec![], "{c} while busy");
        }
        assert_eq!(
            handle_worktree_finish_key(&mut store, &plain(KeyCode::Esc), &mut finish, 1100),
            vec![]
        );
        assert_eq!(store.state().overlay, Some(OverlayKind::WorktreeFinish), "still open");
    }

    #[test]
    fn d_arms_and_a_second_d_discards_any_other_key_disarms() {
        let mut store = store_with(vec![ws("a")], vec![si("a", "one")]);
        let mut finish = finish_open(&mut store, "garage/x");
        assert_eq!(finish_key(&mut store, &mut finish, 'd', 1000), vec![], "first d arms");
        assert!(finish.discard_armed(1001));
        // An unbound key is consumed (modal) and disarms.
        assert_eq!(finish_key(&mut store, &mut finish, 'z', 1100), vec![]);
        assert!(!finish.discard_armed(1101));
        assert_eq!(finish_key(&mut store, &mut finish, 'd', 1200), vec![], "re-arms");
        assert_eq!(
            finish_key(&mut store, &mut finish, 'd', 1300),
            vec![Effect::FinishWorktree {
                record: wt_record("garage/x"),
                action: FinishAction::Discard,
            }]
        );
    }

    #[test]
    fn keep_and_esc_close_without_any_request() {
        for key in [plain(KeyCode::Char('k')), plain(KeyCode::Esc)] {
            let mut store = store_with(vec![ws("a")], vec![si("a", "one")]);
            let mut finish = finish_open(&mut store, "garage/x");
            assert_eq!(
                handle_worktree_finish_key(&mut store, &key, &mut finish, 1000),
                vec![],
                "keep never sends a finish request"
            );
            assert_eq!(store.state().overlay, None);
            assert_eq!(finish.current(), None);
        }
    }

    #[test]
    fn the_overlay_is_modal_wall_keys_do_nothing() {
        let mut store = store_with(vec![ws("a")], vec![si("a", "one")]);
        let mut finish = finish_open(&mut store, "garage/x");
        for c in ['q', 'x', 'n', '?'] {
            assert_eq!(finish_key(&mut store, &mut finish, c, 1000), vec![]);
        }
        assert_eq!(store.state().overlay, Some(OverlayKind::WorktreeFinish));
        assert_eq!(store.state().keys_target_chip(), "keys → finish worktree");
    }

    #[test]
    fn a_finish_error_keeps_the_overlay_open_with_the_message() {
        let mut store = store_with(vec![ws("a")], vec![si("a", "one")]);
        let mut finish = finish_open(&mut store, "garage/x");
        finish_key(&mut store, &mut finish, 'm', 1000);
        let notice = worktree_finish_settled(
            &mut store,
            &mut finish,
            &wt_record("garage/x"),
            FinishAction::Merge,
            Some("worktree has uncommitted changes".to_owned()),
        );
        assert_eq!(notice, None);
        assert_eq!(store.state().overlay, Some(OverlayKind::WorktreeFinish));
        assert_eq!(finish.error.as_deref(), Some("worktree has uncommitted changes"));
        assert_eq!(finish.busy, None);
        // Choices are live again: keep closes, no request.
        assert_eq!(finish_key(&mut store, &mut finish, 'k', 2000), vec![]);
        assert_eq!(store.state().overlay, None);
    }

    #[test]
    fn a_finish_success_closes_with_a_strip_notice() {
        let mut store = store_with(vec![ws("a")], vec![si("a", "one")]);
        let mut finish = finish_open(&mut store, "garage/x");
        finish_key(&mut store, &mut finish, 'm', 1000);
        let notice = worktree_finish_settled(
            &mut store,
            &mut finish,
            &wt_record("garage/x"),
            FinishAction::Merge,
            None,
        );
        assert_eq!(notice.as_deref(), Some("merged garage/x into main"));
        assert_eq!(store.state().overlay, None);
    }
}
