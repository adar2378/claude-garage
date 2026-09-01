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

/// The pit pet's current frame, as the render layer needs it (spec
/// tui-pit-pet "One-row sprites and derived mood" / "Species-true one-row
/// motion") — built by `runtime.rs`'s tick pass from a `pet::StepResult`,
/// consumed here as plain data so `strip.rs` stays free of `ui::pet`
/// simulation logic (it only lays the sprite out).
#[derive(Debug, Clone)]
pub struct PetRender {
    /// The sprite frame's text (`pet::Sprite::text` — always single-width
    /// ASCII/measured glyphs, see `pet.rs`'s width-measurement note).
    pub text: &'static str,
    /// Whether this mood carries a `!` column at all (reserved even unlit,
    /// so the strip never jitters as the bang blinks).
    pub bang: bool,
    /// Whether the `!` is lit THIS frame (amber bold) vs reserved-but-blank.
    pub bang_lit: bool,
    /// Strip-local column, within the filler, where the sprite starts.
    pub x: u16,
    /// True while the mood is Sleep or Box — rendered dim instead of FG.
    pub dim: bool,
    /// A chatter/petting line to draw beside the sprite (spec tui-pit-pet
    /// "Species-voiced chatter"): left of the sprite when there's room,
    /// else right of it, else dropped for the frame — never in the
    /// right-aligned notice slot, which can be a hundred columns away.
    pub say: Option<String>,
}

/// Both click-routed column ranges the strip paints, plus the filler width
/// the pet tick needs for its next `PetSim::step` (spec tui-pit-pet "Narrow
/// strip hides the pet" — the tick has to know how much room there'll be).
pub struct StripCols {
    /// Absolute column range of the blocked badge, when rendered.
    pub badge: Option<Range<u16>>,
    /// Absolute column range of the pet sprite (sprite + reserved bang
    /// column), when rendered this frame.
    pub pet: Option<Range<u16>>,
    /// The filler's width this frame — the room available for the pet,
    /// independent of whether a pet is currently selected.
    pub filler_width: u16,
}

pub struct StripLine {
    pub line: Line<'static>,
    /// Column range (strip-local) of the blocked badge, when rendered.
    pub badge: Option<Range<u16>>,
    /// Column range (strip-local) of the pet sprite (incl. its reserved bang
    /// column), when one was given and there was room for it.
    pub pet: Option<Range<u16>>,
    /// The filler's width this frame, regardless of whether a pet occupies
    /// any of it (spec tui-pit-pet: the tick pass needs this for the next
    /// step's `filler_width`).
    pub filler_width: u16,
}

/// Build the strip's line for `width` columns: tabs left, then a filler
/// (with the pet inserted into it, when given and there's room), then
/// notice · badge · chip right-aligned.
pub fn strip_line(state: &WallState, notice: Option<&str>, width: u16, pet: Option<PetRender>) -> StripLine {
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
    // Pit pet (spec tui-pit-pet "Pet lives in the strip filler"): laid out
    // left-anchored at `pet.x` within the filler, sprite then a reserved
    // bang column, then the rest of the filler as before. Skipped for this
    // frame — falling back to a plain filler span — when there isn't room
    // (spec "Narrow strip hides the pet").
    let mut pet_local: Option<Range<usize>> = None;
    match pet {
        Some(p) => {
            let sprite_width = p.text.chars().count();
            let bang_width = if p.bang { 2 } else { 0 }; // " " + "!" (or blank)
            let required = sprite_width + bang_width;
            // One blank gutter column on each side so the pet never touches
            // the tabs or the right-hand cluster.
            if filler >= required + 2 {
                let x = 1 + usize::from(p.x).min(filler - required - 2);
                let after = filler - x - required;
                // Speech placement: prefer the side with room, left first
                // (the pet's home is the right end, so text reads toward it).
                let say = p.say.as_deref().filter(|t| !t.is_empty());
                let say_w = say.map_or(0, |t| t.chars().count() + 2);
                let (say_left, say_right) = match say {
                    Some(t) if say_w <= x - 1 => (Some(t), None),
                    Some(t) if say_w <= after.saturating_sub(1) => (None, Some(t)),
                    _ => (None, None),
                };
                let say_style = Style::default().fg(colors::FG);
                if let Some(t) = say_left {
                    spans.push(Span::raw(" ".repeat(x - say_w)));
                    spans.push(Span::styled(t.to_owned(), say_style));
                    spans.push(Span::raw("  "));
                } else {
                    spans.push(Span::raw(" ".repeat(x)));
                }
                let sprite_style =
                    Style::default().fg(if p.dim { colors::DIM } else { colors::FG });
                spans.push(Span::styled(p.text, sprite_style));
                pet_local = Some(x..x + required);
                if p.bang {
                    spans.push(Span::raw(" "));
                    if p.bang_lit {
                        spans.push(Span::styled(
                            "!",
                            Style::default().fg(colors::AMBER).add_modifier(Modifier::BOLD),
                        ));
                    } else {
                        // Unlit: still a column-wide space, so the bang's
                        // blink never shifts anything else (spec "bang unlit
                        // still reserves the column").
                        spans.push(Span::raw(" "));
                    }
                }
                if let Some(t) = say_right {
                    spans.push(Span::raw("  "));
                    spans.push(Span::styled(t.to_owned(), say_style));
                    spans.push(Span::raw(" ".repeat(after - say_w)));
                } else {
                    spans.push(Span::raw(" ".repeat(after)));
                }
            } else {
                spans.push(Span::raw(" ".repeat(filler)));
            }
        }
        None => spans.push(Span::raw(" ".repeat(filler))),
    }
    let right_start = left_w + filler;
    spans.extend(right);

    StripLine {
        line: Line::from(spans),
        badge: badge_local.map(|r| {
            (right_start + r.start).min(usize::from(width)) as u16
                ..(right_start + r.end).min(usize::from(width)) as u16
        }),
        pet: pet_local.map(|r| {
            (left_w + r.start).min(usize::from(width)) as u16
                ..(left_w + r.end).min(usize::from(width)) as u16
        }),
        filler_width: filler as u16,
    }
}

/// Render the strip into its 1-row rect; returns the badge's and pet's
/// absolute column ranges for click routing, plus the filler width the
/// pet's next tick needs (spec tui-pit-pet).
pub fn render_strip(
    buf: &mut Buffer,
    rect: Rect,
    state: &WallState,
    notice: Option<&str>,
    pet: Option<PetRender>,
) -> StripCols {
    let built = strip_line(state, notice, rect.width, pet);
    Paragraph::new(built.line)
        .style(Style::default().bg(Color::Black))
        .render(rect, buf);
    StripCols {
        badge: built.badge.map(|r| rect.x + r.start..rect.x + r.end),
        pet: built.pet.map(|r| rect.x + r.start..rect.x + r.end),
        filler_width: built.filler_width,
    }
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
        let s = strip_line(store.state(), None, 120, None);
        let text = s.line.to_string();
        // beta is blocked → salience puts it first, with the dot.
        assert!(text.starts_with(" 1:beta● 2:alpha"), "{text}");
    }

    #[test]
    fn badge_and_chip_sit_at_the_right_edge() {
        let store = store();
        let s = strip_line(store.state(), None, 120, None);
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
        let s = strip_line(store.state(), None, 80, None);
        assert!(s.badge.is_none());
        assert!(!s.line.to_string().contains("blocked"));
    }

    #[test]
    fn engaged_chip_is_black_on_amber() {
        let mut store = store();
        store.focus_session("garage/beta/blocked");
        store.engage();
        let s = strip_line(store.state(), None, 120, None);
        let text = s.line.to_string();
        assert!(text.ends_with(" keys → beta/blocked "), "{text}");
        let chip = s.line.spans.last().unwrap();
        assert_eq!(chip.style.bg, Some(colors::AMBER));
        assert_eq!(chip.style.fg, Some(Color::Black));
    }

    #[test]
    fn notice_renders_before_the_badge() {
        let store = store();
        let s = strip_line(store.state(), Some("closed ghost"), 120, None);
        let text = s.line.to_string();
        assert!(
            text.ends_with(" closed ghost  ● 1 blocked  keys → garage "),
            "{text}"
        );
    }

    #[test]
    fn overflow_keeps_positions_consistent_with_the_clipped_paint() {
        let store = store();
        let s = strip_line(store.state(), Some("a very long notice that overflows"), 30, None);
        // Whatever fits, the badge range never exceeds the width.
        if let Some(badge) = s.badge {
            assert!(badge.end <= 30);
        }
    }

    // ── p11: usage chip (spec tui-context-meters "Strip usage chip") ───────

    #[test]
    fn usage_chip_hidden_entirely_when_both_windows_are_null() {
        let store = store(); // fresh: no /api/usage post yet
        let text = strip_line(store.state(), None, 120, None).line.to_string();
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
        let s = strip_line(store.state(), None, 120, None);
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
        let text = strip_line(five_only.state(), None, 120, None).line.to_string();
        assert!(text.contains("5h 24%"), "{text}");
        assert!(!text.contains("wk"), "{text}");

        let mut week_only = store();
        week_only.usage_fetched(UsageInfo {
            five_hour: None,
            seven_day: Some(UsageWindow { used_percentage: 61, resets_at: None }),
        });
        let text = strip_line(week_only.state(), None, 120, None).line.to_string();
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
        let s = strip_line(store.state(), Some("closed ghost"), 140, None);
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
        let s = strip_line(store.state(), Some(STATUSLINE_HINT), 120, None);
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
        let s = strip_line(store.state(), Some("closed ghost"), 120, None);
        let notice_span = s
            .line
            .spans
            .iter()
            .find(|sp| sp.content.contains("closed ghost"))
            .unwrap();
        assert_eq!(notice_span.style.fg, Some(colors::FG));
    }

    // ── p15: pit pet (spec tui-pit-pet) ─────────────────────────────────

    fn pet_render(x: u16) -> PetRender {
        PetRender { text: "=o.o=", bang: false, bang_lit: false, x, dim: false, say: None }
    }

    #[test]
    fn pet_renders_at_200_cols_with_correct_columns() {
        let store = store();
        let s = strip_line(store.state(), None, 200, Some(pet_render(3)));
        let pet = s.pet.expect("plenty of filler at 200 cols");
        let text = s.line.to_string();
        let sprite: String = text
            .chars()
            .skip(usize::from(pet.start))
            .take(usize::from(pet.end - pet.start))
            .collect();
        assert_eq!(sprite, "=o.o=", "no bang: the reserved range is exactly the sprite text");
    }

    #[test]
    fn pet_at_x_zero_still_leaves_a_gutter_after_the_tabs() {
        let store = store();
        let s = strip_line(store.state(), None, 200, Some(pet_render(0)));
        let pet = s.pet.expect("room at 200 cols");
        let text: Vec<char> = s.line.to_string().chars().collect();
        let start = usize::from(pet.start);
        assert_eq!(text[start - 1], ' ', "one blank column before the sprite");
        assert_ne!(text[start - 2], ' ', "…and the tab text sits right before that gutter");
    }

    #[test]
    fn speech_sits_left_of_a_right_homed_pet_and_right_of_a_left_one() {
        let store = store();
        let mut right_home = pet_render(150);
        right_home.say = Some("you're doing great!!".to_owned());
        let s = strip_line(store.state(), None, 200, Some(right_home));
        let text = s.line.to_string();
        let pet = s.pet.expect("room at 200 cols");
        let before: String = text.chars().take(usize::from(pet.start)).collect();
        assert!(before.trim_end().ends_with("you're doing great!!"), "speech reads toward the sprite: {before:?}");
        assert!(before.ends_with("  "), "two-column gap between speech and sprite");

        let mut left_home = pet_render(0);
        left_home.say = Some("quack.".to_owned());
        let s = strip_line(store.state(), None, 200, Some(left_home));
        let text = s.line.to_string();
        let pet = s.pet.expect("room at 200 cols");
        let after: String = text.chars().skip(usize::from(pet.end)).collect();
        assert!(after.starts_with("  quack."), "no room on the left: speech goes right: {after:?}");
    }

    #[test]
    fn speech_is_dropped_but_the_sprite_stays_when_neither_side_fits() {
        let store = store();
        let mut p = pet_render(0);
        p.say = Some("x".repeat(400));
        let s = strip_line(store.state(), None, 200, Some(p));
        let text = s.line.to_string();
        assert!(s.pet.is_some(), "sprite still renders");
        assert!(!text.contains("xxxx"), "oversized speech dropped for the frame");
    }

    #[test]
    fn pet_hidden_at_80_cols_with_six_workspaces_and_a_notice() {
        let mut store = WallStore::new();
        store.workspaces_fetched((1..=6).map(|i| ws(&format!("ws{i}"))).collect());
        store.sessions_fetched(
            (1..=6)
                .map(|i| si(&format!("ws{i}"), "one", "working"))
                .collect(),
        );
        let s = strip_line(
            store.state(),
            Some("a longish strip notice sits here"),
            80,
            Some(pet_render(0)),
        );
        assert!(s.pet.is_none(), "six tabs + a notice leave no filler room at 80 cols");
        assert!(!s.line.to_string().contains("=o.o="));
    }

    #[test]
    fn badge_range_unaffected_by_pet_presence() {
        let store = store(); // one blocked session baked in (beta/blocked)
        let without = strip_line(store.state(), None, 120, None);
        let with = strip_line(store.state(), None, 120, Some(pet_render(2)));
        assert_eq!(without.badge, with.badge, "badge sits at the same absolute columns either way");
    }

    #[test]
    fn bang_unlit_still_reserves_the_column() {
        let store = store();
        let lit = PetRender { text: "(O.O)", bang: true, bang_lit: true, x: 0, dim: false, say: None };
        let unlit = PetRender { text: "(O.O)", bang: true, bang_lit: false, x: 0, dim: false, say: None };
        let s_lit = strip_line(store.state(), None, 120, Some(lit));
        let s_unlit = strip_line(store.state(), None, 120, Some(unlit));
        assert_eq!(s_lit.pet, s_unlit.pet, "the bang column's width never changes on blink");
        assert_eq!(s_lit.badge, s_unlit.badge, "and nothing downstream shifts either");
    }
}
