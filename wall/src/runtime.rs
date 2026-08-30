//! Runtime: tokio; PTY readers and the SSE stream as tasks feeding ONE mpsc
//! event channel; a single state-owning loop applies events and draws —
//! "daemon is authoritative, render is pure", same shape as p8 (task 1.3).
//!
//! Wave 3 (task 3.3) adds the full three-layer key dispatch (spec
//! tui-key-routing):
//!   - garage layer: single-key app commands through the ported
//!     [`GarageCommand`] mapping, incl. every p8.1–p8.4 lifecycle key
//!     (`1-9 [ ] a A n N x X K w m R Enter ? q`), the armed `x`/`X`/`K`
//!     double-press flows and their exact Dart strip-notice wording;
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
use serde_json::Value;
use tokio::sync::mpsc;

use crate::api::client::{ApiError, GarageClient};
use crate::api::models::{SessionInfo, WorkspaceInfo};
use crate::api::sse::SseClient;
use crate::input::encode::encode_key;
use crate::input::paste::wrap_bracketed_paste;
use crate::state::armed_action::{ArmedAction, ArmedClose};
use crate::state::salience::restorable_session_ids;
use crate::state::store::{garage_command_for, GarageCommand, WallStore};
use crate::state::wall_state::{KeyLayer, OverlayKind, WallSession, WallState};
use crate::state::workspace_remove::{confirm_kill_target, kill_remove_notice, remove_arm_notice};
use crate::ui::escalation::{EscalationPolicy, HEARTBEAT_INTERVAL_MS};
use crate::ui::help::render_help;
use crate::ui::hit_targets::{rail_target_at, tile_index_at, triage_row_index_at, RailTarget};
use crate::ui::layout::{tile_inner, wall_layout, WallLayout};
use crate::ui::rail::render_rail;
use crate::ui::registry::{FrozenTile, TileRegistry};
use crate::ui::scroll::{capture_frozen, page_lines, tmux_history_size, ScrollModel, WHEEL_LINES};
use crate::ui::strip::render_strip;
use crate::ui::theme::colors;
use crate::ui::tile::{render_centered_lines, render_tile, TileView};
use crate::ui::triage::{render_triage, triage_queue_rows, wrap_selection};
use crate::ui::workspace_add::{render_workspace_add, WorkspaceAddForm};

const FRAME_CAP: Duration = Duration::from_millis(33); // ~30fps render cap (not spec-mandated)

/// Elapsed timers and the done-fade must advance without input; 10s keeps
/// the "refresh ≤ 30s" contract with margin at negligible render cost.
const ELAPSED_TICK: Duration = Duration::from_secs(10);

/// Throttle for the frozen `+N lines` counter refresh (one tmux call per
/// frozen tile).
const FROZEN_COUNT_REFRESH: Duration = Duration::from_secs(1);

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
    /// SSE `status` event.
    Status {
        id: String,
        status: String,
        since: Option<i64>,
    },
    /// A transient strip notice (fetch/effect errors, lifecycle results).
    Notice { text: String, ttl_ms: i64 },
    /// A spawn effect settled (success or failure) — clears the busy guard.
    SpawnSettled,
    /// A restore effect settled for this session id.
    RestoreSettled(String),
    /// The add-workspace PUT settled: `error: None` closes the overlay and
    /// focuses the new workspace; `Some` keeps it open with the inline error.
    WorkspaceAddSettled {
        name: String,
        error: Option<String>,
    },
    Quit,
}

pub type EventSender = mpsc::UnboundedSender<AppEvent>;

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

// ── Strip notices ─────────────────────────────────────────────────────────

/// The garage-layer typing hint (post-review Dart wording, verbatim — the
/// click-smoke harness greps it).
pub const TYPING_HINT: &str = "enter engages the focused terminal — keys go to garage now";

/// Transient strip notice with a TTL, plus the Dart TUI's two behaviors the
/// armed flows depend on: prefix-clearing (disarming clears its own arm
/// notice, never an unrelated one) and the rate-limited typing hint (a burst
/// of typing does not restart the hint's timer). Time is injected (ms) so it
/// unit-tests without timers.
#[derive(Default)]
pub struct Notices {
    text: Option<String>,
    expires_at_ms: i64,
}

impl Notices {
    pub fn show(&mut self, text: impl Into<String>, ttl_ms: i64, now_ms: i64) {
        self.text = Some(text.into());
        self.expires_at_ms = now_ms + ttl_ms;
    }

    /// Garage-layer typing hint, rate-limited: while the hint is already up,
    /// further keypresses in the burst do NOT restart the timer.
    pub fn show_typing_hint(&mut self, now_ms: i64) {
        if self.text.as_deref() == Some(TYPING_HINT) && now_ms < self.expires_at_ms {
            return;
        }
        self.show(TYPING_HINT, 2500, now_ms);
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
    /// `q`: orderly quit.
    Quit,
}

// ── Garage-layer router ──────────────────────────────────────────────────

/// First free `claude-N` label in the workspace (the web UI's default label
/// family — port of the Dart `_nextLabel`).
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
/// p8.1–p8.4). Pure — state transitions go through the store, IO becomes
/// returned [`Effect`]s, notices land in [`Notices`] — so the armed
/// `x`/`X`/`K` flows and their exact strip wording unit-test without a
/// daemon. Mirrors the Dart `_onGarageKey` ordering exactly.
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
        let Some(command) = garage_command_for(key) else {
            // Unbound garage keys are consumed (never reach an agent), but
            // silence reads as a dead wall — show where the keys actually go.
            if is_printable(key) {
                self.notices.show_typing_hint(now_ms);
            }
            return effects;
        };
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
    /// parallel per-id calls (the web UI's restore-all shape), so one
    /// failure never blocks the rest.
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
}

// ── Key-event reduction ──────────────────────────────────────────────────

/// Reduce a crossterm key event to the garage layer's string form (the
/// Dart handler's `event.character`, Enter normalized to `"\r"`).
/// `None` = nothing typeable (consumed silently — Ctrl/Alt-modified keys,
/// Esc, arrows … are all unbound in the garage layer).
pub fn garage_key_string(key: &KeyEvent) -> Option<String> {
    match key.code {
        KeyCode::Enter => Some("\r".to_owned()),
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
/// `dir_exists`, so this routes pure in tests), Esc cancels and resets the
/// form. Returned effects are the caller's IO (the PUT).
pub fn handle_overlay_key(
    store: &mut WallStore,
    key: &KeyEvent,
    queue_selection: &mut usize,
    form: &mut WorkspaceAddForm,
    home: &str,
    dir_exists: &dyn Fn(&str) -> bool,
) -> Vec<Effect> {
    let Some(kind) = store.state().overlay else {
        return Vec::new();
    };
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
                    return vec![Effect::AddWorkspace { name, dir }];
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
    }
    Vec::new()
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
                    Ok(Some(worktree)) => {
                        // v1 worktree policy = keep: the strip says where to
                        // finish it (exact Dart wording — harness greps it).
                        let branch = worktree
                            .get("branch")
                            .and_then(Value::as_str)
                            .map(str::to_owned)
                            .unwrap_or_else(|| format!("garage/{label}"));
                        send_notice(
                            &tx,
                            format!("worktree kept: {branch} — merge or discard in the web wall"),
                            8000,
                        );
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
        sse.run(stop, on_event, on_poll);
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
    /// Rects of the last drawn frame — the mouse router reads the SAME
    /// geometry the paint used. `None` until the first frame.
    layout: Option<WallLayout>,
    /// Strip badge columns from the last frame (absolute), when rendered.
    badge_cols: Option<Range<u16>>,
    /// Modal rects from the last frame, per open overlay.
    triage_rect: Option<Rect>,
    ws_add_rect: Option<Rect>,
}

impl App {
    /// Apply one event; true = quit.
    fn apply(&mut self, ev: AppEvent) -> bool {
        match ev {
            AppEvent::Quit => return true,
            AppEvent::TileOutput => {}
            AppEvent::Workspaces(w) => self.store.workspaces_fetched(w),
            AppEvent::Sessions(s) => self.store.sessions_fetched(s),
            AppEvent::Status { id, status, since } => {
                self.store.status_changed(&id, &status, since);
            }
            AppEvent::Notice { text, ttl_ms } => {
                self.router.notices.show(text, ttl_ms, now_ms());
            }
            AppEvent::SpawnSettled => self.router.spawn_settled(),
            AppEvent::RestoreSettled(id) => self.router.restore_settled(&id),
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
            AppEvent::Term(Event::Resize(_, _)) => {} // layout recomputes per frame
            AppEvent::Term(Event::Paste(text)) => self.handle_paste(&text),
            AppEvent::Term(Event::Mouse(mouse)) => self.handle_mouse(&mouse),
            AppEvent::Term(Event::Key(key)) => {
                if key.kind != KeyEventKind::Release && self.handle_key(&key) {
                    return true;
                }
            }
            AppEvent::Term(_) => {}
        }
        false
    }

    /// Bracketed paste (crossterm `Event::Paste`): while engaged, forward to
    /// the focused tile wrapped in the paste guards (spec: "Paste
    /// forwarding"), snapping a frozen view live first like any PTY write.
    /// In the garage/overlay layers pastes are consumed — pasted text must
    /// never fire app commands.
    fn handle_paste(&mut self, text: &str) {
        if self.store.state().layer != KeyLayer::Engaged {
            return;
        }
        if let Some(id) = self.store.state().focused_session_id.clone() {
            self.registry.set_frozen(&id, None);
            self.registry.write(&id, &wrap_bracketed_paste(text));
        }
    }

    /// The three-layer dispatch; true = quit.
    fn handle_key(&mut self, key: &KeyEvent) -> bool {
        match self.store.state().layer {
            KeyLayer::Overlay => {
                self.keylog
                    .log("overlay", &format!("{:?} {:?}", key.code, key.modifiers));
                let home = std::env::var("HOME").unwrap_or_default();
                let effects = handle_overlay_key(
                    &mut self.store,
                    key,
                    &mut self.queue_selection,
                    &mut self.ws_form,
                    &home,
                    &|path| std::path::Path::new(path).is_dir(),
                );
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
                let effects = self
                    .router
                    .on_garage_key(&mut self.store, &key_str, now_ms());
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
            }
            return;
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
        // Strip badge click: open the triage queue (the `A` binding).
        if row >= layout.strip.y && self.badge_cols.as_ref().is_some_and(|r| r.contains(&col)) {
            if self.store.state().layer == KeyLayer::Engaged {
                self.store.disengage();
            }
            self.queue_selection = 0;
            self.store.open_overlay(OverlayKind::TriageQueue);
        }
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
        let layout = wall_layout(area, ids.len(), maximized_index);

        // PTY size tracks the tile (TIOCSWINSZ) on every layout pass —
        // startup, grid reshape, maximize both ways, terminal resize.
        for (i, id) in ids.iter().enumerate() {
            let (cols, rows) = tile_inner(layout.tiles[i]);
            self.registry.apply_size(id, cols, rows);
        }

        let buf = frame.buffer_mut();
        render_rail(buf, layout.rail, self.store.state(), now_ms);

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

        self.badge_cols = render_strip(
            buf,
            layout.strip,
            self.store.state(),
            self.router.notices.current(now_ms),
        );

        self.triage_rect = None;
        self.ws_add_rect = None;
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
        layout: None,
        badge_cols: None,
        triage_rect: None,
        ws_add_rect: None,
    };
    let mut dirty = true;
    let mut last_render = Instant::now() - FRAME_CAP;
    let mut last_elapsed_tick = Instant::now();
    let mut last_heartbeat = Instant::now();

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
        // Reattach retries / staggered attaches fire from the tick too, so
        // they never wait on an input event.
        if app.registry.tick(Instant::now()) {
            dirty = true;
        }
        if app.refresh_frozen_counts() {
            dirty = true;
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
            Ok(None) => return Ok(()),
            Ok(Some(ev)) => ev,
        };
        // Drain everything pending; handle in arrival order.
        let mut events = vec![first];
        while let Ok(ev) = rx.try_recv() {
            events.push(ev);
        }
        for ev in events {
            dirty = true;
            if app.apply(ev) {
                return Ok(());
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
            dir: Some(format!("/repos/{name}")),
            branch: None,
        }
    }

    fn si(workspace: &str, label: &str) -> SessionInfo {
        SessionInfo {
            id: format!("garage/{workspace}/{label}"),
            workspace: workspace.to_owned(),
            label: label.to_owned(),
            dir: Some(format!("/repos/{workspace}")),
            attached: false,
            status: "working".to_owned(),
            since: Some(1000),
            message: None,
            branch: None,
            restorable: false,
        }
    }

    fn si_blocked(workspace: &str, label: &str, since: i64) -> SessionInfo {
        SessionInfo {
            status: "needs-input".to_owned(),
            since: Some(since),
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
        handle_overlay_key(store, &key, selection, form, "/home/me", &|_| true)
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
        let effects =
            handle_overlay_key(&mut store, &plain(KeyCode::Enter), &mut sel, &mut form, "/h", &|_| false);
        assert!(effects.is_empty());
        assert_eq!(form.error.as_deref(), Some("no such directory: /nope"));
        assert_eq!(store.state().overlay, Some(OverlayKind::WorkspaceAdd));
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
}
