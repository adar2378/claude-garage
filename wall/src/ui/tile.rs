//! One grid tile (port of `tui/lib/ui/tile.dart`): an embedded terminal
//! attached to a tmux session, with a border-title status bar, placeholders
//! for every non-terminal state, and the frozen-view affordance.
//!
//! Salience (mockup contract): amber border+title EXCLUSIVELY for
//! needs-input — it wins even over the engaged highlight (a blocked tile
//! must never stop being amber); engaged gets the bright thick frame;
//! focused gets an underlined title; working/idle stay neutral; done is
//! green, fading after 2 minutes. While frozen, the title shows the
//! back-to-live affordance `↓ live · +N lines`.
//!
//! The engaged cursor is tui-term's inverse block at the emulator's cursor
//! cell, shown ONLY while engaged and live — tui-term itself additionally
//! gates on the screen's DECTCEM state (apps that hide the cursor keep it
//! hidden). Unengaged tiles show none: a wall of six carets is noise.

use ratatui::buffer::Buffer;
use ratatui::layout::{Alignment, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Paragraph, Widget};
use tui_term::widget::{Cursor, PseudoTerminal};

use crate::slots::{self, ClickTarget, SlotClickRegion, SlotCtx, SlotId, SlotItem, SlotRegistry};
use crate::state::wall_state::WallSession;
use crate::ui::theme::{colors, elapsed_for, glyph_for, status_color, status_color_fading};

/// Everything the tile render needs besides the terminal screen itself.
pub struct TileView<'a> {
    pub session: &'a WallSession,
    pub focused: bool,
    pub engaged: bool,
    /// A restore call is in flight for this session.
    pub restoring: bool,
    /// Non-`None` once the reattach budget is exhausted.
    pub dead_reason: Option<&'a str>,
    /// `Some(n)` while the frozen scrollback view is active (`n` = lines
    /// arrived since freezing).
    pub frozen_new_lines: Option<i64>,
    pub now_ms: i64,
    /// Motion on: `done` fades over its 2 minutes instead of switching off.
    pub fade_done: bool,
    /// Motion on: the throbber frame for the attaching / restoring
    /// placeholders. `None` keeps the static `…` / `⟳` markers.
    pub spinner: Option<String>,
    /// The slot registry the title bar asks for its right-hand items.
    pub slots: &'a SlotRegistry,
    /// `tile.gutter` markers for this session and where the view sits in the
    /// scrollback; `None` when there are none (the tile then draws exactly as
    /// before).
    pub gutter: Option<Gutter<'a>>,
}

/// The gutter markers of one tile plus the scrollback position they map
/// through.
pub struct Gutter<'a> {
    pub items: &'a [SlotItem],
    /// Absolute scrollback row of the body's top row: the frozen model's
    /// `top_abs`, or the tmux history size for a live tail view.
    pub top_abs: i64,
}

/// The body row (0-based) a marker at absolute scrollback `row` is drawn on,
/// when it is inside the visible `height` rows.
pub fn gutter_screen_row(row: u64, top_abs: i64, height: u16) -> Option<u16> {
    let offset = i64::try_from(row).ok()?.checked_sub(top_abs)?;
    (0..i64::from(height)).contains(&offset).then_some(offset as u16)
}

/// Draw the gutter markers into the tile's left border column (no layout
/// change, so the PTY never resizes as markers come and go) and return their
/// click regions. A row holds one marker: the first by priority wins.
fn render_gutter(buf: &mut Buffer, rect: Rect, v: &TileView, gutter: &Gutter) -> Vec<SlotClickRegion> {
    let height = rect.height.saturating_sub(2);
    let mut items: Vec<&SlotItem> = gutter.items.iter().filter(|i| i.gutter_row.is_some()).collect();
    items.sort_by_key(|i| std::cmp::Reverse(i.priority)); // stable: ties keep provider order
    let mut taken: Vec<u16> = Vec::new();
    let mut regions = Vec::new();
    for item in items {
        let Some(row) = item.gutter_row.and_then(|r| gutter_screen_row(r, gutter.top_abs, height)) else {
            continue;
        };
        if taken.contains(&row) {
            continue;
        }
        taken.push(row);
        let y = rect.y + 1 + row;
        // One column: the first character of the item's text, in its style.
        let Some(span) = item.text.iter().find(|s| !s.content.is_empty()) else { continue };
        let glyph: String = span.content.chars().take(1).collect();
        buf.set_string(rect.x, y, glyph, span.style);
        if let Some(target) = item.on_click.clone() {
            regions.push(SlotClickRegion {
                rect: Rect::new(rect.x, y, 1, 1),
                target,
                slot: SlotId::TileGutter,
                session: Some(v.session.id.clone()),
                workspace: Some(v.session.workspace.clone()),
            });
        }
    }
    regions
}

/// Border color + type per the salience ladder.
pub fn border_for(v: &TileView) -> (Color, BorderType) {
    let blocked = v.session.needs_input();
    let color = if blocked {
        colors::AMBER
    } else if v.engaged {
        colors::FG
    } else if v.focused {
        colors::DIM
    } else {
        colors::FAINT
    };
    let border_type = if v.engaged {
        BorderType::Thick
    } else {
        BorderType::Rounded
    };
    (color, border_type)
}

/// `▰`/`▱` segment count for the tile-bar context meter (spec
/// tui-context-meters "Tile context meter"): `ceil(pct/25)`, capped at 4.
/// Naturally 0 at `pct == 0` (no segment forced) and ≥1 for any `pct > 0` —
/// `u32::div_ceil` gives both for free, no separate `.max(1)` needed.
fn context_segments(pct: u32) -> u32 {
    pct.div_ceil(25).min(4)
}

/// Dim below 80, the palette's red ("compact or restart soon") at 80 and
/// above — NEVER amber; amber stays exclusive to needs-input even here.
fn context_meter_color(pct: u32) -> Color {
    if pct >= 80 {
        colors::CTX_HOT
    } else {
        colors::DIM
    }
}

/// `▰▰▱▱ 42%`-style meter text.
fn context_meter_text(pct: u32) -> String {
    let filled = context_segments(pct) as usize;
    format!("{}{} {pct}%", "▰".repeat(filled), "▱".repeat(4 - filled))
}

/// The `tile.title.right` core provider: the context meter, when the session
/// has context telemetry.
pub fn context_meter_provider(ctx: &SlotCtx) -> Vec<SlotItem> {
    let Some(context) = ctx.session.and_then(|s| s.context.as_ref()) else {
        return Vec::new();
    };
    vec![SlotItem::new(
        vec![Span::styled(
            format!(" {}", context_meter_text(context.used_percentage)),
            Style::default().fg(context_meter_color(context.used_percentage)),
        )],
        100,
    )]
}

/// Truncate `text` to `width` cells with a trailing ellipsis (the subtitle's
/// own truncation — spec tui-wall "Auto-subtitle in the tile bar").
fn fit_subtitle(text: &str, width: usize) -> String {
    if width == 0 {
        return String::new();
    }
    let chars: Vec<char> = text.chars().collect();
    if chars.len() <= width {
        return text.to_owned();
    }
    if width == 1 {
        return "…".to_owned();
    }
    let mut out: String = chars[..width - 1].iter().collect();
    out.push('…');
    out
}

/// The border-title status bar: glyph, display name (spec tui-wall "Title
/// as display name" — the live title when there is one, else the label),
/// ⎇ branch, elapsed (waiting time for needs-input), frozen affordance,
/// context meter, and — lowest priority, only when it still fits — the
/// demoted auto-label as a dim trailing id. `width` is the tile's rendered
/// width (the Block's top edge, corners included). Ladder: glyph, branch,
/// elapsed, frozen, and the meter never shrink; the name ellipsis-truncates
/// into what's left (falling back to the untruncated label when not even
/// one character of the title fits); the trailing id drops first.
pub fn title_line(v: &TileView, width: u16) -> Line<'static> {
    title_parts(v, width).0
}

/// [`title_line`] plus the clickable `tile.title.right` items: their column
/// ranges within the line (the line starts one column inside the border).
pub fn title_parts(
    v: &TileView,
    width: u16,
) -> (Line<'static>, Vec<(std::ops::Range<u16>, ClickTarget)>) {
    let s = v.session;
    let blocked = s.needs_input();
    let glyph_color = if v.fade_done {
        status_color_fading(&s.status, s.since, v.now_ms)
    } else {
        status_color(&s.status, s.since, v.now_ms)
    };
    let name_color = if blocked {
        colors::AMBER
    } else if v.engaged || v.focused {
        colors::FG
    } else {
        colors::DIM
    };
    let mut name_style = Style::default().fg(name_color);
    if v.engaged {
        name_style = name_style.add_modifier(Modifier::BOLD);
    }
    if v.focused {
        name_style = name_style.add_modifier(Modifier::UNDERLINED);
    }

    // Everything that never shrinks, built first so the name knows how much
    // room it actually has.
    let mut tail: Vec<Span<'static>> = Vec::new();
    if let Some(branch) = &s.branch {
        tail.push(Span::styled(
            format!(" ⎇ {branch}"),
            Style::default().fg(colors::FAINT),
        ));
    }
    if let Some(elapsed) = elapsed_for(&s.status, s.since, v.now_ms) {
        tail.push(Span::styled(
            format!(" {elapsed}"),
            Style::default().fg(if blocked { colors::AMBER } else { colors::DIM }),
        ));
    }
    if let Some(n) = v.frozen_new_lines {
        tail.push(Span::styled(
            format!(" ↓ live · +{n} lines "),
            Style::default().fg(colors::FG).add_modifier(Modifier::BOLD),
        ));
    }
    // Context meter (spec tui-context-meters "Tile context meter"): renders
    // unconditionally, never truncated. `None` renders as before.
    let slot_ctx = SlotCtx {
        session: Some(s),
        ..SlotCtx::default()
    };
    let tail_before_slots: usize = tail.iter().map(Span::width).sum();
    let laid = slots::layout(v.slots.items(SlotId::TileTitleRight, &slot_ctx), None);
    tail.extend(laid.spans.iter().cloned());

    let glyph = format!(" {} ", glyph_for(&s.status));
    let glyph_w = glyph.chars().count();
    let fixed: usize =
        glyph.chars().count() + tail.iter().map(Span::width).sum::<usize>() + 1; // +1 trailing space
    let avail = (width as usize).saturating_sub(fixed);
    let name = s.display_name();
    let mut fitted = fit_subtitle(name, avail);
    if fitted.is_empty() {
        // Not even one character fits — old behavior: the label renders
        // untruncated rather than the bar losing its identity entirely.
        fitted = s.label.clone();
    }
    let titled = fitted != s.label; // the title, not the label, is the name

    let mut spans = vec![
        Span::styled(glyph, Style::default().fg(glyph_color)),
        Span::styled(fitted, name_style),
    ];
    spans.extend(tail);
    // The demoted auto-label id, dim, lowest priority — only when the title
    // took the name slot and the id still fits whole.
    if titled && name != s.label {
        let used: usize = spans.iter().map(Span::width).sum();
        let room = (width as usize).saturating_sub(used + 2);
        if s.label.chars().count() <= room {
            spans.push(Span::styled(
                format!(" {}", s.label),
                Style::default().fg(colors::FAINT),
            ));
        }
    }
    spans.push(Span::raw(" "));
    let slot_offset = glyph_w + Span::width(&spans[1]) + tail_before_slots;
    let clicks = laid.clicks(slot_offset, usize::from(width.saturating_sub(2)));
    (Line::from(spans), clicks)
}

/// The tile's placeholder body, or `None` when a terminal should render.
/// `has_client` = a PTY client is running for this tile. Exact Dart wording
/// — the p8.1 harness greps the restorable line.
pub fn placeholder_for(v: &TileView, has_client: bool) -> Option<(String, String)> {
    if !v.session.live() {
        // Restorable: no PTY is ever spawned (spec tui-wall "Restorable
        // placeholder"). While a restore call is in flight the placeholder
        // says so (optimistic; the refetch settles the real status).
        if v.restoring {
            let head = match &v.spinner {
                Some(spin) => format!("{spin} restoring…"),
                None => "⟳ restoring…".to_owned(),
            };
            return Some((head, "resuming the conversation".into()));
        }
        return Some((
            "⟳ restorable".into(),
            "press Enter (or click) to restore · x x to discard".into(),
        ));
    }
    if let Some(reason) = v.dead_reason {
        return Some(("✕ attach dead".into(), reason.to_owned()));
    }
    if !has_client {
        let head = match &v.spinner {
            Some(spin) => format!("{spin} attaching"),
            None => "… attaching".to_owned(),
        };
        return Some((head, String::new()));
    }
    None
}

/// Center `lines` vertically and horizontally inside `rect` (the Dart
/// `Center(Column(...))` placeholder shape). Also used by the grid empty
/// states.
pub fn render_centered_lines(buf: &mut Buffer, rect: Rect, lines: Vec<Line<'static>>) {
    if rect.height == 0 || rect.width == 0 {
        return;
    }
    let n = lines.len() as u16;
    let y = rect.y + rect.height.saturating_sub(n) / 2;
    let target = Rect {
        y,
        height: n.min(rect.height.saturating_sub(y - rect.y)),
        ..rect
    };
    Paragraph::new(lines)
        .alignment(Alignment::Center)
        .render(target, buf);
}

/// Render one tile: border + title always; body = placeholder or the
/// terminal screen (`screen` is the live vt100 screen, or the frozen
/// capture snapshot while frozen).
///
/// Returns the click regions of the clickable `tile.title.right` items it
/// painted (each scoped to this tile's session and workspace).
pub fn render_tile(
    buf: &mut Buffer,
    rect: Rect,
    v: &TileView,
    screen: Option<&vt100::Screen>,
) -> Vec<SlotClickRegion> {
    if rect.width < 2 || rect.height < 2 {
        return Vec::new();
    }
    let (color, border_type) = border_for(v);
    let (title, title_clicks) = title_parts(v, rect.width);
    let mut regions: Vec<SlotClickRegion> = title_clicks
        .into_iter()
        .map(|(cols, target)| SlotClickRegion {
            rect: Rect::new(rect.x + 1 + cols.start, rect.y, cols.end - cols.start, 1),
            target,
            slot: SlotId::TileTitleRight,
            session: Some(v.session.id.clone()),
            workspace: Some(v.session.workspace.clone()),
        })
        .collect();
    let block = Block::bordered()
        .border_type(border_type)
        .border_style(Style::default().fg(color))
        .title(title);

    if let Some((heading, detail)) = placeholder_for(v, screen.is_some()) {
        let inner = block.inner(rect);
        block.render(rect, buf);
        let mut lines = vec![Line::styled(heading, Style::default().fg(colors::DIM))];
        if !detail.is_empty() {
            lines.push(Line::styled(detail, Style::default().fg(colors::FAINT)));
        }
        render_centered_lines(buf, inner, lines);
        return regions;
    }

    let screen = screen.expect("placeholder_for covers the missing-client case");
    // Cursor rule (task 4.1): inverse block at the emulator cursor, only on
    // the engaged live tile, suppressed in a frozen history view; tui-term
    // gates on DECTCEM itself.
    //
    // tui-term's default only paints its REVERSED overlay when the cursor
    // cell has contents; on an EMPTY cell (the common case — the cursor
    // sits just past the prompt/composer text) it falls back to a "█" glyph
    // with a plain gray foreground, which is not an inverse cell at all.
    // The p8 contract (asserted by run_p82.sh via capture-pane -e SGR
    // parsing) is exactly ONE SGR-7 inverse-video cell, so give the
    // empty-cell path a reversed space: both paths then render inverse.
    let cursor = Cursor::default()
        .symbol(" ")
        .style(Style::default().add_modifier(Modifier::REVERSED))
        .visibility(v.engaged && v.frozen_new_lines.is_none());
    PseudoTerminal::new(screen)
        .block(block)
        .cursor(cursor)
        .render(rect, buf);
    if let Some(gutter) = &v.gutter {
        regions.extend(render_gutter(buf, rect, v, gutter));
    }
    regions
}

#[cfg(test)]
mod tests {
    //! Tile title/placeholder text and border salience (port of the
    //! tile-facing checks in `tui/test/` — the harnesses grep several of
    //! these strings verbatim).
    use super::*;
    use ratatui::buffer::Buffer;

    fn session(status: &str) -> WallSession {
        WallSession {
            id: "garage/ws/lbl".into(),
            workspace: "ws".into(),
            label: "lbl".into(),
            dir: None,
            status: status.into(),
            since: Some(0),
            message: None,
            branch: None,
            worktree: false,
            title: None,
            context: None,
        }
    }

    /// A registry for the tests' `TileView`s (leaked: a view borrows it).
    fn test_slots() -> &'static SlotRegistry {
        Box::leak(Box::new(SlotRegistry::core()))
    }

    fn view(session: &WallSession) -> TileView<'_> {
        TileView {
            session,
            focused: false,
            engaged: false,
            restoring: false,
            dead_reason: None,
            frozen_new_lines: None,
            now_ms: 65_000,
            fade_done: false,
            spinner: None,
            slots: test_slots(),
            gutter: None,
        }
    }

    #[test]
    fn attaching_and_restoring_markers_are_static_without_a_spinner() {
        let s = session("working");
        let v = view(&s);
        assert_eq!(placeholder_for(&v, false).unwrap().0, "… attaching");
        let r = session("restorable");
        let mut v = view(&r);
        v.restoring = true;
        assert_eq!(placeholder_for(&v, false).unwrap().0, "⟳ restoring…");
    }

    #[test]
    fn a_spinner_replaces_only_the_leading_marker() {
        let s = session("working");
        let mut v = view(&s);
        v.spinner = Some("⠷".into());
        assert_eq!(placeholder_for(&v, false).unwrap().0, "⠷ attaching");
        let r = session("restorable");
        let mut v = view(&r);
        v.restoring = true;
        v.spinner = Some("⠯".into());
        assert_eq!(placeholder_for(&v, false).unwrap().0, "⠯ restoring…");
        // The restorable (idle) placeholder never spins.
        v.restoring = false;
        assert_eq!(placeholder_for(&v, false).unwrap().0, "⟳ restorable");
    }

    #[test]
    fn fade_done_only_changes_the_done_glyph() {
        let done = session("done");
        let mut v = view(&done);
        v.now_ms = 60_000;
        let hard = title_line(&v, 40).spans[0].style.fg;
        v.fade_done = true;
        let faded = title_line(&v, 40).spans[0].style.fg;
        assert_eq!(hard, Some(colors::GREEN));
        assert!(matches!(faded, Some(Color::Rgb(..))), "mid-fade is a stepped tone");
        let working = session("working");
        let mut w = view(&working);
        let before = title_line(&w, 40);
        w.fade_done = true;
        assert_eq!(title_line(&w, 40), before);
    }

    #[test]
    fn amber_border_wins_even_over_engaged() {
        let s = session("needs-input");
        let mut v = view(&s);
        v.engaged = true;
        let (color, border_type) = border_for(&v);
        assert_eq!(color, colors::AMBER);
        assert_eq!(border_type, BorderType::Thick, "engaged still bolds the frame");
    }

    #[test]
    fn border_ladder_engaged_focused_neutral() {
        let s = session("working");
        let mut v = view(&s);
        v.engaged = true;
        assert_eq!(border_for(&v).0, colors::FG);
        v.engaged = false;
        v.focused = true;
        assert_eq!(border_for(&v), (colors::DIM, BorderType::Rounded));
        v.focused = false;
        assert_eq!(border_for(&v).0, colors::FAINT);
    }

    #[test]
    fn title_carries_glyph_label_elapsed() {
        let s = session("needs-input");
        let v = view(&s);
        assert_eq!(title_line(&v, 200).to_string(), " ● lbl 1:05 ");
    }

    #[test]
    fn title_shows_branch_and_frozen_affordance() {
        let mut s = session("working");
        s.branch = Some("garage/lbl".into());
        let mut v = view(&s);
        v.frozen_new_lines = Some(42);
        assert_eq!(
            title_line(&v, 200).to_string(),
            " ◐ lbl ⎇ garage/lbl 1:05 ↓ live · +42 lines  "
        );
    }

    #[test]
    fn idle_and_done_omit_the_elapsed_timer() {
        let s = session("idle");
        assert_eq!(title_line(&view(&s), 200).to_string(), " ○ lbl ");
        let s = session("done");
        assert_eq!(title_line(&view(&s), 200).to_string(), " ✓ lbl ");
    }

    // ── p13: title as display name (spec tui-wall "Title as display name") ─

    #[test]
    fn no_title_renders_byte_identical_to_before_the_feature() {
        let s = session("idle");
        assert_eq!(title_line(&view(&s), 200).to_string(), " ○ lbl ");
    }

    #[test]
    fn a_title_matching_the_label_adds_no_trailing_id() {
        let mut s = session("idle");
        s.title = Some("lbl".to_owned());
        assert_eq!(title_line(&view(&s), 200).to_string(), " ○ lbl ");
    }

    #[test]
    fn title_takes_the_name_slot_with_the_label_demoted_to_a_dim_id() {
        let mut s = session("idle");
        s.title = Some("build the thing".to_owned());
        let line = title_line(&view(&s), 200);
        assert_eq!(line.to_string(), " ○ build the thing lbl ");
        let name_span = &line.spans[1];
        assert_eq!(name_span.style.fg, Some(colors::DIM)); // idle name color
        let id_span = &line.spans[line.spans.len() - 2];
        assert_eq!(id_span.style.fg, Some(colors::FAINT), "demoted id is faint");
    }

    #[test]
    fn title_name_keeps_amber_salience_when_blocked() {
        let mut s = session("needs-input");
        s.title = Some("build the thing".to_owned());
        let line = title_line(&view(&s), 200);
        assert_eq!(line.spans[1].style.fg, Some(colors::AMBER));
    }

    #[test]
    fn long_title_truncates_without_pushing_branch_or_elapsed_out() {
        // Spec scenario "Long title truncates without pushing structure
        // out": 40-wide tile — the name truncates, branch/elapsed unhurt.
        let mut s = session("needs-input");
        s.branch = Some("garage/lbl".to_owned());
        s.title = Some("a very long summary of what this session is doing right now".to_owned());
        let line = title_line(&view(&s), 40);
        let text = line.to_string();
        assert!(text.chars().count() <= 40, "{text}");
        assert!(text.contains(" ⎇ garage/lbl 1:05 "), "{text}");
        assert!(line.spans[1].content.ends_with('…'), "{text}");
    }

    #[test]
    fn name_falls_back_to_the_label_when_nothing_is_left() {
        // Branch/elapsed must never be shortened to make room; with zero
        // room the bar keeps its old identity (the untruncated label).
        let mut s = session("needs-input");
        s.branch = Some("a-genuinely-quite-long-branch-name-here".to_owned());
        s.title = Some("anything".to_owned());
        let line = title_line(&view(&s), 30);
        let text = line.to_string();
        assert!(!text.contains("anything"));
        assert!(text.contains("lbl"), "{text}");
        assert!(text.contains("a-genuinely-quite-long-branch-name-here"), "{text}");
    }

    // ── p24: clickable tile.title.right items ───────────────────────────

    fn button_slots() -> SlotRegistry {
        let mut slots = SlotRegistry::core();
        slots.register(SlotId::TileTitleRight, |_: &slots::SlotCtx| {
            vec![SlotItem {
                on_click: Some(ClickTarget::new(crate::actions::ids::SESSION_CLOSE).from_plugin("tiles")),
                ..SlotItem::new(vec![Span::raw(" [x]")], 5)
            }]
        });
        slots
    }

    /// The text the buffer shows under `rect`.
    fn cells(buf: &Buffer, rect: Rect) -> String {
        (rect.x..rect.x + rect.width).map(|x| buf[(x, rect.y)].symbol()).collect()
    }

    #[test]
    fn a_title_button_region_covers_exactly_the_columns_its_text_landed_on() {
        let slots = button_slots();
        for (title, width) in [(None, 50u16), (Some("a long title that has to be cut short"), 44)] {
            let mut s = session("working");
            s.branch = Some("feat/x".into());
            s.title = title.map(str::to_owned);
            let mut v = view(&s);
            v.slots = &slots;
            let rect = Rect::new(3, 2, width, 4);
            let mut buf = Buffer::empty(Rect::new(0, 0, 60, 8));
            let parser = vt100::Parser::new(2, width - 2, 0);
            let regions = render_tile(&mut buf, rect, &v, Some(parser.screen()));
            assert_eq!(regions.len(), 1, "{title:?}");
            let region = &regions[0];
            // `title_line` is sized to the full tile width, so a long title's
            // last column can fall under the border and be clipped; the
            // region must clip with it and never claim columns the paint did
            // not reach.
            let shown = cells(&buf, region.rect);
            assert!(" [x]".starts_with(&shown) && shown.len() >= 3, "{title:?}: {shown:?}");
            if title.is_none() {
                assert_eq!(shown, " [x]");
            }
            assert_eq!(region.rect.y, rect.y, "the title row");
            assert_eq!(region.session.as_deref(), Some("garage/ws/lbl"));
            assert_eq!(region.workspace.as_deref(), Some("ws"));
            assert_eq!(region.slot, SlotId::TileTitleRight);
        }
    }

    #[test]
    fn a_title_button_clipped_by_a_narrow_tile_is_clipped_in_the_region_too() {
        let slots = button_slots();
        let s = session("working");
        let mut v = view(&s);
        v.slots = &slots;
        let rect = Rect::new(0, 0, 12, 3);
        let mut buf = Buffer::empty(rect);
        let parser = vt100::Parser::new(1, 10, 0);
        let regions = render_tile(&mut buf, rect, &v, Some(parser.screen()));
        for r in &regions {
            assert!(r.rect.right() < rect.right(), "never over the right border: {r:?}");
        }
    }

    #[test]
    fn a_tile_with_no_clickable_title_item_has_no_regions() {
        let s = session("working");
        let v = view(&s);
        let mut buf = Buffer::empty(Rect::new(0, 0, 40, 4));
        let parser = vt100::Parser::new(2, 38, 0);
        assert!(render_tile(&mut buf, Rect::new(0, 0, 40, 4), &v, Some(parser.screen())).is_empty());
    }

    // ── p24: tile.gutter ────────────────────────────────────────────────

    #[test]
    fn gutter_rows_map_through_the_scrollback_offset() {
        // Body rows 0..10 show absolute rows 100..110.
        assert_eq!(gutter_screen_row(100, 100, 10), Some(0));
        assert_eq!(gutter_screen_row(109, 100, 10), Some(9));
        assert_eq!(gutter_screen_row(99, 100, 10), None, "scrolled out above");
        assert_eq!(gutter_screen_row(110, 100, 10), None, "below the view");
        assert_eq!(gutter_screen_row(0, 0, 10), Some(0), "a view at the very top of history");
        assert_eq!(gutter_screen_row(5, 100, 0), None, "no body rows");
        assert_eq!(gutter_screen_row(u64::MAX, 0, 10), None, "no overflow");
    }

    fn marker(row: u64, glyph: &str, priority: i16, clickable: bool) -> SlotItem {
        SlotItem {
            gutter_row: Some(row),
            on_click: clickable
                .then(|| ClickTarget::new(crate::actions::ids::SESSION_FOCUS).from_plugin("turns")),
            ..SlotItem::new(vec![Span::styled(glyph.to_owned(), Style::default().fg(colors::AMBER))], priority)
        }
    }

    /// Render a 20x8 live tile (body 6 rows) and return the buffer + regions.
    fn render_with_gutter(items: &[SlotItem], top_abs: i64) -> (Buffer, Vec<SlotClickRegion>) {
        let s = session("working");
        let mut v = view(&s);
        v.gutter = Some(Gutter { items, top_abs });
        let rect = Rect::new(2, 1, 20, 8);
        let mut buf = Buffer::empty(Rect::new(0, 0, 30, 10));
        let parser = vt100::Parser::new(6, 18, 0);
        let regions = render_tile(&mut buf, rect, &v, Some(parser.screen()));
        (buf, regions)
    }

    #[test]
    fn gutter_markers_land_on_their_rows_in_the_left_border_column() {
        let items = [marker(102, "▌", 1, true), marker(104, "◆", 1, false), marker(50, "x", 1, true)];
        let (buf, regions) = render_with_gutter(&items, 100);
        // Tile at x=2,y=1; body starts at y=2. Row 102 -> body row 2 -> y=4.
        assert_eq!(buf[(2, 4)].symbol(), "▌");
        assert_eq!(buf[(2, 4)].fg, colors::AMBER);
        assert_eq!(buf[(2, 6)].symbol(), "◆");
        assert_eq!(buf[(2, 3)].symbol(), "│", "other rows keep the border");
        // Only the clickable, visible marker has a region; the off-screen one has none.
        assert_eq!(regions.len(), 1);
        assert_eq!(regions[0].rect, Rect::new(2, 4, 1, 1));
        assert_eq!(regions[0].slot, SlotId::TileGutter);
        assert_eq!(regions[0].session.as_deref(), Some("garage/ws/lbl"));
    }

    #[test]
    fn markers_follow_the_scrollback_offset_when_the_view_moves() {
        let items = [marker(102, "▌", 1, true)];
        let (_, at_100) = render_with_gutter(&items, 100);
        let (_, at_101) = render_with_gutter(&items, 101);
        assert_eq!(at_100[0].rect.y, 4);
        assert_eq!(at_101[0].rect.y, 3, "scrolled one line: the marker moves up one row");
        let (buf, scrolled_out) = render_with_gutter(&items, 103);
        assert!(scrolled_out.is_empty());
        assert!((2..8).all(|y| buf[(2, y)].symbol() == "│"), "no marker drawn");
    }

    #[test]
    fn one_marker_per_row_the_higher_priority_wins() {
        let items = [marker(101, "a", 1, false), marker(101, "B", 9, false), marker(101, "c", 9, false)];
        let (buf, _) = render_with_gutter(&items, 100);
        assert_eq!(buf[(2, 3)].symbol(), "B", "highest priority, ties keep the earlier");
    }

    #[test]
    fn a_tile_with_no_gutter_items_renders_byte_identical_to_one_without_the_feature() {
        let (plain, plain_regions) = {
            let s = session("working");
            let v = view(&s);
            let rect = Rect::new(2, 1, 20, 8);
            let mut buf = Buffer::empty(Rect::new(0, 0, 30, 10));
            let parser = vt100::Parser::new(6, 18, 0);
            let regions = render_tile(&mut buf, rect, &v, Some(parser.screen()));
            (buf, regions)
        };
        let (with_empty, regions) = render_with_gutter(&[], 100);
        assert_eq!(plain, with_empty);
        assert_eq!(plain_regions, regions);
    }

    #[test]
    fn a_placeholder_tile_draws_no_gutter() {
        let s = session("restorable");
        let items = [marker(100, "▌", 1, true)];
        let mut v = view(&s);
        v.gutter = Some(Gutter { items: &items, top_abs: 100 });
        let mut buf = Buffer::empty(Rect::new(0, 0, 30, 10));
        assert!(render_tile(&mut buf, Rect::new(2, 1, 20, 8), &v, None).is_empty());
    }

    // ── p11: context meter (spec tui-context-meters "Tile context meter") ──

    fn context(used_percentage: u32, source: &str) -> crate::api::models::ContextInfo {
        crate::api::models::ContextInfo {
            used_percentage,
            source: source.to_owned(),
        }
    }

    #[test]
    fn context_segment_math_edges() {
        assert_eq!(context_segments(0), 0, "0% shows every segment empty");
        assert_eq!(context_segments(1), 1, "min 1 segment once pct > 0");
        assert_eq!(context_segments(25), 1);
        assert_eq!(context_segments(79), 4, "ceil(79/25) == 4, same as 80/88/100");
        assert_eq!(context_segments(80), 4);
        assert_eq!(context_segments(100), 4);
    }

    #[test]
    fn context_meter_color_is_dim_below_80_never_amber() {
        assert_eq!(context_meter_color(0), colors::DIM);
        assert_eq!(context_meter_color(42), colors::DIM);
        assert_eq!(context_meter_color(79), colors::DIM);
        assert_ne!(context_meter_color(79), colors::AMBER);
    }

    #[test]
    fn context_meter_color_is_hot_red_at_80_and_above() {
        assert_eq!(context_meter_color(80), colors::CTX_HOT);
        assert_eq!(context_meter_color(88), colors::CTX_HOT);
        assert_eq!(context_meter_color(100), colors::CTX_HOT);
        assert_ne!(context_meter_color(88), colors::AMBER);
    }

    #[test]
    fn meter_renders_dim_at_42_percent() {
        let mut s = session("idle");
        s.context = Some(context(42, "statusline"));
        let line = title_line(&view(&s), 200);
        assert_eq!(line.to_string(), " ○ lbl ▰▰▱▱ 42% ");
        let meter_span = &line.spans[line.spans.len() - 2];
        assert_eq!(meter_span.style.fg, Some(colors::DIM));
    }

    #[test]
    fn meter_renders_red_at_88_percent() {
        let mut s = session("idle");
        s.context = Some(context(88, "transcript"));
        let line = title_line(&view(&s), 200);
        assert_eq!(line.to_string(), " ○ lbl ▰▰▰▰ 88% ");
        let meter_span = &line.spans[line.spans.len() - 2];
        assert_eq!(meter_span.style.fg, Some(colors::CTX_HOT));
    }

    #[test]
    fn null_context_renders_byte_identical_to_before_the_feature() {
        let s = session("needs-input"); // context: None from the fixture
        assert_eq!(title_line(&view(&s), 200).to_string(), " ● lbl 1:05 ");
    }

    #[test]
    fn meter_sits_above_the_subtitle_in_the_truncation_ladder() {
        // Ladder priority (spec): above the subtitle, below
        // glyph/label/branch/elapsed — those never shrink, the meter never
        // shrinks either, only the subtitle drops when nothing is left.
        let mut s = session("idle");
        s.context = Some(context(42, "statusline"));
        s.title = Some("a very long summary that will not fit at all".to_owned());
        let line = title_line(&view(&s), 16);
        let text = line.to_string();
        assert!(text.contains("▰▰▱▱ 42%"), "{text}");
        assert!(!text.contains("a very long summary"), "{text}");
    }

    #[test]
    fn restorable_placeholder_uses_the_exact_dart_wording() {
        let s = session("restorable");
        let v = view(&s);
        assert_eq!(
            placeholder_for(&v, false),
            Some((
                "⟳ restorable".into(),
                "press Enter (or click) to restore · x x to discard".into()
            ))
        );
    }

    #[test]
    fn restoring_placeholder_while_the_call_is_in_flight() {
        let s = session("restorable");
        let mut v = view(&s);
        v.restoring = true;
        assert_eq!(
            placeholder_for(&v, false),
            Some(("⟳ restoring…".into(), "resuming the conversation".into()))
        );
    }

    #[test]
    fn engaged_cursor_is_one_inverse_cell_even_on_an_empty_cell() {
        // Regression (p9 parity gate, run_p82.sh): tui-term's default cursor
        // paints REVERSED only when the cursor cell has contents; just past
        // the prompt (empty cell) it drew a gray "█" instead, so the outer
        // capture had ZERO inverse cells. Both paths must yield exactly one
        // REVERSED cell at the emulator cursor.
        let s = session("working");
        let mut v = view(&s);
        v.engaged = true;

        // "% " then the cursor rests on the EMPTY cell at column 2.
        let mut parser = vt100::Parser::new(5, 20, 0);
        parser.process(b"% ");
        let mut buf = Buffer::empty(Rect::new(0, 0, 22, 7));
        render_tile(&mut buf, Rect::new(0, 0, 22, 7), &v, Some(parser.screen()));
        // Inner origin is (1,1): border offsets the emulator grid by one.
        let cell = &buf[(1 + 2, 1)];
        assert!(
            cell.style().add_modifier.contains(Modifier::REVERSED),
            "empty-cell cursor must render as an inverse cell, got {cell:?}"
        );
        let reversed = buf
            .content()
            .iter()
            .filter(|c| c.style().add_modifier.contains(Modifier::REVERSED))
            .count();
        assert_eq!(reversed, 1, "exactly one inverse cell (the cursor)");

        // Cursor ON a cell with contents (move back onto '%'): still one
        // inverse cell, via tui-term's overlay path.
        parser.process(b"\x1b[1;1H");
        let mut buf = Buffer::empty(Rect::new(0, 0, 22, 7));
        render_tile(&mut buf, Rect::new(0, 0, 22, 7), &v, Some(parser.screen()));
        assert!(buf[(1, 1)].style().add_modifier.contains(Modifier::REVERSED));

        // Unengaged: no inverse cell anywhere.
        v.engaged = false;
        let mut buf = Buffer::empty(Rect::new(0, 0, 22, 7));
        render_tile(&mut buf, Rect::new(0, 0, 22, 7), &v, Some(parser.screen()));
        let reversed = buf
            .content()
            .iter()
            .filter(|c| c.style().add_modifier.contains(Modifier::REVERSED))
            .count();
        assert_eq!(reversed, 0, "unengaged tiles paint no cursor");
    }

    #[test]
    fn dead_and_attaching_placeholders() {
        let s = session("working");
        let mut v = view(&s);
        v.dead_reason = Some("attach exited (code 1) — gave up after 4 reattach attempts");
        assert_eq!(
            placeholder_for(&v, false).unwrap().0,
            "✕ attach dead"
        );
        v.dead_reason = None;
        assert_eq!(
            placeholder_for(&v, false),
            Some(("… attaching".into(), String::new()))
        );
        assert_eq!(placeholder_for(&v, true), None, "client present → terminal");
    }
}
