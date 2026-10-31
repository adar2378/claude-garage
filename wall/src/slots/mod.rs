//! UI slots (p22 tui-actions "UI slots"): named contribution points the core
//! renderers ask for items. A provider maps a [`SlotCtx`] to [`SlotItem`]s
//! (a plain function, or a struct holding state of its own); the renderer lays the items out in registration order
//! and, when it has a width budget, drops the lowest-priority items first.
//! Only core providers are registered today; the plugin host fills the same
//! slots later.

use std::ops::Range;
use std::sync::Arc;

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
}

impl SlotItem {
    pub fn new(text: Vec<Span<'static>>, priority: i16) -> Self {
        SlotItem {
            text,
            on_click: None,
            priority,
            placement: Placement::Flow,
            row_style: None,
        }
    }

    pub fn width(&self) -> usize {
        self.text.iter().map(Span::width).sum()
    }
}

/// What a provider may look at. Which fields are set depends on the slot:
/// tiles supply `session`, rail headers supply `group`; `state` is absent
/// where the caller has none (a bare tile title).
#[derive(Default)]
pub struct SlotCtx<'a> {
    pub state: Option<&'a WallState>,
    pub session: Option<&'a WallSession>,
    pub group: Option<&'a WorkspaceGroup>,
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
}

impl SlotRegistry {
    pub fn register(&mut self, slot: SlotId, provider: impl SlotProvider + 'static) {
        self.providers.push((slot, Box::new(provider)));
    }

    /// The items every provider of `slot` contributes, in registration order.
    pub fn items(&self, slot: SlotId, ctx: &SlotCtx) -> Vec<SlotItem> {
        self.providers
            .iter()
            .filter(|(s, _)| *s == slot)
            .flat_map(|(_, provider)| provider.items(ctx))
            .collect()
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
