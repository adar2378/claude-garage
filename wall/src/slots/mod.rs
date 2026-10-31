//! UI slots (p22 tui-actions "UI slots"): named contribution points the core
//! renderers ask for items. A provider maps a [`SlotCtx`] to [`SlotItem`]s
//! (a plain function, or a struct holding state of its own); the renderer lays the items out in registration order
//! and, when it has a width budget, drops the lowest-priority items first.
//! Only core providers are registered today; the plugin host fills the same
//! slots later.

use std::cell::Cell;
use std::ops::Range;
use std::sync::Arc;
use std::time::Duration;

use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::Span;
use serde_json::Value;

use crate::actions::ActionId;
use crate::state::salience::WorkspaceGroup;
use crate::state::wall_state::{WallSession, WallState};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SlotId {
    TileTitleRight,
    RailRow,
    /// Whole extra rows under a workspace's sessions (workspace-scoped).
    RailExtraRows,
    RailHeader,
    /// Row-addressed markers drawn in the tile's left gutter.
    TileGutter,
    StripRight,
    /// Defined for the plugin host; nothing renders it yet.
    Overlay,
    /// Defined for the plugin host; no context menu exists yet.
    ContextMenu,
}

impl SlotId {
    /// Every slot, in declaration order.
    pub const ALL: [SlotId; 8] = [
        SlotId::TileTitleRight,
        SlotId::RailRow,
        SlotId::RailExtraRows,
        SlotId::RailHeader,
        SlotId::TileGutter,
        SlotId::StripRight,
        SlotId::Overlay,
        SlotId::ContextMenu,
    ];

    /// The slot named `name` (the manifest/wire spelling), if any.
    pub fn parse(name: &str) -> Option<SlotId> {
        SlotId::ALL.into_iter().find(|s| s.as_str() == name)
    }

    pub fn as_str(self) -> &'static str {
        match self {
            SlotId::TileTitleRight => "tile.title.right",
            SlotId::RailRow => "rail.row",
            SlotId::RailExtraRows => "rail.extra_rows",
            SlotId::RailHeader => "rail.header",
            SlotId::TileGutter => "tile.gutter",
            SlotId::StripRight => "strip.right",
            SlotId::Overlay => "overlay",
            SlotId::ContextMenu => "context.menu",
        }
    }
}

/// What a click on a slot item runs: the action and an optional JSON payload
/// (p24 design.md Gap B). `plugin` names the contributing plugin, `None` for
/// core items; it is what makes the click a `Widget` source (a plugin's
/// item) instead of a plain `Mouse` one. Core providers pass
/// `ClickTarget::from(ids::X)`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ClickTarget {
    pub action: ActionId,
    pub args: Option<Arc<Value>>,
    pub plugin: Option<Arc<str>>,
    /// The session this item is about, for items that sit in a row which is
    /// not that session's own (a closed chat in `rail.extra_rows`). It scopes
    /// the click region, so a core action such as `session.restore` takes it
    /// as its target; `args` never does.
    pub session: Option<Arc<str>>,
}

impl ClickTarget {
    pub fn new(action: ActionId) -> Self {
        ClickTarget { action, args: None, plugin: None, session: None }
    }

    pub fn with_args(mut self, args: Value) -> Self {
        self.args = Some(Arc::new(args));
        self
    }

    pub fn about_session(mut self, session: &str) -> Self {
        self.session = Some(Arc::from(session));
        self
    }

    pub fn from_plugin(mut self, plugin: &str) -> Self {
        self.plugin = Some(Arc::from(plugin));
        self
    }
}

impl From<ActionId> for ClickTarget {
    fn from(action: ActionId) -> Self {
        ClickTarget::new(action)
    }
}

/// One clickable screen region a renderer painted: where it is, what it
/// runs, and whose it is. `session`/`workspace` are the item's own scope (the
/// tile or rail row it sits on); `None` falls back to the focus, which is how
/// a `strip.right` click on a session-scoped core action targets the focused
/// session.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SlotClickRegion {
    pub rect: Rect,
    pub target: ClickTarget,
    pub slot: SlotId,
    pub session: Option<String>,
    pub workspace: Option<String>,
}

impl SlotClickRegion {
    /// What a confirming click must match exactly: the slot, the painted
    /// rect and the item's identity (action, plugin, scope, payload). Two
    /// items at the same place with different ids or targets are different
    /// regions, and so is the same item after the layout moved it.
    pub fn identity(&self) -> RegionId {
        RegionId {
            slot: self.slot,
            rect: self.rect,
            action: self.target.action,
            plugin: self.target.plugin.clone(),
            session: self.session.clone(),
            workspace: self.workspace.clone(),
            args: self.target.args.clone(),
        }
    }
}

/// The exact region (and item) a plugin arm was raised from; see
/// [`SlotClickRegion::identity`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RegionId {
    pub slot: SlotId,
    pub rect: Rect,
    pub action: ActionId,
    pub plugin: Option<Arc<str>>,
    pub session: Option<String>,
    pub workspace: Option<String>,
    pub args: Option<Arc<Value>>,
}

/// Where a slot's renderer puts an item.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum Placement {
    /// In the slot's run of items, in registration order.
    #[default]
    Flow,
    /// `strip.right` only: floated into the strip's filler at column `x`
    /// (within the filler), with an optional caption drawn beside it where
    /// there is room. At most one such item shows.
    Filler { x: u16, caption: Option<String> },
}

/// Frame interval bounds for an animated item, in milliseconds.
pub const FRAME_MS_MIN: u16 = 100;
pub const FRAME_MS_MAX: u16 = 2000;

/// An animated item's frames and interval. The TUI picks the frame from its
/// own clock, so a plugin pushes the frame set once per state change and no
/// process ticks (p24 design.md Gap G).
#[derive(Clone, Debug, PartialEq)]
pub struct Animated {
    pub frames: Arc<[Vec<Span<'static>>]>,
    pub frame_ms: u16,
}

impl Animated {
    /// The frame shown at `now_ms`.
    pub fn frame_at(&self, now_ms: i64) -> &[Span<'static>] {
        let index = (now_ms.max(0) / i64::from(self.frame_ms)) as usize % self.frames.len();
        &self.frames[index]
    }
}

/// One contribution: styled text, an optional click action, a priority
/// (higher survives longer when space runs out) and a placement.
#[derive(Clone, Debug)]
pub struct SlotItem {
    pub text: Vec<Span<'static>>,
    pub on_click: Option<ClickTarget>,
    pub priority: i16,
    pub placement: Placement,
    /// `rail.extra_rows` only: a style laid over the whole row (a
    /// background, typically).
    pub row_style: Option<Style>,
    /// `tile.gutter` only: the absolute scrollback row (counted from the
    /// first line of the session's scrollback) this marker is drawn beside.
    pub gutter_row: Option<u64>,
    /// Set for an animated item: `text` then holds frame 0 and the registry
    /// swaps in the frame for the current draw.
    pub animated: Option<Animated>,
}

impl SlotItem {
    pub fn new(text: Vec<Span<'static>>, priority: i16) -> Self {
        SlotItem {
            text,
            on_click: None,
            priority,
            placement: Placement::Flow,
            row_style: None,
            gutter_row: None,
            animated: None,
        }
    }

    /// An animated item: `frames` cycled every `frame_ms` (clamped to
    /// 100..=2000 ms). `None` when there are no frames.
    pub fn animated(frames: Vec<Vec<Span<'static>>>, frame_ms: u16, priority: i16) -> Option<Self> {
        let first = frames.first()?.clone();
        let frame_ms = frame_ms.clamp(FRAME_MS_MIN, FRAME_MS_MAX);
        let mut item = SlotItem::new(first, priority);
        item.animated = Some(Animated { frames: frames.into(), frame_ms });
        Some(item)
    }

    pub fn width(&self) -> usize {
        self.text.iter().map(Span::width).sum()
    }
}

/// The room each slot has, as the renderers last laid it out (p24 design.md
/// section 5 "Slot geometry"). A provider reads it from
/// [`SlotCtx::geometry`] to size what it contributes; the TUI still clips and
/// drops on overlap, so it is advice, never a guarantee.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SlotGeometry {
    /// `strip.right`: the strip's width in columns.
    pub strip_width: u16,
    /// `strip.right`: the columns left free for filler items after the tabs
    /// and the flow items (`x0..x1`, strip-relative); `None` when there is
    /// no room at all.
    pub strip_filler: Option<Range<u16>>,
    /// `tile.title.right`: the most columns an item can take in the narrowest
    /// drawn tile before the title's fixed parts are squeezed.
    pub tile_title_max: u16,
    /// `tile.gutter`: the gutter is one column wide.
    pub tile_gutter_max: u16,
    /// `rail.row`, `rail.extra_rows`, `rail.header`: the rail's usable width.
    pub rail_width: u16,
}

impl SlotGeometry {
    /// The wire shape plugins see under `ui.slots` in their context JSON.
    pub fn to_json(&self) -> Value {
        let strip = match &self.strip_filler {
            Some(r) => serde_json::json!({
                "width": self.strip_width,
                "filler": { "x0": r.start, "x1": r.end },
            }),
            None => serde_json::json!({ "width": self.strip_width, "filler": null }),
        };
        serde_json::json!({
            "strip.right": strip,
            "tile.title.right": { "max_cols": self.tile_title_max },
            "tile.gutter": { "max_cols": self.tile_gutter_max },
            "rail.row": { "max_cols": self.rail_width },
            "rail.extra_rows": { "max_cols": self.rail_width },
            "rail.header": { "max_cols": self.rail_width },
        })
    }
}

/// What a provider may look at. Which fields are set depends on the slot:
/// tiles supply `session`, rail headers supply `group`; `state` is absent
/// where the caller has none (a bare tile title).
#[derive(Clone, Copy, Default)]
pub struct SlotCtx<'a> {
    pub state: Option<&'a WallState>,
    pub session: Option<&'a WallSession>,
    pub group: Option<&'a WorkspaceGroup>,
    /// The slot geometry of the last drawn frame; filled in by
    /// [`SlotRegistry::items`], so callers building a ctx leave it `None`.
    pub geometry: Option<&'a SlotGeometry>,
}

/// Something that contributes items to a slot. Plain functions and closures
/// qualify; a struct can hold state of its own (the pit pet's current frame).
pub trait SlotProvider {
    fn items(&self, ctx: &SlotCtx) -> Vec<SlotItem>;
}

impl<F: Fn(&SlotCtx) -> Vec<SlotItem>> SlotProvider for F {
    fn items(&self, ctx: &SlotCtx) -> Vec<SlotItem> {
        self(ctx)
    }
}

impl SlotProvider for Box<dyn SlotProvider> {
    fn items(&self, ctx: &SlotCtx) -> Vec<SlotItem> {
        (**self).items(ctx)
    }
}

/// The providers per slot. The app owns one and hands it to the renderers.
#[derive(Default)]
pub struct SlotRegistry {
    providers: Vec<(SlotId, Box<dyn SlotProvider>)>,
    /// The clock animated items read: set once per frame by
    /// [`SlotRegistry::begin_frame`].
    clock_ms: Cell<i64>,
    /// The shortest `frame_ms` among animated items asked for since
    /// `begin_frame` (that is, the ones drawn this frame), or 0 for none.
    animation_ms: Cell<u16>,
    /// Geometry of the last drawn frame, handed to providers via the ctx.
    geometry: std::cell::RefCell<SlotGeometry>,
}

impl SlotRegistry {
    pub fn register(&mut self, slot: SlotId, provider: impl SlotProvider + 'static) {
        self.providers.push((slot, Box::new(provider)));
    }

    /// Start a draw: set the clock animated items read and forget which
    /// animations the previous frame showed.
    pub fn begin_frame(&self, now_ms: i64) {
        self.clock_ms.set(now_ms);
        self.animation_ms.set(0);
    }

    /// How often the TUI must redraw to keep the animated items of the last
    /// frame moving; `None` when it drew none. The loop wakes for animation
    /// only on `Some`, so a wall without animated widgets pays nothing.
    pub fn animation_interval(&self) -> Option<Duration> {
        match self.animation_ms.get() {
            0 => None,
            ms => Some(Duration::from_millis(u64::from(ms))),
        }
    }

    /// Record the geometry of the frame just drawn, for providers to read on
    /// the next one.
    pub fn set_geometry(&self, geometry: SlotGeometry) {
        *self.geometry.borrow_mut() = geometry;
    }

    pub fn geometry(&self) -> SlotGeometry {
        self.geometry.borrow().clone()
    }

    /// The items every provider of `slot` contributes, in registration order.
    /// Animated items come back showing the frame for the current clock; an
    /// animated item returned here counts as visible for
    /// [`SlotRegistry::animation_interval`] (a renderer that later drops it
    /// for width costs at most one idle redraw per frame interval).
    pub fn items(&self, slot: SlotId, ctx: &SlotCtx) -> Vec<SlotItem> {
        let geometry = self.geometry.borrow();
        let ctx = SlotCtx { geometry: Some(&geometry), ..*ctx };
        let mut items: Vec<SlotItem> = self
            .providers
            .iter()
            .filter(|(s, _)| *s == slot)
            .flat_map(|(_, provider)| provider.items(&ctx))
            .collect();
        for item in &mut items {
            if let Some(anim) = &item.animated {
                item.text = anim.frame_at(self.clock_ms.get()).to_vec();
                let current = self.animation_ms.get();
                self.animation_ms
                    .set(if current == 0 { anim.frame_ms } else { current.min(anim.frame_ms) });
            }
        }
        items
    }

    /// A registry holding the core providers.
    pub fn core() -> SlotRegistry {
        let mut registry = SlotRegistry::default();
        registry.register(SlotId::TileTitleRight, crate::ui::tile::context_meter_provider);
        registry.register(SlotId::StripRight, crate::ui::strip::usage_chip_provider);
        registry.register(SlotId::RailHeader, crate::ui::rail::family_label_provider);
        registry
    }
}

/// One laid-out item's place: its columns within the laid-out run and the
/// action a click on it runs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Placed {
    pub cols: Range<usize>,
    pub on_click: Option<ClickTarget>,
}

/// The result of [`layout`]: the spans to draw and where each surviving item
/// landed, so the click router can hit-test the same columns the paint used.
#[derive(Debug, Default)]
pub struct Laid {
    pub spans: Vec<Span<'static>>,
    pub placed: Vec<Placed>,
}

impl Laid {
    /// Click ranges (`cols` shifted by `offset`) of the items that carry an
    /// action, clipped to `limit` columns.
    pub fn clicks(&self, offset: usize, limit: usize) -> Vec<(Range<u16>, ClickTarget)> {
        self.placed
            .iter()
            .filter_map(|p| {
                let target = p.on_click.clone()?;
                let start = (offset + p.cols.start).min(limit);
                let end = (offset + p.cols.end).min(limit);
                (start < end).then_some((start as u16..end as u16, target))
            })
            .collect()
    }
}

/// Lay items out left to right in the order given. With a `max_width`, drop
/// the lowest-priority items (the later one on a tie) until the rest fit
/// whole; `None` keeps everything (the core surfaces clip or reserve space
/// themselves, and their output must not change).
pub fn layout(mut items: Vec<SlotItem>, max_width: Option<usize>) -> Laid {
    if let Some(max) = max_width {
        let mut total: usize = items.iter().map(SlotItem::width).sum();
        while total > max && !items.is_empty() {
            let lowest = items
                .iter()
                .enumerate()
                .min_by_key(|(i, item)| (item.priority, std::cmp::Reverse(*i)))
                .map(|(i, _)| i)
                .expect("non-empty");
            total -= items.remove(lowest).width();
        }
    }
    let mut laid = Laid::default();
    let mut col = 0;
    for item in items {
        let width = item.width();
        laid.placed.push(Placed {
            cols: col..col + width,
            on_click: item.on_click.clone(),
        });
        col += width;
        laid.spans.extend(item.text);
    }
    laid
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(text: &str, priority: i16) -> SlotItem {
        SlotItem::new(vec![Span::raw(text.to_owned())], priority)
    }

    fn joined(spans: &[Span]) -> String {
        spans.iter().map(|s| s.content.as_ref()).collect()
    }

    #[test]
    fn no_budget_keeps_everything_in_registration_order() {
        let out = layout(vec![item("aa", 1), item("bb", 9), item("cc", 5)], None);
        assert_eq!(joined(&out.spans), "aabbcc");
    }

    #[test]
    fn budget_drops_lowest_priority_first_and_keeps_survivors_whole() {
        let items = || vec![item("aaa", 1), item("bbb", 9), item("ccc", 5)];
        assert_eq!(joined(&layout(items(), Some(9)).spans), "aaabbbccc");
        assert_eq!(joined(&layout(items(), Some(8)).spans), "bbbccc");
        assert_eq!(joined(&layout(items(), Some(3)).spans), "bbb");
        assert_eq!(joined(&layout(items(), Some(2)).spans), "");
    }

    fn clickable(text: &str, priority: i16, on_click: ActionId) -> SlotItem {
        SlotItem {
            on_click: Some(on_click.into()),
            ..item(text, priority)
        }
    }

    #[test]
    fn layout_reports_each_survivors_columns_and_click_action() {
        use crate::actions::ids;
        let laid = layout(
            vec![
                item("aa", 5),
                clickable("bbb", 9, ids::PET_PET),
                clickable("cccc", 1, ids::OVERLAY_OPEN_QUEUE),
                item("d", 5),
            ],
            Some(6),
        );
        // "cccc" (lowest priority) is dropped; the rest keep their order.
        assert_eq!(joined(&laid.spans), "aabbbd");
        let cols: Vec<_> = laid.placed.iter().map(|p| p.cols.clone()).collect();
        assert_eq!(cols, vec![0..2, 2..5, 5..6]);
        assert_eq!(laid.clicks(10, 100), vec![(12..15, ids::PET_PET.into())]);
        assert_eq!(laid.clicks(10, 13), vec![(12..13, ids::PET_PET.into())], "clipped to the limit");
        assert!(laid.clicks(10, 12).is_empty(), "fully clipped away");
    }

    fn spans(text: &str) -> Vec<Span<'static>> {
        vec![Span::raw(text.to_owned())]
    }

    #[test]
    fn animated_frames_cycle_on_the_clock_and_wrap() {
        let item = SlotItem::animated(vec![spans("a"), spans("b"), spans("c")], 250, 1).unwrap();
        let anim = item.animated.as_ref().unwrap();
        let shown = |t: i64| anim.frame_at(t)[0].content.to_string();
        assert_eq!(
            [0, 249, 250, 499, 500, 750, 999].map(shown),
            ["a", "a", "b", "b", "c", "a", "a"]
        );
        assert_eq!(shown(-5), "a", "a clock before zero does not panic");
        assert_eq!(joined(&item.text), "a", "the base text is frame 0");
    }

    #[test]
    fn the_frame_interval_is_clamped_and_an_empty_frame_list_is_not_an_item() {
        let at = |ms| SlotItem::animated(vec![spans("a")], ms, 0).unwrap().animated.unwrap().frame_ms;
        assert_eq!((at(1), at(99), at(100), at(2000), at(2001), at(u16::MAX)), (100, 100, 100, 2000, 2000, 2000));
        assert!(SlotItem::animated(Vec::new(), 250, 0).is_none());
    }

    #[test]
    fn the_registry_serves_the_frame_for_its_clock_and_reports_the_visible_interval() {
        let mut registry = SlotRegistry::default();
        registry.register(SlotId::StripRight, |_: &SlotCtx| {
            vec![
                SlotItem::animated(vec![spans("1"), spans("2")], 400, 1).unwrap(),
                SlotItem::animated(vec![spans("x"), spans("y")], 150, 1).unwrap(),
                item("static", 1),
            ]
        });
        let ctx = SlotCtx::default();
        assert_eq!(registry.animation_interval(), None, "nothing drawn yet");
        registry.begin_frame(0);
        assert_eq!(joined(&layout(registry.items(SlotId::StripRight, &ctx), None).spans), "1xstatic");
        assert_eq!(registry.animation_interval(), Some(Duration::from_millis(150)), "the fastest wins");
        registry.begin_frame(450);
        assert_eq!(joined(&layout(registry.items(SlotId::StripRight, &ctx), None).spans), "2ystatic");
        // A frame that asks for no animated slot clears the interval.
        registry.begin_frame(500);
        let _ = registry.items(SlotId::RailRow, &ctx);
        assert_eq!(registry.animation_interval(), None);
    }

    #[test]
    fn a_registry_with_only_static_items_never_reports_an_animation() {
        let registry = SlotRegistry::core();
        registry.begin_frame(123);
        for slot in SlotId::ALL {
            let _ = registry.items(slot, &SlotCtx::default());
        }
        assert_eq!(registry.animation_interval(), None);
    }

    #[test]
    fn a_click_target_carries_its_payload_and_plugin_through_layout() {
        let target = ClickTarget::new(ActionId::intern("closer:close"))
            .with_args(serde_json::json!({"keep": 2}))
            .from_plugin("closer");
        let laid = layout(
            vec![
                item("aa", 1),
                SlotItem { on_click: Some(target.clone()), ..item("bb", 1) },
            ],
            None,
        );
        let clicks = laid.clicks(5, 100);
        assert_eq!(clicks, vec![(7..9, target)]);
        assert_eq!(clicks[0].1.args.as_deref(), Some(&serde_json::json!({"keep": 2})));
        assert_eq!(clicks[0].1.plugin.as_deref(), Some("closer"));
    }

    #[test]
    fn a_boxed_provider_can_hold_state_and_a_registry_owns_its_own_providers() {
        use std::cell::Cell;
        use std::rc::Rc;
        struct Counter(Rc<Cell<u32>>);
        impl SlotProvider for Counter {
            fn items(&self, _ctx: &SlotCtx) -> Vec<SlotItem> {
                self.0.set(self.0.get() + 1);
                vec![item(&format!("n{}", self.0.get()), 1)]
            }
        }
        let calls = Rc::new(Cell::new(0));
        let boxed: Box<dyn SlotProvider> = Box::new(Counter(calls.clone()));
        let mut mine = SlotRegistry::default();
        mine.register(SlotId::StripRight, boxed);
        let ctx = SlotCtx::default();
        assert_eq!(joined(&layout(mine.items(SlotId::StripRight, &ctx), None).spans), "n1");
        assert_eq!(joined(&layout(mine.items(SlotId::StripRight, &ctx), None).spans), "n2");
        assert_eq!(calls.get(), 2);
        assert!(SlotRegistry::default().items(SlotId::StripRight, &ctx).is_empty(), "another registry is untouched");
    }

    #[test]
    fn a_priority_tie_drops_the_later_item() {
        assert_eq!(joined(&layout(vec![item("aa", 3), item("bb", 3)], Some(2)).spans), "aa");
    }

    #[test]
    fn core_registry_fills_the_core_slots_and_leaves_the_plugin_slots_empty() {
        let registry = SlotRegistry::core();
        let ctx = SlotCtx::default();
        assert!(registry.items(SlotId::Overlay, &ctx).is_empty());
        assert!(registry.items(SlotId::ContextMenu, &ctx).is_empty());
        assert!(registry.items(SlotId::RailRow, &ctx).is_empty());
        let ids = SlotId::ALL.map(SlotId::as_str);
        assert_eq!(
            ids,
            [
                "tile.title.right",
                "rail.row",
                "rail.extra_rows",
                "rail.header",
                "tile.gutter",
                "strip.right",
                "overlay",
                "context.menu"
            ]
        );
        for slot in SlotId::ALL {
            assert_eq!(SlotId::parse(slot.as_str()), Some(slot));
        }
        assert_eq!(SlotId::parse("rail.nope"), None);
        assert!(registry.items(SlotId::RailExtraRows, &ctx).is_empty());
        assert!(registry.items(SlotId::TileGutter, &ctx).is_empty());
    }
}
