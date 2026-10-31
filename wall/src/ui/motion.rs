//! Motion (p23, spec tui-motion): the one place that decides whether the
//! wall animates, owns the running effects, and says when the loop needs a
//! timed redraw.
//!
//! Rules this module enforces:
//! - No timer without work: [`Motion::next_tick`] is `None` unless an effect
//!   or a visible spinner exists, so an idle wall stays event-driven.
//! - Effects only change how a frame is drawn. Key handling and PTY writes
//!   run before rendering and never look at this module.
//! - Amber is reserved: the only amber effect is the single [`FxKind::Pulse`]
//!   on a tile that just became `needs-input`.
//! - With motion off, every method is a no-op and a frame equals the
//!   unanimated rendering. A finished effect is removed before it is
//!   processed again, so the settled frame is untouched by tachyonfx.
//!
//! The 2-minute `done` fade is NOT an effect (a 120 s tachyonfx timer would
//! redraw ~3600 times per session): it is a pure colour step evaluated at
//! render time, advanced by the loop's existing 10 s elapsed tick.

use std::cell::Cell;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;
use std::time::Duration;

use ratatui::buffer::Buffer;
use ratatui::layout::{Margin, Rect};
use ratatui::style::{Color, Style};
use ratatui::text::Span;
use tachyonfx::{fx, CellFilter, Effect, Interpolation};
use throbber_widgets_tui::{Throbber, ThrobberState, BRAILLE_SIX};

use crate::slots::{SlotCtx, SlotItem, SlotProvider};
use crate::ui::theme::{colors, DONE_FADE_MS};

/// Redraw cadence while a tachyonfx effect runs (~30 fps).
pub const FX_TICK: Duration = Duration::from_millis(33);
/// Redraw cadence while only spinners are visible (one braille step).
pub const SPINNER_TICK: Duration = Duration::from_millis(100);
/// Longest animation step applied in one frame; a wake after idle must not
/// skip an effect to its end.
const MAX_STEP_MS: i64 = 100;
/// How long a daemon-restart request keeps the strip spinner up if the SSE
/// stream never reports a drop.
const RESTART_GRACE_MS: i64 = 15_000;
/// How long the strip spinner runs after the stream drops. Past it the strip
/// shows a static "daemon down" and the wall stops ticking: a dead daemon
/// must not keep the CPU awake forever.
const DOWN_SPIN_MS: i64 = 15_000;

/// Fade-in on attach.
const FADE_IN_MS: u32 = 300;
/// The single `needs-input` attention pulse.
const PULSE_MS: u32 = 600;
/// Closed-session dissolve.
const DISSOLVE_MS: u32 = 350;

// ── Off switch ───────────────────────────────────────────────────────────

/// Env values that count as "set": anything but empty / `0` / `false` /
/// `no` / `off`.
fn env_flag(value: &str) -> bool {
    let v = value.trim().to_ascii_lowercase();
    !(v.is_empty() || matches!(v.as_str(), "0" | "false" | "no" | "off"))
}

/// The `animations` key of a `config.toml` body: a single hand-parsed
/// `animations = true|false` line (no TOML crate for one key).
pub fn config_animations(text: &str) -> Option<bool> {
    let mut found = None;
    for line in text.lines() {
        let line = line.split('#').next().unwrap_or("").trim();
        let Some((key, value)) = line.split_once('=') else { continue };
        if key.trim() == "animations" {
            found = match value.trim().trim_matches(|c| c == '"' || c == '\'') {
                "true" => Some(true),
                "false" => Some(false),
                _ => found,
            };
        }
    }
    found
}

/// Whether the wall may animate: off when any of `GARAGE_NO_MOTION`,
/// `NO_MOTION`, `REDUCE_MOTION` is set, or the config says
/// `animations = false`. Pure; [`load_enabled`] feeds it the real world.
pub fn motion_enabled(config: Option<&str>, env: &dyn Fn(&str) -> Option<String>) -> bool {
    let env_off = ["GARAGE_NO_MOTION", "NO_MOTION", "REDUCE_MOTION"]
        .iter()
        .any(|name| env(name).is_some_and(|v| env_flag(&v)));
    if env_off {
        return false;
    }
    config.and_then(config_animations).unwrap_or(true)
}

/// [`motion_enabled`] against the process environment and
/// `$GARAGE_DIR/config.toml` (else `~/.garage/config.toml`).
pub fn load_enabled() -> bool {
    let config = std::fs::read_to_string(crate::state::persistence::garage_dir().join("config.toml")).ok();
    motion_enabled(config.as_deref(), &|name| std::env::var(name).ok())
}

// ── Spinner ──────────────────────────────────────────────────────────────

/// One braille throbber frame for `now_ms` (the throbber crate's
/// `BRAILLE_SIX` set; a single cell wide).
pub fn spinner_span(now_ms: i64) -> Span<'static> {
    let steps = BRAILLE_SIX.symbols.len() as i64;
    let frame = (now_ms.max(0) / SPINNER_TICK.as_millis() as i64) % steps;
    let mut state = ThrobberState::default();
    // `calc_step(0)` means "random" in the crate: count from 1 (an index
    // past the end wraps, so frame 5 -> step 6 -> symbol 0).
    state.calc_step(frame as i8 + 1);
    let span = Throbber::default()
        .throbber_set(BRAILLE_SIX)
        .to_symbol_span(&state);
    // The crate pads the symbol with a trailing space; callers add their own.
    Span::raw(span.content.trim_end().to_owned())
}

/// The spinner glyph as plain text.
pub fn spinner_symbol(now_ms: i64) -> String {
    spinner_span(now_ms).content.into_owned()
}

// ── Done fade ────────────────────────────────────────────────────────────

/// Steps in the `done` fade; with the 10 s elapsed tick this is one step per
/// tick over the 2 minutes.
pub const DONE_FADE_STEPS: i64 = 12;
const DONE_GREEN: (u8, u8, u8) = (0, 190, 0);
const DONE_NEUTRAL: (u8, u8, u8) = (100, 100, 100);

/// The `done` glyph colour `elapsed_ms` after the transition: exactly
/// [`colors::GREEN`] at the start, exactly [`colors::FAINT`] once the 2
/// minutes are up, stepped green-to-neutral between. Never amber.
pub fn done_fade_color(since_ms: Option<i64>, now_ms: i64) -> Color {
    let Some(since) = since_ms else { return colors::GREEN };
    let step = ((now_ms - since).max(0) * DONE_FADE_STEPS) / DONE_FADE_MS;
    if step <= 0 {
        return colors::GREEN;
    }
    if step >= DONE_FADE_STEPS {
        return colors::FAINT;
    }
    let lerp = |a: u8, b: u8| (a as i64 + (b as i64 - a as i64) * step / DONE_FADE_STEPS) as u8;
    Color::Rgb(
        lerp(DONE_GREEN.0, DONE_NEUTRAL.0),
        lerp(DONE_GREEN.1, DONE_NEUTRAL.1),
        lerp(DONE_GREEN.2, DONE_NEUTRAL.2),
    )
}

// ── Effects ──────────────────────────────────────────────────────────────

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FxKind {
    /// A tile's terminal appearing (attach): its text brightens in.
    FadeIn,
    /// The one attention pulse on a tile that just became `needs-input`.
    Pulse,
}

struct TileFx {
    id: String,
    kind: FxKind,
    rect: Rect,
    effect: Effect,
    /// Drawn this frame (a tile that is not drawn drops its effects).
    seen: bool,
    /// Created this frame: its first step is zero-length, so the idle gap
    /// before it does not eat the animation.
    fresh: bool,
}

/// A closed session's last frame, dissolving away over the new layout.
struct Ghost {
    rect: Rect,
    snapshot: Buffer,
    scratch: Buffer,
    effect: Effect,
    fresh: bool,
}

fn fx_duration(ms: i64) -> tachyonfx::Duration {
    tachyonfx::Duration::from_millis(ms.clamp(0, MAX_STEP_MS) as _)
}

fn new_effect(kind: FxKind) -> Effect {
    match kind {
        FxKind::FadeIn => fx::fade_from_fg(Color::Rgb(40, 40, 40), (FADE_IN_MS, Interpolation::QuadOut)),
        // A light flash that settles into the border's own amber; only the
        // frame (border + title row) is touched, never the terminal body.
        FxKind::Pulse => fx::fade_from_fg(Color::Rgb(255, 236, 179), (PULSE_MS, Interpolation::SineOut))
            .with_filter(CellFilter::Outer(Margin::new(1, 1))),
    }
}

/// Running effects, the spinner flags and the redraw clock.
pub struct Motion {
    enabled: bool,
    effects: Vec<TileFx>,
    ghosts: Vec<Ghost>,
    /// Ids whose terminal is attached (fade-in fires on the edge).
    attached: HashSet<String>,
    /// Ids that have faded in once this run: a workspace/view switch
    /// re-attaches a tile without fading it in again.
    faded_in: HashSet<String>,
    /// Last seen `needs-input` per drawn tile (pulse fires on false → true).
    blocked: HashMap<String, bool>,
    /// The last frame's clean tiles (before effects and overlays).
    prev_buf: Option<Buffer>,
    prev_tiles: Vec<(String, Rect)>,
    drawn: Vec<(String, Rect)>,
    last_ms: Option<i64>,
    step_ms: i64,
    spinners: bool,
    /// Shared with the strip's spinner provider.
    clock: Rc<Cell<i64>>,
    restarting_since: Rc<Cell<Option<i64>>>,
    /// When the SSE stream last dropped (shared with the strip provider).
    down_since: Rc<Cell<Option<i64>>>,
    saw_down: bool,
}

impl Motion {
    pub fn new(enabled: bool) -> Motion {
        Motion {
            enabled,
            effects: Vec::new(),
            ghosts: Vec::new(),
            attached: HashSet::new(),
            faded_in: HashSet::new(),
            blocked: HashMap::new(),
            prev_buf: None,
            prev_tiles: Vec::new(),
            drawn: Vec::new(),
            last_ms: None,
            step_ms: 0,
            spinners: false,
            clock: Rc::new(Cell::new(0)),
            restarting_since: Rc::new(Cell::new(None)),
            down_since: Rc::new(Cell::new(None)),
            saw_down: false,
        }
    }

    pub fn enabled(&self) -> bool {
        self.enabled
    }

    /// When the loop must wake to redraw, or `None` when nothing animates.
    pub fn next_tick(&self) -> Option<Duration> {
        if !self.enabled {
            return None;
        }
        if !self.effects.is_empty() || !self.ghosts.is_empty() {
            Some(FX_TICK)
        } else if self.spinners {
            Some(SPINNER_TICK)
        } else {
            None
        }
    }

    /// True while any effect (not spinner) is alive.
    pub fn effects_running(&self) -> bool {
        !self.effects.is_empty() || !self.ghosts.is_empty()
    }

    /// The spinner glyph for tiles, or `None` when motion is off (the static
    /// `…` / `⟳` markers stay).
    pub fn tile_spinner(&self, now_ms: i64) -> Option<String> {
        self.enabled.then(|| spinner_symbol(now_ms))
    }

    /// Frame start: advance the clock and clear per-frame marks.
    pub fn begin_frame(&mut self, now_ms: i64) {
        if !self.enabled {
            return;
        }
        self.step_ms = self.last_ms.map_or(0, |last| (now_ms - last).clamp(0, MAX_STEP_MS));
        self.last_ms = Some(now_ms);
        self.clock.set(now_ms);
        self.spinners = false;
        self.drawn.clear();
        for e in &mut self.effects {
            e.seen = false;
        }
    }

    /// A tile with a visible spinner was drawn / the strip spinner is up.
    pub fn spinner_shown(&mut self) {
        if self.enabled {
            self.spinners = true;
        }
    }

    /// Tiles drawn last frame whose session no longer exists (`exists`
    /// says false): closed, not merely scrolled off. Starts their dissolve
    /// from the last clean frame.
    pub fn closed(&mut self, exists: impl Fn(&str) -> bool) {
        if !self.enabled {
            return;
        }
        let Some(prev) = &self.prev_buf else { return };
        let gone: Vec<(String, Rect)> = self
            .prev_tiles
            .iter()
            .filter(|(id, _)| !exists(id))
            .cloned()
            .collect();
        for (id, rect) in gone {
            self.attached.remove(&id);
            self.faded_in.remove(&id);
            self.blocked.remove(&id);
            self.effects.retain(|e| e.id != id);
            let rect = rect.intersection(prev.area);
            if rect.is_empty() {
                continue;
            }
            let mut snapshot = Buffer::empty(rect);
            for y in rect.top()..rect.bottom() {
                for x in rect.left()..rect.right() {
                    snapshot[(x, y)] = prev[(x, y)].clone();
                }
            }
            self.ghosts.push(Ghost {
                rect,
                scratch: snapshot.clone(),
                snapshot,
                effect: fx::dissolve((DISSOLVE_MS, Interpolation::QuadIn)),
                fresh: true,
            });
        }
        self.prev_tiles.retain(|(id, _)| exists(id));
    }

    /// One tile was drawn this frame. Starts the fade-in on the
    /// not-attached → attached edge and the pulse on the
    /// not-blocked → blocked edge (never on first sight).
    pub fn tile_drawn(&mut self, id: &str, rect: Rect, attached: bool, blocked: bool) {
        if !self.enabled {
            return;
        }
        self.drawn.push((id.to_owned(), rect));
        if attached {
            if self.attached.insert(id.to_owned()) && self.faded_in.insert(id.to_owned()) {
                self.start(id, rect, FxKind::FadeIn);
            }
        } else {
            self.attached.remove(id);
        }
        if self.blocked.insert(id.to_owned(), blocked) == Some(false) && blocked {
            self.start(id, rect, FxKind::Pulse);
        }
        for e in self.effects.iter_mut().filter(|e| e.id == id) {
            e.seen = true;
            e.rect = rect;
        }
    }

    fn start(&mut self, id: &str, rect: Rect, kind: FxKind) {
        // One effect of a kind per tile.
        self.effects.retain(|e| !(e.id == id && e.kind == kind));
        self.effects.push(TileFx {
            id: id.to_owned(),
            kind,
            rect,
            effect: new_effect(kind),
            seen: true,
            fresh: true,
        });
    }

    /// After the tiles are painted and before the strip and overlays: keep
    /// the clean frame for dissolves, then run the effects over it.
    pub fn apply(&mut self, buf: &mut Buffer) {
        if !self.enabled {
            return;
        }
        // Only the tile area matters, but a whole-buffer clone per draw is
        // cheap and only happens while motion is on.
        self.prev_buf = Some(buf.clone());
        self.prev_tiles = std::mem::take(&mut self.drawn);

        // A finished effect is dropped BEFORE it is processed again, so the
        // frame after the last animated one is the plain rendering.
        self.effects.retain(|e| e.seen && !e.effect.done());
        let step = fx_duration(self.step_ms);
        for e in &mut self.effects {
            let dt = if std::mem::take(&mut e.fresh) { tachyonfx::Duration::ZERO } else { step };
            e.effect.process(dt, buf, e.rect);
        }

        self.ghosts.retain(|g| !g.effect.done());
        for g in &mut self.ghosts {
            let dt = if std::mem::take(&mut g.fresh) { tachyonfx::Duration::ZERO } else { step };
            let area = g.rect;
            g.effect.process(dt, &mut g.scratch, area);
            // Paint what has not dissolved yet; blanks stay transparent so
            // the tiles that moved in underneath show through.
            for y in area.top()..area.bottom() {
                for x in area.left()..area.right() {
                    let kept = &g.scratch[(x, y)];
                    if kept.symbol() == g.snapshot[(x, y)].symbol() && kept.symbol() != " " {
                        buf[(x, y)] = kept.clone();
                    }
                }
            }
        }
    }

    // ── Daemon restart spinner ───────────────────────────────────────────

    /// `Effect::RestartDaemon` was dispatched.
    pub fn daemon_restart_requested(&mut self, now_ms: i64) {
        if self.enabled {
            self.restarting_since.set(Some(now_ms));
            self.saw_down = false;
        }
    }

    /// The SSE connection changed. Tracks when it dropped (for the spinner
    /// cap); a restart is over once it comes back after having dropped.
    pub fn connection_changed(&mut self, live: bool, now_ms: i64) {
        if !self.enabled {
            return;
        }
        if live {
            self.down_since.set(None);
        } else if self.down_since.get().is_none() {
            self.down_since.set(Some(now_ms));
        }
        if self.restarting_since.get().is_none() {
            return;
        }
        if !live {
            self.saw_down = true;
        } else if self.saw_down {
            self.restarting_since.set(None);
        }
    }

    /// The strip's spinner provider, sharing this clock.
    pub fn strip_spinner(&self) -> Option<StripSpinner> {
        self.enabled.then(|| StripSpinner {
            clock: self.clock.clone(),
            restarting_since: self.restarting_since.clone(),
            down_since: self.down_since.clone(),
        })
    }

    /// Whether the strip spinner is animating for this state (so the loop
    /// keeps ticking). The static "daemon down" caption does not tick.
    pub fn strip_spinner_active(&self, daemon_live: bool, now_ms: i64) -> bool {
        self.enabled
            && strip_caption(daemon_live, self.restarting_since.get(), self.down_since.get(), now_ms)
                .is_some_and(|c| c.spinning)
    }
}

struct Caption {
    text: &'static str,
    spinning: bool,
}

fn strip_caption(
    daemon_live: bool,
    restarting_since: Option<i64>,
    down_since: Option<i64>,
    now_ms: i64,
) -> Option<Caption> {
    let spin = |text| Some(Caption { text, spinning: true });
    if restarting_since.is_some_and(|since| now_ms - since < RESTART_GRACE_MS) {
        spin("daemon restarting…")
    } else if daemon_live {
        None
    } else if down_since.is_some_and(|since| now_ms - since < DOWN_SPIN_MS) {
        spin("reconnecting…")
    } else {
        Some(Caption { text: "daemon down", spinning: false })
    }
}

/// `strip.right` provider: a throbber plus a dim caption while the daemon
/// restarts or the stream is down.
pub struct StripSpinner {
    clock: Rc<Cell<i64>>,
    restarting_since: Rc<Cell<Option<i64>>>,
    down_since: Rc<Cell<Option<i64>>>,
}

impl SlotProvider for StripSpinner {
    fn items(&self, ctx: &SlotCtx) -> Vec<SlotItem> {
        let live = ctx.state.is_none_or(|s| s.daemon_live);
        let now = self.clock.get();
        let Some(caption) = strip_caption(live, self.restarting_since.get(), self.down_since.get(), now) else {
            return Vec::new();
        };
        let glyph = if caption.spinning { spinner_symbol(now) } else { "◌".to_owned() };
        vec![SlotItem::new(
            vec![
                Span::styled(format!("{glyph} "), Style::default().fg(colors::DIM)),
                Span::styled(caption.text, Style::default().fg(colors::FAINT)),
            ],
            200,
        )]
    }
}

#[cfg(test)]
mod tests;
