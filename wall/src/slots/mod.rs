//! UI slots (p22 tui-actions "UI slots"): named contribution points the core
//! renderers ask for items. A provider is a pure function from a [`SlotCtx`]
//! to [`SlotItem`]s; the renderer lays the items out in registration order
//! and, when it has a width budget, drops the lowest-priority items first.
//! Only core providers are registered today; the plugin host fills the same
//! slots later.

use std::sync::OnceLock;

use ratatui::text::Span;

use crate::actions::ActionId;
use crate::state::salience::WorkspaceGroup;
use crate::state::wall_state::{WallSession, WallState};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SlotId {
    TileTitleRight,
    RailRow,
    RailHeader,
    StripRight,
    /// Defined for the plugin host; nothing renders it yet.
    Overlay,
    /// Defined for the plugin host; no context menu exists yet.
    ContextMenu,
}

impl SlotId {
    pub fn as_str(self) -> &'static str {
        match self {
            SlotId::TileTitleRight => "tile.title.right",
            SlotId::RailRow => "rail.row",
            SlotId::RailHeader => "rail.header",
            SlotId::StripRight => "strip.right",
            SlotId::Overlay => "overlay",
            SlotId::ContextMenu => "context.menu",
        }
    }
}

/// One contribution: styled text, an optional click action, and a priority
/// (higher survives longer when space runs out).
#[derive(Clone, Debug)]
pub struct SlotItem {
    pub text: Vec<Span<'static>>,
    pub on_click: Option<ActionId>,
    pub priority: i16,
}

impl SlotItem {
    pub fn new(text: Vec<Span<'static>>, priority: i16) -> Self {
        SlotItem {
            text,
            on_click: None,
            priority,
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

pub type Provider = fn(&SlotCtx) -> Vec<SlotItem>;

#[derive(Default)]
pub struct SlotRegistry {
    providers: Vec<(SlotId, Provider)>,
}

impl SlotRegistry {
    pub fn register(&mut self, slot: SlotId, provider: Provider) {
        self.providers.push((slot, provider));
    }

    /// The items every provider of `slot` contributes, in registration order.
    pub fn items(&self, slot: SlotId, ctx: &SlotCtx) -> Vec<SlotItem> {
        self.providers
            .iter()
            .filter(|(s, _)| *s == slot)
            .flat_map(|(_, provider)| provider(ctx))
            .collect()
    }

    /// The registry the wall renders through: the core providers.
    pub fn core() -> &'static SlotRegistry {
        static CORE: OnceLock<SlotRegistry> = OnceLock::new();
        CORE.get_or_init(|| {
            let mut registry = SlotRegistry::default();
            registry.register(SlotId::TileTitleRight, crate::ui::tile::context_meter_provider);
            registry.register(SlotId::StripRight, crate::ui::strip::usage_chip_provider);
            registry.register(SlotId::RailHeader, crate::ui::rail::family_label_provider);
            registry
        })
    }
}

/// Lay items out left to right in the order given. With a `max_width`, drop
/// the lowest-priority items (the later one on a tie) until the rest fit
/// whole; `None` keeps everything (the core surfaces clip or reserve space
/// themselves, and their output must not change).
pub fn layout(mut items: Vec<SlotItem>, max_width: Option<usize>) -> Vec<Span<'static>> {
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
    items.into_iter().flat_map(|item| item.text).collect()
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
        assert_eq!(joined(&out), "aabbcc");
    }

    #[test]
    fn budget_drops_lowest_priority_first_and_keeps_survivors_whole() {
        let items = || vec![item("aaa", 1), item("bbb", 9), item("ccc", 5)];
        assert_eq!(joined(&layout(items(), Some(9))), "aaabbbccc");
        assert_eq!(joined(&layout(items(), Some(8))), "bbbccc");
        assert_eq!(joined(&layout(items(), Some(3))), "bbb");
        assert_eq!(joined(&layout(items(), Some(2))), "");
    }

    #[test]
    fn a_priority_tie_drops_the_later_item() {
        assert_eq!(joined(&layout(vec![item("aa", 3), item("bb", 3)], Some(2))), "aa");
    }

    #[test]
    fn core_registry_fills_the_core_slots_and_leaves_the_plugin_slots_empty() {
        let registry = SlotRegistry::core();
        let ctx = SlotCtx::default();
        assert!(registry.items(SlotId::Overlay, &ctx).is_empty());
        assert!(registry.items(SlotId::ContextMenu, &ctx).is_empty());
        assert!(registry.items(SlotId::RailRow, &ctx).is_empty());
        let ids = [
            SlotId::TileTitleRight,
            SlotId::RailRow,
            SlotId::RailHeader,
            SlotId::StripRight,
            SlotId::Overlay,
            SlotId::ContextMenu,
        ]
        .map(SlotId::as_str);
        assert_eq!(
            ids,
            ["tile.title.right", "rail.row", "rail.header", "strip.right", "overlay", "context.menu"]
        );
    }
}
