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

/// The border-title status bar: glyph, label, ⎇ branch, elapsed (waiting
/// time for needs-input), frozen affordance.
pub fn title_line(v: &TileView) -> Line<'static> {
    let s = v.session;
    let blocked = s.needs_input();
    let glyph_color = status_color(&s.status, s.since, v.now_ms);
    let label_color = if blocked {
        colors::AMBER
    } else if v.engaged || v.focused {
        colors::FG
    } else {
        colors::DIM
    };
    let mut label_style = Style::default().fg(label_color);
    if v.engaged {
        label_style = label_style.add_modifier(Modifier::BOLD);
    }
    if v.focused {
        label_style = label_style.add_modifier(Modifier::UNDERLINED);
    }

    let mut spans = vec![
        Span::styled(format!(" {} ", glyph_for(&s.status)), Style::default().fg(glyph_color)),
        Span::styled(s.label.clone(), label_style),
    ];
    if let Some(branch) = &s.branch {
        spans.push(Span::styled(
            format!(" ⎇ {branch}"),
            Style::default().fg(colors::FAINT),
        ));
    }
    if let Some(elapsed) = elapsed_for(&s.status, s.since, v.now_ms) {
        spans.push(Span::styled(
            format!(" {elapsed}"),
            Style::default().fg(if blocked { colors::AMBER } else { colors::DIM }),
        ));
    }
    if let Some(n) = v.frozen_new_lines {
        spans.push(Span::styled(
            format!(" ↓ live · +{n} lines "),
            Style::default().fg(colors::FG).add_modifier(Modifier::BOLD),
        ));
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
        .title(title_line(v));

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
        assert_eq!(title_line(&v).to_string(), " ● lbl 1:05 ");
    }

    #[test]
    fn title_shows_branch_and_frozen_affordance() {
        let mut s = session("working");
        s.branch = Some("garage/lbl".into());
        let mut v = view(&s);
        v.frozen_new_lines = Some(42);
        assert_eq!(
            title_line(&v).to_string(),
            " ◐ lbl ⎇ garage/lbl 1:05 ↓ live · +42 lines  "
        );
    }

    #[test]
    fn idle_and_done_omit_the_elapsed_timer() {
        let s = session("idle");
        assert_eq!(title_line(&view(&s)).to_string(), " ○ lbl ");
        let s = session("done");
        assert_eq!(title_line(&view(&s)).to_string(), " ✓ lbl ");
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
