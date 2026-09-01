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

use crate::state::wall_state::WallSession;
use crate::ui::theme::{colors, elapsed_for, glyph_for, status_color};

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
    let s = v.session;
    let blocked = s.needs_input();
    let glyph_color = status_color(&s.status, s.since, v.now_ms);
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
    if let Some(context) = &s.context {
        tail.push(Span::styled(
            format!(" {}", context_meter_text(context.used_percentage)),
            Style::default().fg(context_meter_color(context.used_percentage)),
        ));
    }

    let glyph = format!(" {} ", glyph_for(&s.status));
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
    Line::from(spans)
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
            return Some(("⟳ restoring…".into(), "resuming the conversation".into()));
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
        return Some(("… attaching".into(), String::new()));
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
pub fn render_tile(buf: &mut Buffer, rect: Rect, v: &TileView, screen: Option<&vt100::Screen>) {
    if rect.width < 2 || rect.height < 2 {
        return;
    }
    let (color, border_type) = border_for(v);
    let block = Block::bordered()
        .border_type(border_type)
        .border_style(Style::default().fg(color))
        .title(title_line(v, rect.width));

    if let Some((heading, detail)) = placeholder_for(v, screen.is_some()) {
        let inner = block.inner(rect);
        block.render(rect, buf);
        let mut lines = vec![Line::styled(heading, Style::default().fg(colors::DIM))];
        if !detail.is_empty() {
            lines.push(Line::styled(detail, Style::default().fg(colors::FAINT)));
        }
        render_centered_lines(buf, inner, lines);
        return;
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
}

#[cfg(test)]
mod tests {
    //! Tile title/placeholder text and border salience (port of the
    //! tile-facing checks in `tui/test/` — the harnesses grep several of
    //! these strings verbatim).
    use super::*;

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

    fn view(session: &WallSession) -> TileView<'_> {
        TileView {
            session,
            focused: false,
            engaged: false,
            restoring: false,
            dead_reason: None,
            frozen_new_lines: None,
            now_ms: 65_000,
        }
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
