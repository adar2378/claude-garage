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

use crate::slots::{self, ClickTarget, Placement, SlotCtx, SlotId, SlotItem, SlotRegistry};
use crate::state::wall_state::{KeyLayer, WallState};
use crate::ui::theme::colors;

/// The one-time install hint's exact wording (spec tui-context-meters
/// "Install affordance", spec tui-hooks-install "Install hint mentions
/// hooks": `I` installs hooks and the context meter) — rendered dim, never
/// amber; the
/// runtime shows it through the same notice slot as any other strip notice
/// (see `runtime.rs`'s hint scheduling), so `strip_line` special-cases this
/// text to pick the dim color instead of an ordinary notice's bright one.
pub const STATUSLINE_HINT: &str = "press I to install hooks (instant status) + context meter";

/// Where each filler item lands within a filler `filler` columns wide:
/// `(offset, item)` pairs in column order. An item needs its width plus a
/// blank column on each side; it is left-anchored at its `x`, clamped so it
/// still fits. Where two overlap the lower-priority one is dropped (a tie
/// drops the later), so a filler item never overwrites another or a flow
/// item.
fn place_fillers(filler: usize, items: Vec<SlotItem>) -> Vec<(usize, SlotItem)> {
    let mut candidates: Vec<(usize, SlotItem)> = items
        .into_iter()
        .filter(|item| filler >= item.width() + 2)
        .map(|item| {
            let Placement::Filler { x, .. } = &item.placement else {
                unreachable!("only filler items are placed");
            };
            let x = 1 + usize::from(*x).min(filler - item.width() - 2);
            (x, item)
        })
        .collect();
    // Stable: equal columns keep their provider order, so "later" is later.
    candidates.sort_by_key(|(x, _)| *x);
    let mut kept: Vec<(usize, SlotItem)> = Vec::new();
    for (x, item) in candidates {
        let mut item = Some(item);
        while let (Some(it), Some((px, prev))) = (item.as_ref(), kept.last()) {
            if *px + prev.width() <= x {
                break; // clear of the previous item
            }
            if it.priority > prev.priority {
                kept.pop(); // the newcomer wins; check against the one before
            } else {
                item = None; // the earlier one holds (ties drop the later)
            }
        }
        if let Some(item) = item {
            kept.push((x, item));
        }
    }
    kept
}

/// The click-routed column ranges the strip paints, and the filler's span
/// (reported to plugins as the room a `strip.right` filler item has).
pub struct StripCols {
    /// Absolute column range of the blocked badge, when rendered.
    pub badge: Option<Range<u16>>,
    /// Absolute click ranges of the clickable `strip.right` slot items.
    pub slots: Vec<(Range<u16>, ClickTarget)>,
    /// The filler's absolute column range (`x0..x1`); empty when there is no
    /// room.
    pub filler: Range<u16>,
}

pub struct StripLine {
    pub line: Line<'static>,
    /// Column range (strip-local) of the blocked badge, when rendered.
    pub badge: Option<Range<u16>>,
    /// Strip-local click ranges of the clickable `strip.right` slot items.
    pub slots: Vec<(Range<u16>, ClickTarget)>,
    /// Strip-local column range of the filler.
    pub filler: Range<u16>,
}

/// Build the strip's line for `width` columns: tabs left, then a filler
/// (with a floated `strip.right` item such as a plugin's pet inserted into it,
/// when there's room), then
/// notice · badge · chip right-aligned.
pub fn strip_line(
    state: &WallState,
    slots: &SlotRegistry,
    notice: Option<&str>,
    width: u16,
) -> StripLine {
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
        let color = if notice == STATUSLINE_HINT {
            colors::DIM
        } else {
            colors::FG
        };
        right.push(Span::styled(
            format!(" {notice} "),
            Style::default().fg(color),
        ));
    }
    // Usage chip (spec tui-context-meters "Strip usage chip"): additive only
    // — inserted between the notice and the badge/keys chip so it never
    // displaces either of them.
    let slot_ctx = SlotCtx {
        state: Some(state),
        ..SlotCtx::default()
    };
    let notice_w: usize = right.iter().map(Span::width).sum();
    let (floating, flow): (Vec<SlotItem>, Vec<SlotItem>) = slots
        .items(SlotId::StripRight, &slot_ctx)
        .into_iter()
        .partition(|item| matches!(item.placement, Placement::Filler { .. }));
    let laid = slots::layout(flow, None);
    right.extend(laid.spans.iter().cloned());
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
    // Filler items (the pit pet, and any a plugin adds): each is left-anchored
    // at its `x` within the filler, flanked by a blank gutter column, with its
    // caption beside it where there is room. An item that does not fit, or
    // that would overlap a higher-priority one, is skipped for this frame —
    // the filler stays a plain span (spec tui-pit-pet "Narrow strip hides the
    // pet").
    let mut click_ranges: Vec<(Range<u16>, ClickTarget)> = Vec::new();
    let placed = place_fillers(filler, floating);
    let say_style = Style::default().fg(colors::FG);
    let mut cursor = 0usize; // filler columns already emitted
    let mut right_used = 0usize; // columns the previous item's caption took after it
    for (i, (x, item)) in placed.iter().enumerate() {
        let Placement::Filler { caption, .. } = &item.placement else {
            unreachable!("only filler items are placed");
        };
        let required = item.width();
        let next_start = placed.get(i + 1).map_or(filler, |(nx, _)| *nx);
        let before = x - cursor - right_used;
        let after = next_start - (x + required);
        // Speech placement: prefer the side with room, left first (the pet's
        // home is the right end, so text reads toward it).
        let say = caption.as_deref().filter(|t| !t.is_empty());
        let say_w = say.map_or(0, |t| t.chars().count() + 2);
        let (say_left, say_right) = match say {
            Some(t) if say_w < before => (Some(t), None),
            Some(t) if say_w <= after.saturating_sub(1) => (None, Some(t)),
            _ => (None, None),
        };
        if let Some(t) = say_left {
            spans.push(Span::raw(" ".repeat(before - say_w)));
            spans.push(Span::styled(t.to_owned(), say_style));
            spans.push(Span::raw("  "));
        } else {
            spans.push(Span::raw(" ".repeat(before)));
        }
        if let Some(target) = item.on_click.clone() {
            let start = (left_w + x).min(usize::from(width)) as u16;
            let end = (left_w + x + required).min(usize::from(width)) as u16;
            click_ranges.push((start..end, target));
        }
        spans.extend(item.text.iter().cloned());
        right_used = 0;
        if let Some(t) = say_right {
            spans.push(Span::raw("  "));
            spans.push(Span::styled(t.to_owned(), say_style));
            right_used = say_w;
        }
        cursor = x + required;
    }
    spans.push(Span::raw(" ".repeat(filler - cursor - right_used)));
    let right_start = left_w + filler;
    spans.extend(right);

    StripLine {
        line: Line::from(spans),
        badge: badge_local.map(|r| {
            (right_start + r.start).min(usize::from(width)) as u16
                ..(right_start + r.end).min(usize::from(width)) as u16
        }),
        slots: {
            click_ranges.extend(laid.clicks(right_start + notice_w, usize::from(width)));
            click_ranges
        },
        filler: left_w.min(usize::from(width)) as u16
            ..(left_w + filler).min(usize::from(width)) as u16,
    }
}

/// Render the strip into its 1-row rect; returns the badge's and the
/// clickable slot items' absolute column ranges for click routing, plus the
/// filler's span.
pub fn render_strip(
    buf: &mut Buffer,
    rect: Rect,
    state: &WallState,
    slots: &SlotRegistry,
    notice: Option<&str>,
) -> StripCols {
    let built = strip_line(state, slots, notice, rect.width);
    Paragraph::new(built.line)
        .style(Style::default().bg(Color::Black))
        .render(rect, buf);
    StripCols {
        badge: built.badge.map(|r| rect.x + r.start..rect.x + r.end),
        slots: built
            .slots
            .into_iter()
            .map(|(r, target)| (rect.x + r.start..rect.x + r.end, target))
            .collect(),
        filler: rect.x + built.filler.start..rect.x + built.filler.end,
    }
}

#[cfg(test)]
mod tests {
    //! Strip content (port of `tui/test/strip_test.dart` semantics): tab
    //! text with amber dots, the blocked badge and its click range, chip
    //! variants, the notice slot.
    use super::*;
    use crate::actions::{ActionId, ids};
    use crate::api::models::{SessionInfo, WorkspaceInfo};
    use crate::state::store::WallStore;

    /// A sprite-shaped filler item for the layout tests below (the shape the
    /// pit-pet plugin pushes: sprite, reserved bang column, caption).
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

    /// The pit pet as a `strip.right` item: the sprite then a reserved bang
    /// column (reserved even unlit, so the strip never jitters as the bang
    /// blinks), floated into the filler at the sim's `x`, with the chatter line
    /// as its caption. A click on it is `pet.pet`.
    pub fn pet_slot_item(p: &PetRender) -> SlotItem {
        let mut text = vec![Span::styled(
            p.text,
            Style::default().fg(if p.dim { colors::DIM } else { colors::FG }),
        )];
        if p.bang {
            text.push(Span::raw(" "));
            text.push(if p.bang_lit {
                Span::styled(
                    "!",
                    Style::default()
                        .fg(colors::AMBER)
                        .add_modifier(Modifier::BOLD),
                )
            } else {
                // Unlit: still a column-wide space (spec "bang unlit still
                // reserves the column").
                Span::raw(" ")
            });
        }
        SlotItem {
            on_click: Some(ids::VIEW_CYCLE.into()),
            placement: Placement::Filler {
                x: p.x,
                caption: p.say.clone(),
            },
            ..SlotItem::new(text, 100)
        }
    }

    fn ws(name: &str) -> WorkspaceInfo {
        WorkspaceInfo {
            name: name.to_owned(),
            dir: Some(format!("/repos/{name}/{name}")),
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
        let s = strip_line(store.state(), &SlotRegistry::core(), None, 120);
        let text = s.line.to_string();
        // beta is blocked → salience puts it first, with the dot.
        assert!(text.starts_with(" 1:beta● 2:alpha"), "{text}");
    }

    #[test]
    fn badge_and_chip_sit_at_the_right_edge() {
        let store = store();
        let s = strip_line(store.state(), &SlotRegistry::core(), None, 120);
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
        let s = strip_line(store.state(), &SlotRegistry::core(), None, 80);
        assert!(s.badge.is_none());
        assert!(!s.line.to_string().contains("blocked"));
    }

    #[test]
    fn engaged_chip_is_black_on_amber() {
        let mut store = store();
        store.focus_session("garage/beta/blocked");
        store.engage();
        let s = strip_line(store.state(), &SlotRegistry::core(), None, 120);
        let text = s.line.to_string();
        assert!(text.ends_with(" keys → beta/blocked "), "{text}");
        let chip = s.line.spans.last().unwrap();
        assert_eq!(chip.style.bg, Some(colors::AMBER));
        assert_eq!(chip.style.fg, Some(Color::Black));
    }

    #[test]
    fn notice_renders_before_the_badge() {
        let store = store();
        let s = strip_line(
            store.state(),
            &SlotRegistry::core(),
            Some("closed ghost"),
            120,
        );
        let text = s.line.to_string();
        assert!(
            text.ends_with(" closed ghost  ● 1 blocked  keys → garage "),
            "{text}"
        );
    }

    #[test]
    fn overflow_keeps_positions_consistent_with_the_clipped_paint() {
        let store = store();
        let s = strip_line(
            store.state(),
            &SlotRegistry::core(),
            Some("a very long notice that overflows"),
            30,
        );
        // Whatever fits, the badge range never exceeds the width.
        if let Some(badge) = s.badge {
            assert!(badge.end <= 30);
        }
    }

    // ── p25: the usage chip is the usage-bar plugin's flow item ─────────────

    /// The usage-bar plugin's golden widget body (also asserted by the plugin
    /// crate, so the two cannot drift).
    const USAGE_BOTH: &str = include_str!("../../../plugins/usage-bar/golden/both.json");

    fn usage_registry() -> SlotRegistry {
        crate::plugins::test_support::registry_with(
            "usage-bar",
            &[],
            garage_protocol::widgets::WidgetSlot::StripRight,
            USAGE_BOTH,
        )
    }

    #[test]
    fn no_plugin_no_chip_and_the_core_registry_has_none() {
        let store = store();
        let text = strip_line(store.state(), &SlotRegistry::core(), None, 120)
            .line
            .to_string();
        assert!(!text.contains("5h"));
        assert!(!text.contains("wk"));
    }

    #[test]
    fn usage_chip_from_the_plugin_is_dim_and_never_displaces_the_notice_badge_or_keys_chip() {
        let store = store(); // one blocked session (beta/blocked) baked in
        let s = strip_line(store.state(), &usage_registry(), Some("closed ghost"), 140);
        let text = s.line.to_string();
        assert!(text.contains("closed ghost"), "{text}");
        assert!(text.contains("5h 24% · wk 61%"), "{text}");
        let chip = s
            .line
            .spans
            .iter()
            .find(|sp| sp.content.contains("5h 24%"))
            .unwrap();
        assert_eq!(chip.style.fg, Some(colors::DIM));
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
        assert_eq!(
            badge_text, " ● 1 blocked ",
            "badge range still matches the paint"
        );
    }

    // ── p11: install hint styling (spec tui-context-meters "Install
    // affordance") ──────────────────────────────────────────────────────

    #[test]
    fn install_hint_notice_renders_dim_never_bright_or_amber() {
        let store = store();
        let s = strip_line(
            store.state(),
            &SlotRegistry::core(),
            Some(STATUSLINE_HINT),
            120,
        );
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
        let s = strip_line(
            store.state(),
            &SlotRegistry::core(),
            Some("closed ghost"),
            120,
        );
        let notice_span = s
            .line
            .spans
            .iter()
            .find(|sp| sp.content.contains("closed ghost"))
            .unwrap();
        assert_eq!(notice_span.style.fg, Some(colors::FG));
    }

    // ── p15: pit pet (spec tui-pit-pet) ─────────────────────────────────

    /// The core registry plus a provider that serves `p` as the pet's item.
    fn with_pet(p: PetRender) -> SlotRegistry {
        let mut registry = SlotRegistry::core();
        registry.register(SlotId::StripRight, move |_: &SlotCtx| {
            vec![pet_slot_item(&p)]
        });
        registry
    }

    /// The pet's click range, when the strip showed it.
    fn pet_range(s: &StripLine) -> Option<Range<u16>> {
        s.slots
            .iter()
            .find(|(_, t)| t.action == ids::VIEW_CYCLE)
            .map(|(r, _)| r.clone())
    }

    fn pet_render(x: u16) -> PetRender {
        PetRender {
            text: "=o.o=",
            bang: false,
            bang_lit: false,
            x,
            dim: false,
            say: None,
        }
    }

    // ── p24: more than one filler item ──────────────────────────────────

    fn filler_item(text: &str, x: u16, priority: i16, caption: Option<&str>) -> SlotItem {
        SlotItem {
            on_click: Some(ClickTarget::new(ActionId::intern(&format!(
                "t:{}",
                &text[..text.len().min(8)]
            )))),
            placement: Placement::Filler {
                x,
                caption: caption.map(str::to_owned),
            },
            ..SlotItem::new(vec![Span::raw(text.to_owned())], priority)
        }
    }

    fn with_fillers(items: Vec<SlotItem>) -> SlotRegistry {
        let mut registry = SlotRegistry::core();
        registry.register(SlotId::StripRight, move |_: &SlotCtx| items.clone());
        registry
    }

    /// The text under each filler item's click range.
    fn painted(s: &StripLine) -> Vec<String> {
        let chars: Vec<char> = s.line.to_string().chars().collect();
        s.slots
            .iter()
            .map(|(r, _)| {
                chars[usize::from(r.start)..usize::from(r.end)]
                    .iter()
                    .collect()
            })
            .collect()
    }

    #[test]
    fn two_fillers_both_show_in_column_order_each_at_its_own_x() {
        let store = store();
        // Provided out of order: the strip sorts by x.
        let registry = with_fillers(vec![
            filler_item("RIGHT", 90, 1, None),
            filler_item("LEFT", 5, 1, None),
        ]);
        let s = strip_line(store.state(), &registry, None, 200);
        assert_eq!(painted(&s), ["LEFT", "RIGHT"]);
        let (l, r) = (&s.slots[0].0, &s.slots[1].0);
        assert_eq!(r.start - l.start, 90 - 5, "each lands at its own x");
    }

    #[test]
    fn overlapping_fillers_drop_the_lower_priority_one() {
        let store = store();
        let high_first = with_fillers(vec![
            filler_item("KEEPER", 10, 9, None),
            filler_item("LOSER", 12, 1, None),
        ]);
        assert_eq!(
            painted(&strip_line(store.state(), &high_first, None, 200)),
            ["KEEPER"]
        );
        // Same overlap, the later column is the higher priority: it wins.
        let high_second = with_fillers(vec![
            filler_item("LOSER", 10, 1, None),
            filler_item("KEEPER", 12, 9, None),
        ]);
        assert_eq!(
            painted(&strip_line(store.state(), &high_second, None, 200)),
            ["KEEPER"]
        );
    }

    #[test]
    fn a_priority_tie_between_overlapping_fillers_keeps_the_earlier() {
        let store = store();
        let registry = with_fillers(vec![
            filler_item("FIRST", 10, 5, None),
            filler_item("SECOND", 12, 5, None),
        ]);
        assert_eq!(
            painted(&strip_line(store.state(), &registry, None, 200)),
            ["FIRST"]
        );
    }

    #[test]
    fn adjacent_fillers_that_just_touch_both_survive() {
        let store = store();
        let registry = with_fillers(vec![
            filler_item("AAAAA", 10, 1, None),
            filler_item("BBBBB", 15, 1, None),
        ]);
        assert_eq!(
            painted(&strip_line(store.state(), &registry, None, 200)),
            ["AAAAA", "BBBBB"]
        );
    }

    #[test]
    fn a_filler_that_does_not_fit_is_skipped_without_hurting_the_others() {
        let store = store();
        let registry = with_fillers(vec![
            filler_item(&"W".repeat(400), 0, 9, None),
            filler_item("OK", 20, 1, None),
        ]);
        assert_eq!(
            painted(&strip_line(store.state(), &registry, None, 200)),
            ["OK"]
        );
    }

    #[test]
    fn captions_stay_in_their_own_gap_between_two_fillers() {
        let store = store();
        let registry = with_fillers(vec![
            filler_item("AA", 2, 1, Some("hello")),
            filler_item("BB", 60, 1, Some("world")),
        ]);
        let s = strip_line(store.state(), &registry, None, 200);
        let text = s.line.to_string();
        assert_eq!(text.chars().count(), 200, "the strip keeps its width");
        let a = usize::from(s.slots[0].0.start);
        let b = usize::from(s.slots[1].0.start);
        let hello = text.find("hello").unwrap();
        let world = text.find("world").unwrap();
        // No room left of A, so its caption reads to its right; B's caption
        // sits to B's left, after A's, without touching it.
        assert!(a < hello && hello + 5 <= world && world + 5 <= b, "{text}");
        assert!(
            text.contains("  hello") && text.contains("world  "),
            "{text}"
        );
    }

    #[test]
    fn pet_alone_is_the_same_as_one_generic_filler() {
        // The single-filler layout the pet snapshots pin must be what the
        // generalised placement produces for one item with a caption.
        let store = store();
        let mut pet = pet_render(30);
        pet.say = Some("hi there".to_owned());
        let a = strip_line(store.state(), &with_pet(pet), None, 150);
        let b = strip_line(
            store.state(),
            &with_fillers(vec![SlotItem {
                on_click: Some(ids::VIEW_CYCLE.into()),
                placement: Placement::Filler {
                    x: 30,
                    caption: Some("hi there".into()),
                },
                ..SlotItem::new(
                    vec![Span::styled("=o.o=", Style::default().fg(colors::FG))],
                    100,
                )
            }]),
            None,
            150,
        );
        assert_eq!(a.line.to_string(), b.line.to_string());
        assert_eq!(a.slots, b.slots);
    }

    #[test]
    fn pet_renders_at_200_cols_with_correct_columns() {
        let store = store();
        let s = strip_line(store.state(), &with_pet(pet_render(3)), None, 200);
        let pet = pet_range(&s).expect("plenty of filler at 200 cols");
        let text = s.line.to_string();
        let sprite: String = text
            .chars()
            .skip(usize::from(pet.start))
            .take(usize::from(pet.end - pet.start))
            .collect();
        assert_eq!(
            sprite, "=o.o=",
            "no bang: the reserved range is exactly the sprite text"
        );
    }

    #[test]
    fn pet_at_x_zero_still_leaves_a_gutter_after_the_tabs() {
        let store = store();
        let s = strip_line(store.state(), &with_pet(pet_render(0)), None, 200);
        let pet = pet_range(&s).expect("room at 200 cols");
        let text: Vec<char> = s.line.to_string().chars().collect();
        let start = usize::from(pet.start);
        assert_eq!(text[start - 1], ' ', "one blank column before the sprite");
        assert_ne!(
            text[start - 2],
            ' ',
            "…and the tab text sits right before that gutter"
        );
    }

    #[test]
    fn speech_sits_left_of_a_right_homed_pet_and_right_of_a_left_one() {
        let store = store();
        let mut right_home = pet_render(150);
        right_home.say = Some("you're doing great!!".to_owned());
        let s = strip_line(store.state(), &with_pet(right_home), None, 200);
        let text = s.line.to_string();
        let pet = pet_range(&s).expect("room at 200 cols");
        let before: String = text.chars().take(usize::from(pet.start)).collect();
        assert!(
            before.trim_end().ends_with("you're doing great!!"),
            "speech reads toward the sprite: {before:?}"
        );
        assert!(
            before.ends_with("  "),
            "two-column gap between speech and sprite"
        );

        let mut left_home = pet_render(0);
        left_home.say = Some("quack.".to_owned());
        let s = strip_line(store.state(), &with_pet(left_home), None, 200);
        let text = s.line.to_string();
        let pet = pet_range(&s).expect("room at 200 cols");
        let after: String = text.chars().skip(usize::from(pet.end)).collect();
        assert!(
            after.starts_with("  quack."),
            "no room on the left: speech goes right: {after:?}"
        );
    }

    #[test]
    fn speech_is_dropped_but_the_sprite_stays_when_neither_side_fits() {
        let store = store();
        let mut p = pet_render(0);
        p.say = Some("x".repeat(400));
        let s = strip_line(store.state(), &with_pet(p), None, 200);
        let text = s.line.to_string();
        assert!(pet_range(&s).is_some(), "sprite still renders");
        assert!(
            !text.contains("xxxx"),
            "oversized speech dropped for the frame"
        );
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
            &with_pet(pet_render(0)),
            Some("a longish strip notice sits here"),
            80,
        );
        assert!(
            pet_range(&s).is_none(),
            "six tabs + a notice leave no filler room at 80 cols"
        );
        assert!(!s.line.to_string().contains("=o.o="));
    }

    #[test]
    fn badge_range_unaffected_by_pet_presence() {
        let store = store(); // one blocked session baked in (beta/blocked)
        let without = strip_line(store.state(), &SlotRegistry::core(), None, 120);
        let with = strip_line(store.state(), &with_pet(pet_render(2)), None, 120);
        assert_eq!(
            without.badge, with.badge,
            "badge sits at the same absolute columns either way"
        );
    }

    #[test]
    fn bang_unlit_still_reserves_the_column() {
        let store = store();
        let lit = PetRender {
            text: "(O.O)",
            bang: true,
            bang_lit: true,
            x: 0,
            dim: false,
            say: None,
        };
        let unlit = PetRender {
            text: "(O.O)",
            bang: true,
            bang_lit: false,
            x: 0,
            dim: false,
            say: None,
        };
        let s_lit = strip_line(store.state(), &with_pet(lit), None, 120);
        let s_unlit = strip_line(store.state(), &with_pet(unlit), None, 120);
        assert_eq!(
            pet_range(&s_lit),
            pet_range(&s_unlit),
            "the bang column's width never changes on blink"
        );
        assert_eq!(
            s_lit.badge, s_unlit.badge,
            "and nothing downstream shifts either"
        );
    }
}
