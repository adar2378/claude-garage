//! The `?` help overlay (port of `tui/lib/ui/help_overlay.dart`): a
//! centered key legend on the overlay layer (spec tui-key-routing: overlays
//! never stack; `?` toggles; Esc/`q`/any click dismiss).

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Clear, Paragraph, Widget};

use crate::ui::layout::centered_rect;
use crate::ui::theme::colors;

/// The full binding legend (the Dart `_bindings` list, verbatim — the help
/// overlay must list every p8.1–p8.4 key, plus the p16-restart `r` chords:
/// spec restart "TUI chords" requires `?` to list all three).
pub const BINDINGS: [(&str, &str); 25] = [
    ("1-9", "focus workspace"),
    ("[ ]", "cycle focused tile"),
    ("Enter", "engage focused tile (restore it when restorable)"),
    ("Ctrl+G", "disengage (while engaged)"),
    ("m", "maximize / restore focused tile"),
    ("t", "open focused session in a new terminal window (macOS)"),
    ("a", "jump to longest-waiting blocked session"),
    ("A", "triage queue"),
    ("b", "hide / show the sidebar (drag its edge to resize)"),
    ("< / >", "narrow / widen the sidebar"),
    ("n / N", "spawn session / worktree session"),
    ("R", "restore all restorable sessions in workspace"),
    ("I", "install hooks (instant status) + context meter feed"),
    ("x x", "close focused session (press twice)"),
    (
        "r r",
        "restart focused session — resumes on the current claude binary",
    ),
    ("r a", "restart idle/done sessions in workspace"),
    ("r d", "restart the daemon (sessions untouched)"),
    ("X X", "remove focused workspace (sessions keep running)"),
    ("X K", "remove focused workspace AND kill its sessions"),
    ("w", "add workspace"),
    (
        "d",
        "detach focused session to its own view, or rejoin main",
    ),
    ("D", "move focused session to another view / new group"),
    ("Tab", "cycle the focused workspace's views"),
    ("?", "toggle this help"),
    ("q", "quit (tmux sessions keep running)"),
];

#[cfg(test)]
fn legend_lines() -> Vec<Line<'static>> {
    plugin_legend(&[])
}

/// The legend plus, when plugins add any, a "Plugins" section: each row is
/// (left column, right column), a plugin's heading having an empty right
/// column.
fn plugin_legend(plugins: &[(String, String)]) -> Vec<Line<'static>> {
    let mut lines: Vec<Line<'static>> = BINDINGS
        .iter()
        .map(|(key, what)| {
            Line::from(vec![
                Span::styled(format!("{key:<8}"), Style::default().fg(colors::FG)),
                Span::styled(*what, Style::default().fg(colors::DIM)),
            ])
        })
        .collect();
    if !plugins.is_empty() {
        lines.push(Line::raw(""));
        lines.push(Line::styled("Plugins", Style::default().fg(colors::FG)));
        for (left, right) in plugins {
            lines.push(if right.is_empty() {
                Line::styled(left.clone(), Style::default().fg(colors::DIM))
            } else {
                Line::from(vec![
                    Span::styled(format!("{left:<8}"), Style::default().fg(colors::FG)),
                    Span::styled(right.clone(), Style::default().fg(colors::DIM)),
                ])
            });
        }
    }
    lines
}

/// The modal's rect for the current frame (also the click router's
/// inside/outside test — any click dismisses help, so it only needs the
/// frame for rendering symmetry).
pub fn help_modal_rect(area: Rect) -> Rect {
    help_modal_rect_with(area, &[])
}

fn help_modal_rect_with(area: Rect, plugins: &[(String, String)]) -> Rect {
    let core = BINDINGS
        .iter()
        .map(|(k, w)| 8 + w.chars().count().max(k.chars().count()));
    let extra = plugins.iter().map(|(l, r)| {
        if r.is_empty() {
            l.chars().count()
        } else {
            8 + r.chars().count()
        }
    });
    let content_w = core.chain(extra).max().unwrap_or(0) as u16;
    // border (2) + horizontal padding (4) like the Dart container.
    let width = (content_w + 6).min(area.width);
    let section = if plugins.is_empty() {
        0
    } else {
        plugins.len() + 2
    };
    let height = ((BINDINGS.len() + section) as u16 + 4).min(area.height);
    centered_rect(area, width, height)
}

pub fn render_help(buf: &mut Buffer, area: Rect) {
    render_help_with(buf, area, &[]);
}

/// [`render_help`] with the plugins' "Plugins" section (see [`plugin_legend`]).
pub fn render_help_with(buf: &mut Buffer, area: Rect, plugins: &[(String, String)]) {
    let rect = help_modal_rect_with(area, plugins);
    Clear.render(rect, buf);
    let block = Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(colors::DIM))
        .title(Line::styled(" keys ", Style::default().fg(colors::FG)));
    let inner = block.inner(rect);
    block.render(rect, buf);
    // Horizontal padding 2, vertical padding 1 (the Dart EdgeInsets).
    let padded = Rect {
        x: inner.x + 2.min(inner.width),
        y: inner.y + 1.min(inner.height),
        width: inner.width.saturating_sub(4),
        height: inner.height.saturating_sub(2),
    };
    Paragraph::new(plugin_legend(plugins)).render(padded, buf);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_lifecycle_key_is_listed() {
        let keys: Vec<&str> = BINDINGS.iter().map(|(k, _)| *k).collect();
        for key in [
            "1-9", "[ ]", "Enter", "Ctrl+G", "m", "t", "a", "A", "n / N", "R", "I", "x x", "r r",
            "r a", "r d", "X X", "X K", "w", "d", "D", "Tab", "?", "q",
        ] {
            assert!(keys.contains(&key), "missing {key}");
        }
    }

    #[test]
    fn legend_lines_pad_keys_to_a_column() {
        let lines = legend_lines();
        assert_eq!(lines.len(), BINDINGS.len());
        assert!(lines[0].to_string().starts_with("1-9     focus workspace"));
    }

    #[test]
    fn plugins_add_a_section_and_without_them_the_overlay_is_unchanged() {
        let area = Rect::new(0, 0, 120, 50);
        let mut plain = Buffer::empty(area);
        render_help(&mut plain, area);
        let mut none = Buffer::empty(area);
        render_help_with(&mut none, area, &[]);
        assert_eq!(plain, none);

        let rows = vec![
            ("usage".to_owned(), String::new()),
            ("u".to_owned(), "Refresh usage".to_owned()),
        ];
        let mut with = Buffer::empty(area);
        render_help_with(&mut with, area, &rows);
        let text: String = (0..50)
            .map(|y| {
                (0..120)
                    .map(|x| with[(x, y)].symbol().to_owned())
                    .collect::<String>()
                    + "\n"
            })
            .collect();
        assert!(
            text.contains("Plugins")
                && text.contains("usage")
                && text.contains("u       Refresh usage"),
            "{text}"
        );
        assert_eq!(
            help_modal_rect_with(area, &rows).height,
            help_modal_rect(area).height + 4
        );
    }

    #[test]
    fn modal_rect_fits_and_centers() {
        let area = Rect::new(0, 0, 120, 40);
        let r = help_modal_rect(area);
        assert!(r.width < area.width && r.height < area.height);
        assert_eq!(
            r.height,
            BINDINGS.len() as u16 + 4,
            "rows + border + padding"
        );
        // Centered: symmetric margins within a cell.
        assert!((r.x - area.x).abs_diff(area.width - (r.x + r.width)) <= 1);
    }
}
