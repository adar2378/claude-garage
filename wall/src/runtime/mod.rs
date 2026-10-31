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

use std::io::Write as _;
use std::ops::Range;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use crossterm::event::{
    Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers,
};
use ratatui::layout::Rect;
use tokio::sync::mpsc;

use crate::actions::ActionId;
use crate::api::client::{ApiError, GarageClient};
use crate::api::models::{
    FinishAction, HooksInstallResult, SessionInfo, UsageInfo, WorkspaceInfo,
    WorktreeRecord,
};
use crate::input::encode::encode_key;
use crate::input::paste::wrap_bracketed_paste;
use crate::keymap::Keymap;
use crate::slots::{SlotId, SlotRegistry};
use crate::state::persistence::{self, WallFile};
use crate::state::store::WallStore;
use crate::state::wall_state::{KeyLayer, OverlayKind, WallSession};
use crate::ui::escalation::{EscalationPolicy, HEARTBEAT_INTERVAL_MS};
use crate::ui::layout::WallLayout;
use crate::ui::motion::Motion;
use crate::ui::pet::{PetClick, PetHost};
use crate::attach::DaemonResolver;
use crate::ui::registry::{FrozenTile, TileRegistry};
use crate::ui::scroll::{capture_frozen, history_size, page_lines, ScrollModel};
use crate::ui::view_picker::ViewPickerState;
use crate::ui::workspace_add::{PickOutcome, WorkspaceAddForm};
use crate::ui::worktree_finish::WorktreeFinishState;

mod dispatch;
mod draw;
mod effects;
mod mouse;
mod overlays;
mod router;
mod tasks;

pub use effects::Effect;
use effects::*;
pub use overlays::handle_overlay_key;
use overlays::*;
pub use router::{GarageRouter, NoticeKind, Notices, RESTART_PREFIX_HINT, TYPING_HINT};
use tasks::*;

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
        // (the keymap binds `"\t"` to `view.cycle`).
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

/// Everything the state loop owns.
struct App {
    store: WallStore,
    router: GarageRouter,
    /// Key → action table (p22 tui-actions): the router and the overlay key
    /// handlers resolve through this one instance.
    keymap: Keymap,
    registry: TileRegistry,
    /// The slot providers every renderer asks for its items.
    slots: SlotRegistry,
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
    /// Clickable `strip.right` slot items from the last frame (absolute).
    strip_slots: Vec<(Range<u16>, ActionId)>,
    /// View-strip click targets from the last frame (absolute columns paired
    /// with the view name they focus) — empty when the strip isn't shown.
    view_strip_targets: Vec<(Range<u16>, String)>,
    /// Modal rects from the last frame, per open overlay.
    triage_rect: Option<Rect>,
    ws_add_rect: Option<Rect>,
    view_picker_rect: Option<Rect>,

    // ── p15: pit pet (spec tui-pit-pet) ───────────────────────────────────
    /// The pit pet behind its feature boundary; `None` when the `pet`
    /// feature is off.
    pet: Option<PetHost>,
    /// p23: running effects, spinner flags and the redraw clock.
    motion: Motion,
    /// The filler width the last-drawn strip had — the room `pet_tick`'s
    /// next `PetSim::step` has to work with. `0` until the first frame
    /// (falls back to a conservative default — see `pet_tick`).
    last_strip_filler: u16,
}

impl App {
    #[cfg(test)]
    fn new(tx: EventSender, base_url: &str, started_ms: i64) -> App {
        App::with_motion(tx, base_url, started_ms, false)
    }

    fn with_motion(tx: EventSender, base_url: &str, started_ms: i64, motion_enabled: bool) -> App {
        let pet = PetHost::new(started_ms);
        let motion = Motion::new(motion_enabled);
        let mut slots = SlotRegistry::core();
        if let Some(provider) = pet.as_ref().and_then(PetHost::slot_provider) {
            slots.register(SlotId::StripRight, provider);
        }
        if let Some(spinner) = motion.strip_spinner() {
            slots.register(SlotId::StripRight, spinner);
        }
        App {
            store: WallStore::new(),
            router: GarageRouter::default(),
            keymap: Keymap::defaults(),
            registry: TileRegistry::with_resolver(tx.clone(), Box::new(DaemonResolver::new(base_url))),
            slots,
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
            strip_slots: Vec::new(),
            view_strip_targets: Vec::new(),
            triage_rect: None,
            ws_add_rect: None,
            view_picker_rect: None,
            pet,
            motion,
            last_strip_filler: 0,
        }
    }

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
            AppEvent::Connection(live) => {
                self.motion.connection_changed(live, now_ms());
                self.store.connection(live);
            }
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
            AppEvent::Term(Event::Mouse(mouse)) => {
                if self.handle_mouse(&mouse) {
                    return true;
                }
            }
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
                let home = std::env::var("HOME").or_else(|_| std::env::var("USERPROFILE")).unwrap_or_default();
                let (effects, notice) = handle_overlay_key(
                    &mut self.store,
                    &self.keymap,
                    key,
                    OverlayRefs {
                        queue_selection: &mut self.queue_selection,
                        form: &mut self.ws_form,
                        view_picker: &mut self.view_picker,
                    },
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
                let outcome = self
                    .router
                    .on_garage_key(&mut self.store, &self.keymap, &key_str, now_ms());
                let mut effects = outcome.effects;
                if let Some((id, ctx)) = outcome.action {
                    effects.extend(self.dispatch_action(id, &ctx));
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
                if matches!(effect, Effect::RestartDaemon) {
                    self.motion.daemon_restart_requested(now_ms());
                }
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
                let Some(spec) = self.registry.spec(id).cloned() else {
                    return;
                };
                let Some(hist) = history_size(&spec, id) else {
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
        let Some(spec) = self.registry.spec(id).cloned() else {
            return;
        };
        if let Some(parser) = capture_frozen(&spec, id, model.top_abs(), cols, rows) {
            let new_lines = history_size(&spec, id).map_or(0, |h| model.new_lines(h));
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

    /// A click on the pet's strip columns (spec tui-pit-pet "Click
    /// routing"): while alert, the same jump `a` performs; otherwise a
    /// petting interaction (the host's job).
    fn handle_pet_click(&mut self, now: i64) {
        if self.store.state().layer == KeyLayer::Engaged {
            self.store.disengage();
        }
        let Some(pet) = &mut self.pet else { return };
        if pet.click(now, self.store.state(), &mut self.router.notices) == PetClick::JumpToBlocked
            && !self.store.jump_to_longest_waiting()
        {
            // Same wording as the `a` key's decline (spec tui-triage).
            self.router.notices.show("no session needs you", 2000, now);
        }
    }

    // ── p15: pit pet tick (spec tui-pit-pet) ──────────────────────────────

    /// The 300 ms pet tick; returns whether the frame needs a redraw.
    fn pet_tick(&mut self, now: i64) -> bool {
        match &mut self.pet {
            Some(pet) => pet.tick(
                now,
                self.store.state(),
                &mut self.router.notices,
                self.last_strip_filler,
            ),
            None => false,
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

/// Idle poll when nothing is dirty and nothing animates.
const IDLE_POLL: Duration = Duration::from_millis(250);

/// How long the loop waits for the next event. Dirty: until the frame cap.
/// Clean: the idle poll, shortened to the motion tick only while something
/// animates (`motion_tick` is `None` otherwise, leaving today's behaviour).
fn loop_timeout(dirty: bool, since_render: Duration, motion_tick: Option<Duration>) -> Duration {
    if dirty {
        return FRAME_CAP.saturating_sub(since_render);
    }
    match motion_tick {
        Some(tick) => IDLE_POLL.min(tick.saturating_sub(since_render)),
        None => IDLE_POLL,
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
    let started_ms = now_ms();
    let mut app = App::with_motion(tx, base_url, started_ms, crate::ui::motion::load_enabled());
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
        // Motion (p23): the only timed redraw. `next_tick` is `None` while
        // nothing animates, so an idle wall never takes this branch.
        if app.motion.next_tick().is_some_and(|tick| last_render.elapsed() >= tick) {
            dirty = true;
        }
        if dirty && last_render.elapsed() >= FRAME_CAP {
            terminal.draw(|f| app.draw_frame(f, now_ms()))?;
            last_render = Instant::now();
            dirty = false;
        }
        let timeout = loop_timeout(dirty, last_render.elapsed(), app.motion.next_tick());
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
mod tests;
