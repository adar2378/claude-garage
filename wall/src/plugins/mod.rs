//! The plugin side of the wall (p24 design sections 5 and 7): a snapshot of
//! what the daemon says plugins offer (actions, buttons, panes) and what they
//! have pushed (widget content), turned into [`SlotItem`]s ONCE per snapshot,
//! plus the providers that hand those items to the slots.
//!
//! Trust: everything here comes from the daemon over loopback, but a plugin
//! chooses the strings. So action ids are interned only from the plugin
//! LISTING (manifest-declared ids, `ActionId::declare`); a widget's click
//! goes through `ActionId::lookup` and, when no manifest declared it, loses
//! its click and keeps its text.

pub mod build;
pub mod context;
pub mod geometry;
pub mod panes;
pub mod revs;

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use garage_protocol::plugins::{PluginList, PluginSummary, PluginWidgets, WidgetsSnapshot};
use garage_protocol::widgets::WidgetSlot;

use crate::actions::ActionId;
use crate::slots::{SlotCtx, SlotId, SlotItem, SlotProvider};

pub const GLOBAL_SCOPE: &str = "*";

/// What must be focused for a plugin action to run.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Need {
    Global,
    Workspace,
    Session,
}

/// One manifest-declared action as the wall knows it.
#[derive(Clone, Debug, PartialEq)]
pub struct PluginAction {
    pub plugin: String,
    /// The local id (`go`), what the run route takes.
    pub local: String,
    pub title: String,
    /// The strictest context the manifest lists: `Session` needs a focused
    /// session, `Workspace` a focused workspace.
    pub need: Need,
    pub confirm: bool,
    pub pane: Option<String>,
    pub default_key: Option<String>,
}

type Index = HashMap<(SlotId, String), Vec<(i32, usize, SlotItem)>>;

#[derive(Default)]
pub struct PluginStore {
    list: PluginList,
    actions: HashMap<ActionId, PluginAction>,
    /// Per plugin, the widget items already built, keyed by slot and scope.
    built: HashMap<String, Vec<(SlotId, String, i32, SlotItem)>>,
    revs: HashMap<String, u64>,
    /// Manifest buttons: shown on every scope of their slot.
    buttons: HashMap<SlotId, Vec<(i32, usize, SlotItem)>>,
    /// Widgets merged for lookup; rebuilt when content changes.
    index: Index,
    /// Bumped on every change, for tests and redraw decisions.
    generation: u64,
}

fn slot_of(slot: WidgetSlot) -> SlotId {
    match slot {
        WidgetSlot::TileTitleRight => SlotId::TileTitleRight,
        WidgetSlot::TileGutter => SlotId::TileGutter,
        WidgetSlot::RailRow => SlotId::RailRow,
        WidgetSlot::RailExtraRows => SlotId::RailExtraRows,
        WidgetSlot::RailHeader => SlotId::RailHeader,
        WidgetSlot::StripRight => SlotId::StripRight,
    }
}

impl PluginStore {
    pub fn shared() -> Rc<RefCell<PluginStore>> {
        Rc::new(RefCell::new(PluginStore::default()))
    }

    pub fn list(&self) -> &PluginList {
        &self.list
    }

    pub fn generation(&self) -> u64 {
        self.generation
    }

    /// True when any plugin is active (nothing is drawn, sent or reported
    /// otherwise).
    pub fn has_active(&self) -> bool {
        self.list.plugins.iter().any(|p| p.active)
    }

    pub fn action(&self, id: ActionId) -> Option<&PluginAction> {
        self.actions.get(&id)
    }

    pub fn actions(&self) -> impl Iterator<Item = (ActionId, &PluginAction)> {
        self.actions.iter().map(|(id, a)| (*id, a))
    }

    pub fn rev(&self, plugin: &str) -> Option<u64> {
        self.revs.get(plugin).copied()
    }

    /// Replace the installed set. Declares the action ids of the active
    /// plugins (the one place that grows the interner), rebuilds the buttons
    /// and drops widget content of plugins that are no longer active.
    pub fn apply_listing(&mut self, list: PluginList) {
        self.actions.clear();
        for p in list.plugins.iter().filter(|p| p.active) {
            for a in &p.actions {
                let Some(id) = ActionId::declare(&a.id) else { continue };
                let local = a.id.split_once(':').map_or(a.id.as_str(), |(_, l)| l).to_owned();
                let need = if a.contexts.iter().any(|c| c == "session") {
                    Need::Session
                } else if a.contexts.iter().any(|c| c == "workspace") {
                    Need::Workspace
                } else {
                    Need::Global
                };
                self.actions.insert(
                    id,
                    PluginAction {
                        plugin: p.id.clone(),
                        local,
                        title: a.title.clone(),
                        need,
                        confirm: a.confirm,
                        pane: a.pane.clone(),
                        default_key: a.default_key.clone(),
                    },
                );
            }
        }
        self.list = list;
        let active: Vec<String> = self.list.plugins.iter().filter(|p| p.active).map(|p| p.id.clone()).collect();
        self.built.retain(|id, _| active.contains(id));
        self.revs.retain(|id, _| active.contains(id));
        self.rebuild_buttons();
        self.rebuild_index();
    }

    /// Replace one plugin's widget content (or, for a full snapshot, every
    /// plugin in it). A plugin that is not active is ignored.
    pub fn apply_widgets(&mut self, snapshot: WidgetsSnapshot, full: bool) {
        if full {
            self.built.clear();
            self.revs.clear();
        }
        for (id, widgets) in snapshot.plugins {
            self.apply_plugin_widgets(&id, widgets);
        }
        self.rebuild_index();
    }

    fn apply_plugin_widgets(&mut self, id: &str, widgets: PluginWidgets) {
        if !self.list.plugins.iter().any(|p| p.id == id && p.active) {
            return;
        }
        let mut items = Vec::new();
        for (_, w) in widgets.widgets {
            let slot = slot_of(w.slot);
            for (scope, list) in w.scopes {
                for item in list {
                    for built in build::items(id, slot, &item) {
                        items.push((slot, scope.clone(), i32::from(built.priority), built));
                    }
                }
            }
        }
        self.built.insert(id.to_owned(), items);
        self.revs.insert(id.to_owned(), widgets.rev);
    }

    fn order_of(&self, plugin: &str) -> usize {
        self.list.plugins.iter().position(|p| p.id == plugin).unwrap_or(usize::MAX)
    }

    fn rebuild_buttons(&mut self) {
        let mut buttons: HashMap<SlotId, Vec<(i32, usize, SlotItem)>> = HashMap::new();
        for p in self.list.plugins.iter().filter(|p| p.active) {
            for b in &p.buttons {
                if let Some(item) = build::button(&p.id, b) {
                    buttons.entry(slot_of(b.slot)).or_default().push((b.priority, self.order_of(&p.id), item));
                }
            }
        }
        self.buttons = buttons;
    }

    fn rebuild_index(&mut self) {
        self.generation += 1;
        let mut index: Index = HashMap::new();
        for (plugin, items) in &self.built {
            let order = self.order_of(plugin);
            for (slot, scope, priority, item) in items {
                index.entry((*slot, scope.clone())).or_default().push((*priority, order, item.clone()));
            }
        }
        self.index = index;
    }

    /// The items for `slot` at `scope`, buttons included, ordered by
    /// (priority descending, install order). Widgets of one plugin keep the
    /// order the plugin pushed them in (the sort is stable).
    pub fn items_for(&self, slot: SlotId, scope: &str) -> Vec<SlotItem> {
        let widgets = self.index.get(&(slot, scope.to_owned())).map(Vec::as_slice).unwrap_or(&[]);
        let buttons = self.buttons.get(&slot).map(Vec::as_slice).unwrap_or(&[]);
        let mut all: Vec<&(i32, usize, SlotItem)> = widgets.iter().chain(buttons).collect();
        all.sort_by_key(|(priority, order, _)| (std::cmp::Reverse(*priority), *order));
        all.into_iter().map(|(_, _, item)| item.clone()).collect()
    }

    pub fn summary(&self, plugin: &str) -> Option<&PluginSummary> {
        self.list.plugins.iter().find(|p| p.id == plugin)
    }
}

/// Which scope key a slot's content is stored under, from what the renderer
/// knows (design section 5: session id, workspace name, or `*`).
fn scope_for(slot: SlotId, ctx: &SlotCtx) -> Option<String> {
    match slot {
        SlotId::StripRight => Some(GLOBAL_SCOPE.to_owned()),
        SlotId::TileTitleRight | SlotId::TileGutter | SlotId::RailRow => ctx.session.map(|s| s.id.clone()),
        SlotId::RailHeader | SlotId::RailExtraRows => ctx.group.map(|g| g.name.clone()),
        SlotId::Overlay | SlotId::ContextMenu => None,
    }
}

/// One provider per slot, registered next to the core providers. It owns no
/// content of its own: it reads the shared store, which built the items when
/// the snapshot arrived, so a frame only clones what is on screen.
pub struct PluginSlotProvider {
    slot: SlotId,
    store: Rc<RefCell<PluginStore>>,
}

impl PluginSlotProvider {
    pub fn new(slot: SlotId, store: Rc<RefCell<PluginStore>>) -> Self {
        PluginSlotProvider { slot, store }
    }
}

impl SlotProvider for PluginSlotProvider {
    fn items(&self, ctx: &SlotCtx) -> Vec<SlotItem> {
        match scope_for(self.slot, ctx) {
            Some(scope) => self.store.borrow().items_for(self.slot, &scope),
            None => Vec::new(),
        }
    }
}

/// The slots plugins can draw into.
pub const PLUGIN_SLOTS: [SlotId; 6] = [
    SlotId::TileTitleRight,
    SlotId::TileGutter,
    SlotId::RailRow,
    SlotId::RailExtraRows,
    SlotId::RailHeader,
    SlotId::StripRight,
];

/// Register a provider for every plugin slot (after the core ones).
pub fn register(registry: &mut crate::slots::SlotRegistry, store: &Rc<RefCell<PluginStore>>) {
    for slot in PLUGIN_SLOTS {
        registry.register(slot, PluginSlotProvider::new(slot, store.clone()));
    }
}
