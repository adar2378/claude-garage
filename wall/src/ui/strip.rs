//! The single-line bottom strip (port of `tui/lib/ui/strip.dart` — spec
//! tui-wall / tui-key-routing): workspace tabs `1-9` with amber needs-input
//! dots, a transient notice slot, the total blocked badge, and the
//! always-visible keys-target chip.
//!
//! The badge is a click target (spec tui-triage: "Pressing `A` (or clicking
//! the strip badge) SHALL open" the queue) — [`StripLine`] carries its
//! rendered column range so the mouse router uses the SAME positions the
//! paint used.

use std::ops::Range;

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Paragraph, Widget};

use crate::api::models::UsageInfo;
use crate::state::wall_state::{KeyLayer, WallState};
use crate::ui::theme::colors;

/// The one-time statusline-install hint's exact wording (spec
/// tui-context-meters "Install affordance") — rendered dim, never amber; the
/// runtime shows it through the same notice slot as any other strip notice
/// (see `runtime.rs`'s hint scheduling), so `strip_line` special-cases this
/// text to pick the dim color instead of an ordinary notice's bright one.
pub const STATUSLINE_HINT: &str = "context meters: press I to install the statusline feed";

/// `5h N% · wk M%`-style account usage chip (spec tui-context-meters "Strip
/// usage chip"): a null window is omitted, not zero-filled; both null hides
/// the chip entirely (`None`).
fn usage_chip_text(usage: &UsageInfo) -> Option<String> {
    let five_hour = usage
        .five_hour
        .as_ref()
        .map(|w| format!("5h {}%", w.used_percentage));
    let seven_day = usage
        .seven_day
        .as_ref()
        .map(|w| format!("wk {}%", w.used_percentage));
    match (five_hour, seven_day) {
        (None, None) => None,
        (Some(a), None) => Some(a),
        (None, Some(b)) => Some(b),
        (Some(a), Some(b)) => Some(format!("{a} · {b}")),
    }
}

pub struct StripLine {
    pub line: Line<'static>,
    /// Column range (strip-local) of the blocked badge, when rendered.
    pub badge: Option<Range<u16>>,
}

/// Build the strip's line for `width` columns: tabs left, then a filler, then
/// notice · badge · chip right-aligned.
pub fn strip_line(state: &WallState, notice: Option<&str>, width: u16) -> StripLine {
    let mut left: Vec<Span<'static>> = Vec::new();
    for (i, group) in state.groups.iter().take(9).enumerate() {
        let focused = state.focused_workspace.as_deref() == Some(group.name.as_str());
        let mut style = Style::default().fg(if focused { colors::FG } else { colors::DIM });
        if focused {
            style = style.add_modifier(Modifier::BOLD);
        }
        left.push(Span::styled(format!(" {}:{}", i + 1, group.name), style));
        if group.has_needs_input() {
            left.push(Span::styled("●", Style::default().fg(colors::AMBER)));
        }
    }

    let blocked = state.blocked_count();
    let mut right: Vec<Span<'static>> = Vec::new();
    let mut badge_local: Option<Range<usize>> = None;
    if let Some(notice) = notice {
        // The install hint rides the same notice slot but reads dim, never
        // the bright color an ordinary notice gets (spec: "never amber",
        // and dim per the mockup's quiet-affordance tone).
        let color = if notice == STATUSLINE_HINT { colors::DIM } else { colors::FG };
        right.push(Span::styled(format!(" {notice} "), Style::default().fg(color)));
    }
    // Usage chip (spec tui-context-meters "Strip usage chip"): additive only
    // — inserted between the notice and the badge/keys chip so it never
    // displaces either of them.
    if let Some(chip) = usage_chip_text(&state.usage) {
        right.push(Span::styled(
            format!(" {chip} "),
            Style::default().fg(colors::DIM),
        ));
    }
    if blocked > 0 {
        let text = format!(" ● {blocked} blocked ");
        let start: usize = right.iter().map(Span::width).sum();
        badge_local = Some(start..start + text.chars().count());
        right.push(Span::styled(text, Style::default().fg(colors::AMBER)));
    }
    let chip_engaged = state.layer == KeyLayer::Engaged;
    let chip_style = if chip_engaged {
        Style::default().fg(Color::Black).bg(colors::AMBER)
    } else {
        Style::default().fg(colors::DIM)
    };
    right.push(Span::styled(
        format!(" {} ", state.keys_target_chip()),
        chip_style,
    ));

    let left_w: usize = left.iter().map(Span::width).sum();
    let right_w: usize = right.iter().map(Span::width).sum();
    let filler = usize::from(width).saturating_sub(left_w + right_w);

    let mut spans = left;
    spans.push(Span::raw(" ".repeat(filler)));
    let right_start = left_w + filler;
    spans.extend(right);

    StripLine {
        line: Line::from(spans),
        badge: badge_local.map(|r| {
            (right_start + r.start).min(usize::from(width)) as u16
                ..(right_start + r.end).min(usize::from(width)) as u16
        }),
    }
}

/// Render the strip into its 1-row rect; returns the badge's absolute
/// column range for click routing.
pub fn render_strip(
    buf: &mut Buffer,
    rect: Rect,
    state: &WallState,
    notice: Option<&str>,
) -> Option<Range<u16>> {
    let built = strip_line(state, notice, rect.width);
    Paragraph::new(built.line)
        .style(Style::default().bg(Color::Black))
        .render(rect, buf);
    built.badge.map(|r| rect.x + r.start..rect.x + r.end)
}

#[cfg(test)]
mod tests {
    //! Strip content (port of `tui/test/strip_test.dart` semantics): tab
    //! text with amber dots, the blocked badge and its click range, chip
    //! variants, the notice slot.
    use super::*;
    use crate::api::models::{SessionInfo, UsageWindow, WorkspaceInfo};
    use crate::state::store::WallStore;

    fn ws(name: &str) -> WorkspaceInfo {
        WorkspaceInfo {
            name: name.to_owned(),
            dir: Some(format!("/repos/{name}")),
            branch: None,
        }
    }

    fn si(workspace: &str, label: &str, status: &str) -> SessionInfo {
        SessionInfo {
            id: format!("garage/{workspace}/{label}"),
            workspace: workspace.to_owned(),
            label: label.to_owned(),
            dir: None,
            attached: false,
            status: status.to_owned(),
            since: Some(0),
            message: None,
            branch: None,
            restorable: false,
            title: None,
            context: None,
        }
    }

    fn store() -> WallStore {
        let mut store = WallStore::new();
        store.workspaces_fetched(vec![ws("alpha"), ws("beta")]);
        store.sessions_fetched(vec![
            si("alpha", "one", "working"),
            si("beta", "blocked", "needs-input"),
        ]);
        store
    }

    #[test]
    fn tabs_follow_salience_order_with_amber_dots() {
        let store = store();
        let s = strip_line(store.state(), None, 120);
        let text = s.line.to_string();
        // beta is blocked → salience puts it first, with the dot.
        assert!(text.starts_with(" 1:beta● 2:alpha"), "{text}");
    }

    #[test]
    fn badge_and_chip_sit_at_the_right_edge() {
        let store = store();
        let s = strip_line(store.state(), None, 120);
        let text = s.line.to_string();
        assert_eq!(text.chars().count(), 120);
        assert!(text.ends_with(" ● 1 blocked  keys → garage "), "{text}");
        let badge = s.badge.unwrap();
        let badge_text: String = text
            .chars()
            .skip(usize::from(badge.start))
            .take(usize::from(badge.end - badge.start))
            .collect();
        assert_eq!(badge_text, " ● 1 blocked ", "range matches the paint");
    }

    #[test]
    fn no_badge_when_nothing_is_blocked() {
        let mut store = WallStore::new();
        store.workspaces_fetched(vec![ws("a")]);
        store.sessions_fetched(vec![si("a", "one", "working")]);
        let s = strip_line(store.state(), None, 80);
        assert!(s.badge.is_none());
        assert!(!s.line.to_string().contains("blocked"));
    }

    #[test]
    fn engaged_chip_is_black_on_amber() {
        let mut store = store();
        store.focus_session("garage/beta/blocked");
        store.engage();
        let s = strip_line(store.state(), None, 120);
        let text = s.line.to_string();
        assert!(text.ends_with(" keys → beta/blocked "), "{text}");
        let chip = s.line.spans.last().unwrap();
        assert_eq!(chip.style.bg, Some(colors::AMBER));
        assert_eq!(chip.style.fg, Some(Color::Black));
    }

    #[test]
    fn notice_renders_before_the_badge() {
        let store = store();
        let s = strip_line(store.state(), Some("closed ghost"), 120);
        let text = s.line.to_string();
        assert!(
            text.ends_with(" closed ghost  ● 1 blocked  keys → garage "),
            "{text}"
        );
    }

    #[test]
    fn overflow_keeps_positions_consistent_with_the_clipped_paint() {
        let store = store();
        let s = strip_line(store.state(), Some("a very long notice that overflows"), 30);
        // Whatever fits, the badge range never exceeds the width.
        if let Some(badge) = s.badge {
            assert!(badge.end <= 30);
        }
    }

    // ── p11: usage chip (spec tui-context-meters "Strip usage chip") ───────

    #[test]
    fn usage_chip_hidden_entirely_when_both_windows_are_null() {
        let store = store(); // fresh: no /api/usage post yet
        let text = strip_line(store.state(), None, 120).line.to_string();
        assert!(!text.contains("5h"));
        assert!(!text.contains("wk"));
    }

    #[test]
    fn usage_chip_shows_both_windows_dim() {
        let mut store = store();
        store.usage_fetched(UsageInfo {
            five_hour: Some(UsageWindow { used_percentage: 24, resets_at: None }),
            seven_day: Some(UsageWindow { used_percentage: 61, resets_at: None }),
        });
        let s = strip_line(store.state(), None, 120);
        let text = s.line.to_string();
        assert!(text.contains("5h 24% · wk 61%"), "{text}");
        let chip_span = s
            .line
            .spans
            .iter()
            .find(|sp| sp.content.contains("5h 24%"))
            .unwrap();
        assert_eq!(chip_span.style.fg, Some(colors::DIM));
    }

    #[test]
    fn usage_chip_omits_a_null_window() {
        let mut five_only = store();
        five_only.usage_fetched(UsageInfo {
            five_hour: Some(UsageWindow { used_percentage: 24, resets_at: None }),
            seven_day: None,
        });
        let text = strip_line(five_only.state(), None, 120).line.to_string();
        assert!(text.contains("5h 24%"), "{text}");
        assert!(!text.contains("wk"), "{text}");

        let mut week_only = store();
        week_only.usage_fetched(UsageInfo {
            five_hour: None,
            seven_day: Some(UsageWindow { used_percentage: 61, resets_at: None }),
        });
        let text = strip_line(week_only.state(), None, 120).line.to_string();
        assert!(text.contains("wk 61%"), "{text}");
        assert!(!text.contains("5h"), "{text}");
    }

    #[test]
    fn usage_chip_never_displaces_the_notice_badge_or_keys_chip() {
        let mut store = store(); // one blocked session (beta/blocked) baked in
        store.usage_fetched(UsageInfo {
            five_hour: Some(UsageWindow { used_percentage: 24, resets_at: None }),
            seven_day: Some(UsageWindow { used_percentage: 61, resets_at: None }),
        });
        let s = strip_line(store.state(), Some("closed ghost"), 140);
        let text = s.line.to_string();
        assert!(text.contains("closed ghost"), "{text}");
        assert!(text.contains("5h 24% · wk 61%"), "{text}");
        assert!(
            text.ends_with(" ● 1 blocked  keys → garage "),
            "badge and keys chip still land at the very end: {text}"
        );
        let badge = s.badge.unwrap();
        let badge_text: String = text
            .chars()
            .skip(usize::from(badge.start))
            .take(usize::from(badge.end - badge.start))
            .collect();
        assert_eq!(badge_text, " ● 1 blocked ", "badge range still matches the paint");
    }

    // ── p11: install hint styling (spec tui-context-meters "Install
    // affordance") ──────────────────────────────────────────────────────

    #[test]
    fn install_hint_notice_renders_dim_never_bright_or_amber() {
        let store = store();
        let s = strip_line(store.state(), Some(STATUSLINE_HINT), 120);
        let hint_span = s
            .line
            .spans
            .iter()
            .find(|sp| sp.content.contains("press I"))
            .unwrap();
        assert_eq!(hint_span.style.fg, Some(colors::DIM));
        assert_ne!(hint_span.style.fg, Some(colors::AMBER));
    }

    #[test]
    fn an_ordinary_notice_still_renders_bright() {
        let store = store();
        let s = strip_line(store.state(), Some("closed ghost"), 120);
        let notice_span = s
            .line
            .spans
            .iter()
            .find(|sp| sp.content.contains("closed ghost"))
            .unwrap();
        assert_eq!(notice_span.style.fg, Some(colors::FG));
    }
}
