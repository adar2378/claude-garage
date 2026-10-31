//! Wire content to [`SlotItem`]s: styles through the wall palette, clicks
//! through the action interner, widths measured once.

use garage_protocol::plugins::ButtonSummary;
use garage_protocol::widgets::{
    Click, Placement as WirePlacement, Segment, Style as WireStyle, WidgetItem, sanitise,
};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::Span;

use crate::actions::ActionId;
use crate::slots::{ClickTarget, Placement, SlotId, SlotItem};
use crate::ui::theme::colors;

/// What the colours a plugin may ask for look like, as RGB, for snapping a
/// `#rrggbb` to the wall's palette. Amber is deliberately absent: it is the
/// needs-input signal and nothing else may use it.
const PALETTE: [(Color, (u8, u8, u8)); 8] = [
    (Color::White, (229, 229, 229)),
    (Color::Gray, (160, 160, 160)),
    (Color::DarkGray, (96, 96, 96)),
    (Color::Green, (60, 190, 90)),
    (Color::Red, (220, 60, 60)),
    (Color::Cyan, (60, 190, 210)),
    (Color::Blue, (80, 120, 230)),
    (Color::Magenta, (200, 90, 200)),
];

fn distance(a: (u8, u8, u8), b: (u8, u8, u8)) -> i32 {
    let d = |x: u8, y: u8| (i32::from(x) - i32::from(y)).pow(2);
    d(a.0, b.0) + d(a.1, b.1) + d(a.2, b.2)
}

/// What the `needs_input` token builds to: a stand-in colour no real palette entry
/// uses. The slot provider swaps it for amber while a session needs input and
/// for plain text otherwise (see [`resolve_needs_input`]), so a plugin can
/// alert in the signal colour without owning it, and the colour is never lit
/// when nothing is waiting.
pub const NEEDS_INPUT_MARK: Color = Color::Rgb(254, 1, 77);

/// Replace the [`NEEDS_INPUT_MARK`] colour in `item`'s text (every frame of an
/// animated item too): amber when `lit`, plain text colour when not.
pub fn resolve_needs_input(item: &mut SlotItem, lit: bool) {
    let to = if lit { colors::AMBER } else { colors::FG };
    let fix = |spans: &mut [Span<'static>]| {
        for s in spans {
            if s.style.fg == Some(NEEDS_INPUT_MARK) {
                s.style.fg = Some(to);
            }
            if s.style.bg == Some(NEEDS_INPUT_MARK) {
                s.style.bg = Some(to);
            }
        }
    };
    fix(&mut item.text);
    if let Some(a) = &mut item.animated
        && a.frames
            .iter()
            .flatten()
            .any(|s| s.style.fg == Some(NEEDS_INPUT_MARK) || s.style.bg == Some(NEEDS_INPUT_MARK))
    {
        let mut frames: Vec<Vec<Span<'static>>> = a.frames.to_vec();
        frames.iter_mut().for_each(|f| fix(f));
        a.frames = frames.into();
    }
}

/// A theme token or `#rrggbb` as a wall colour.
pub fn color(spec: &str) -> Option<Color> {
    match spec {
        "text" => Some(colors::FG),
        "dim" => Some(colors::DIM),
        "accent" => Some(Color::Cyan),
        "ok" => Some(colors::GREEN),
        // Not amber, and not ANSI yellow either (terminals draw it amber):
        // amber is reserved for needs-input.
        "warn" => Some(colors::WARN),
        "error" => Some(colors::ERROR),
        "needs_input" => Some(NEEDS_INPUT_MARK),
        hex if hex.len() == 7 && hex.starts_with('#') => {
            let n = u32::from_str_radix(&hex[1..], 16).ok()?;
            let rgb = ((n >> 16) as u8, (n >> 8) as u8, n as u8);
            PALETTE
                .iter()
                .min_by_key(|(_, p)| distance(*p, rgb))
                .map(|(c, _)| *c)
        }
        _ => None,
    }
}

pub fn style(s: Option<&WireStyle>) -> Style {
    let Some(s) = s else { return Style::default() };
    let mut out = Style::default();
    if let Some(c) = s.fg.as_deref().and_then(color) {
        out = out.fg(c);
    }
    if let Some(c) = s.bg.as_deref().and_then(color) {
        out = out.bg(c);
    }
    for (on, m) in [
        (s.bold, Modifier::BOLD),
        (s.italic, Modifier::ITALIC),
        (s.underline, Modifier::UNDERLINED),
        (s.dim, Modifier::DIM),
    ] {
        if on {
            out = out.add_modifier(m);
        }
    }
    out
}

fn span(seg: &Segment) -> Span<'static> {
    Span::styled(sanitise(&seg.text), style(seg.style.as_ref()))
}

/// The click a wire action string means for `plugin`, or `None` when no
/// manifest (and no core id) declared it. Never interns anything.
pub fn resolve_click(plugin: &str, click: &Click) -> Option<ClickTarget> {
    let a = click.action.as_str();
    let id = if let Some((owner, _)) = a.split_once(':') {
        // Only this plugin's own actions: another plugin's id is refused
        // even when that plugin declared it.
        if owner != plugin {
            return None;
        }
        ActionId::lookup(a)?
    } else if garage_protocol::widgets::is_core_action_id(a) {
        ActionId::lookup(a)?
    } else {
        ActionId::lookup(&format!("{plugin}:{a}"))?
    };
    let mut target = ClickTarget::new(id).from_plugin(plugin);
    if let Some(args) = &click.args {
        target = target.with_args(args.clone());
    }
    Some(target)
}

fn priority(p: i32) -> i16 {
    p.clamp(i32::from(i16::MIN), i32::from(i16::MAX)) as i16
}

fn placement(p: &WirePlacement) -> Placement {
    match p {
        WirePlacement::Flow => Placement::Flow,
        WirePlacement::Filler { x, caption, .. } => Placement::Filler {
            x: *x,
            caption: caption.as_ref().map(|c| sanitise(c)),
        },
        // A placement a later v1 release adds: draw it in the flow.
        _ => Placement::Flow,
    }
}

/// The SlotItems one wire item becomes. Usually one; a flow item whose
/// segments carry clicks becomes one item per run of equal click, so each
/// part is its own click region.
pub fn items(plugin: &str, slot: SlotId, item: &WidgetItem) -> Vec<SlotItem> {
    let item_click = item.click.as_ref().and_then(|c| resolve_click(plugin, c));
    let session = item.session.as_deref();
    let finish = |mut it: SlotItem, click: Option<ClickTarget>| {
        it.placement = placement(&item.placement);
        it.row_style = item.row_style.as_ref().map(|s| style(Some(s)));
        it.gutter_row = item.row;
        it.on_click = click.map(|c| match session {
            Some(s) => c.about_session(s),
            None => c,
        });
        it
    };
    if let Some(frames) = &item.frames {
        let spans: Vec<Vec<Span<'static>>> = frames
            .iter()
            .map(|f| f.iter().map(span).collect())
            .collect();
        // Each frame's own click (the first on it), so a frame can do
        // something else than its neighbours.
        let per_frame: Vec<Option<ClickTarget>> = frames
            .iter()
            .map(|f| {
                f.iter()
                    .find_map(|s| s.click.as_ref())
                    .and_then(|c| resolve_click(plugin, c))
            })
            .collect();
        let frame_click = per_frame.iter().flatten().next().cloned();
        // An item-level click covers every frame; per-frame ones only apply without it.
        let per_frame = if item_click.is_some() {
            Vec::new()
        } else {
            per_frame
        };
        let ms = item.frame_ms.unwrap_or(250).min(u32::from(u16::MAX)) as u16;
        return SlotItem::animated(spans, ms, priority(item.priority))
            .map(|it| it.with_frame_x(item.frame_x.as_deref().unwrap_or(&[])))
            .map(|it| it.with_frame_clicks(per_frame))
            .map(|it| finish(it, item_click.or(frame_click)))
            .into_iter()
            .collect();
    }
    let segments = item.segments.as_deref().unwrap_or(&[]);
    let clickable_flow = slot != SlotId::TileGutter
        && matches!(item.placement, WirePlacement::Flow)
        && item.click.is_none()
        && segments.iter().any(|s| s.click.is_some());
    if !clickable_flow {
        let click = item_click.or_else(|| {
            segments
                .iter()
                .find_map(|s| s.click.as_ref())
                .and_then(|c| resolve_click(plugin, c))
        });
        let text = segments.iter().map(span).collect();
        // A gutter marker with no text is just a click target: a blank column.
        return vec![finish(SlotItem::new(text, priority(item.priority)), click)];
    }
    let mut out: Vec<SlotItem> = Vec::new();
    let mut run: Vec<&Segment> = Vec::new();
    let flush = |run: &mut Vec<&Segment>, out: &mut Vec<SlotItem>| {
        if run.is_empty() {
            return;
        }
        let click = run[0].click.as_ref().and_then(|c| resolve_click(plugin, c));
        let text = run.iter().map(|s| span(s)).collect();
        out.push(finish(SlotItem::new(text, priority(item.priority)), click));
        run.clear();
    };
    for seg in segments {
        if run.first().is_some_and(|f| f.click != seg.click) {
            flush(&mut run, &mut out);
        }
        run.push(seg);
    }
    flush(&mut run, &mut out);
    out
}

/// A manifest button as a static flow item.
pub fn button(plugin: &str, b: &ButtonSummary) -> Option<SlotItem> {
    let mut click = Click::new(b.on_click.clone());
    click.args = b.args.clone();
    let mut item = SlotItem::new(vec![Span::raw(sanitise(&b.label))], priority(b.priority));
    item.on_click = Some(resolve_click(plugin, &click)?);
    Some(item)
}

#[cfg(test)]
mod tests {
    use super::*;
    use garage_protocol::widgets::WidgetItem;
    use serde_json::json;

    fn wire(v: serde_json::Value) -> WidgetItem {
        serde_json::from_value(v).unwrap()
    }

    fn declared() {
        ActionId::declare("bt:go").unwrap();
        ActionId::declare("bt:other").unwrap();
    }

    #[test]
    fn tokens_map_to_the_palette_and_warn_is_never_amber() {
        assert_eq!(color("text"), Some(colors::FG));
        assert_eq!(color("error"), Some(colors::ERROR));
        let warn = color("warn").unwrap();
        for forbidden in [
            colors::AMBER,
            Color::Yellow,
            Color::LightYellow,
            Color::Rgb(255, 255, 0),
        ] {
            assert_ne!(warn, forbidden, "warn must not look like needs-input");
        }
        // Not a near-miss of amber either.
        let Color::Rgb(r, g, b) = warn else {
            panic!("warn is an RGB tone, not an ANSI name: {warn:?}")
        };
        let Color::Rgb(ar, ag, ab) = colors::AMBER else {
            unreachable!()
        };
        assert!(
            distance((r, g, b), (ar, ag, ab)) > 60 * 60,
            "warn is too close to amber"
        );
        assert_eq!(color("puce"), None);
        // snake_case names only: the kebab spelling of the drafts is gone
        assert_eq!(color("needs_input"), Some(NEEDS_INPUT_MARK));
        assert_eq!(color("needs-input"), None);
    }

    #[test]
    fn hex_snaps_to_the_nearest_palette_colour_and_never_to_amber() {
        for hex in ["#ffb340", "#ffa500", "#ff8800", "#ffcc66"] {
            assert_ne!(color(hex), Some(colors::AMBER), "{hex}");
        }
        assert_eq!(color("#00ff00"), Some(Color::Green));
        assert_eq!(color("#ffffff"), Some(Color::White));
        assert_eq!(color("#101010"), Some(Color::DarkGray));
        assert_eq!(color("#12345"), None);
    }

    #[test]
    fn a_click_resolves_only_to_a_declared_or_core_id_and_never_interns() {
        declared();
        let c = |a: &str| resolve_click("bt", &Click::new(a).args(json!({"k": 1})));
        let t = c("bt:go").unwrap();
        assert_eq!(
            (t.action.as_str(), t.plugin.as_deref()),
            ("bt:go", Some("bt"))
        );
        assert_eq!(t.args.as_deref(), Some(&json!({"k": 1})));
        assert_eq!(
            c("go").unwrap().action.as_str(),
            "bt:go",
            "a bare id is this plugin's"
        );
        assert_eq!(
            c("session.close").unwrap().action,
            crate::actions::ids::SESSION_CLOSE
        );
        assert!(c("bt:never-declared").is_none());
        assert!(
            ActionId::lookup("bt:never-declared").is_none(),
            "resolving did not intern it"
        );
        assert!(
            c("evil:go").is_none(),
            "another plugin's id nobody declared"
        );
        ActionId::declare("victim:poke").unwrap();
        assert!(
            c("victim:poke").is_none(),
            "another plugin's id, declared by that plugin, is still not ours to click"
        );
    }

    #[test]
    fn a_cross_plugin_click_drops_the_target_but_keeps_the_item_visible() {
        declared();
        ActionId::declare("victim:poke").unwrap();
        let it = &items(
            "bt",
            SlotId::StripRight,
            &wire(json!({"segments": [{"text": "hi", "click": {"action": "victim:poke"}}]})),
        )[0];
        assert_eq!(it.width(), 2);
        assert!(it.on_click.is_none());
        let b = ButtonSummary {
            slot: garage_protocol::widgets::WidgetSlot::StripRight,
            label: "[x]".into(),
            tooltip: None,
            on_click: "victim:poke".into(),
            args: None,
            confirm: false,
            priority: 0,
        };
        assert!(
            button("bt", &b).is_none(),
            "a button on another plugin's action is not drawn"
        );
    }

    #[test]
    fn an_undeclared_click_keeps_the_text_and_drops_the_click() {
        declared();
        let it = &items(
            "bt",
            SlotId::StripRight,
            &wire(json!({"segments": [{"text": "hi", "click": {"action": "bt:ghost"}}]})),
        )[0];
        assert_eq!(it.width(), 2);
        assert!(it.on_click.is_none());
    }

    #[test]
    fn segments_with_different_clicks_become_separate_regions_in_a_flow() {
        declared();
        let out = items(
            "bt",
            SlotId::StripRight,
            &wire(json!({"priority": 4, "segments": [
                {"text": "a ", "style": {"fg": "dim"}},
                {"text": "go", "click": {"action": "bt:go"}},
                {"text": "x", "click": {"action": "bt:go"}},
                {"text": "other", "click": {"action": "bt:other"}},
            ]})),
        );
        let shown: Vec<(String, Option<&str>)> = out
            .iter()
            .map(|i| {
                (
                    i.text.iter().map(|s| s.content.to_string()).collect(),
                    i.on_click.as_ref().map(|c| c.action.as_str()),
                )
            })
            .collect();
        assert_eq!(
            shown,
            [
                ("a ".into(), None),
                ("gox".into(), Some("bt:go")),
                ("other".into(), Some("bt:other"))
            ]
        );
        assert!(out.iter().all(|i| i.priority == 4));
    }

    #[test]
    fn extra_rows_gutter_rows_filler_and_animation_carry_over() {
        declared();
        let row = &items(
            "bt",
            SlotId::RailExtraRows,
            &wire(
                json!({"segments": [{"text": "closed one"}], "row_style": {"bg": "dim"}, "session": "garage/w/one", "click": {"action": "session.restore"}}),
            ),
        )[0];
        assert!(row.row_style.is_some());
        let t = row.on_click.as_ref().unwrap();
        assert_eq!(
            t.session.as_deref(),
            Some("garage/w/one"),
            "the row is about that session"
        );
        assert!(t.args.is_none());

        let g = &items(
            "bt",
            SlotId::TileGutter,
            &wire(json!({"row": 42, "segments": [{"text": "|", "click": {"action": "bt:go"}}]})),
        )[0];
        assert_eq!(g.gutter_row, Some(42));
        assert!(g.on_click.is_some());

        let f = &items(
            "bt",
            SlotId::StripRight,
            &wire(
                json!({"placement": {"kind": "filler", "x": 3, "caption": "hi"}, "segments": [{"text": "p"}]}),
            ),
        )[0];
        assert_eq!(
            f.placement,
            Placement::Filler {
                x: 3,
                caption: Some("hi".into())
            }
        );

        let a = &items(
            "bt",
            SlotId::StripRight,
            &wire(json!({"frames": [[{"text": "1"}], [{"text": "2"}]], "frame_ms": 50})),
        )[0];
        let anim = a.animated.as_ref().unwrap();
        assert_eq!(
            (anim.frames.len(), anim.frame_ms),
            (2, 100),
            "clamped to the 100 ms floor"
        );
    }

    #[test]
    fn text_is_cleaned_again_on_this_side() {
        let it = &items(
            "bt",
            SlotId::StripRight,
            &wire(json!({"segments": [{"text": "a\u{202e}b\nc"}]})),
        )[0];
        assert_eq!(it.text[0].content, "abc");
    }

    #[test]
    fn a_button_is_a_static_item_with_its_priority_and_click() {
        declared();
        let b = ButtonSummary {
            slot: garage_protocol::widgets::WidgetSlot::TileTitleRight,
            label: "[go]".into(),
            tooltip: None,
            on_click: "go".into(),
            args: Some(json!({"k": 2})),
            confirm: false,
            priority: 7,
        };
        let it = button("bt", &b).unwrap();
        assert_eq!((it.priority, it.width()), (7, 4));
        assert_eq!(it.on_click.unwrap().action.as_str(), "bt:go");
        let ghost = ButtonSummary {
            on_click: "ghost".into(),
            ..b
        };
        assert!(
            button("bt", &ghost).is_none(),
            "a button on an undeclared action is not drawn"
        );
    }
}
