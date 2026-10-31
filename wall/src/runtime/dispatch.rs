//! The one action dispatcher: keys and clicks both end here (p22
//! tui-actions "Named actions"). `GarageRouter::on_garage_key` resolves a
//! key to an action and hands it over; `hit_targets::action_at` does the
//! same for a click. What the router does not own (the overlay forms and
//! queue selection, the pit pet) is handled here, keyed on the action id.

use crate::actions::{ActionCtx, ActionId, ActionSource, ids};
use crate::state::wall_state::KeyLayer;

use super::{App, Effect, OverlayRefs, now_ms, overlay_action};

impl App {
    /// Run one action with its context, whatever raised it. Returns the
    /// effects the caller performs.
    pub(super) fn dispatch_action(&mut self, id: ActionId, ctx: &ActionCtx) -> Vec<Effect> {
        self.dispatch_action_at(id, ctx, now_ms())
    }

    /// [`Self::dispatch_action`] with an explicit clock, for the armed-press
    /// windows tests drive in virtual time.
    pub(super) fn dispatch_action_at(
        &mut self,
        id: ActionId,
        ctx: &ActionCtx,
        now: i64,
    ) -> Vec<Effect> {
        // Outcome check: whatever path a plugin's own request takes
        // (`jump.blocked`, a queue jump, ...), it never leaves the human
        // typing into a session. The new focus stays; the layer goes back.
        let plugin = match &ctx.source {
            ActionSource::Plugin { plugin, .. } => Some(plugin.clone()),
            _ => None,
        };
        let was_engaged = self.store.state().layer == KeyLayer::Engaged;
        let effects = self.dispatch_action_inner(id, ctx, now);
        if let Some(plugin) = plugin
            && !was_engaged && self.store.state().layer == KeyLayer::Engaged {
                self.store.disengage();
                self.router.notices.show(
                    format!("plugin {plugin} tried to put you into a session — press Enter to engage"),
                    4000,
                    now,
                );
            }
        effects
    }

    fn dispatch_action_inner(&mut self, id: ActionId, ctx: &ActionCtx, now: i64) -> Vec<Effect> {
        // The confirm rule for plugin-originated triggers: they may only arm
        // a dangerous action; a human key or click confirms (and a refused
        // id never runs). Everything it handles ends here.
        let mut gated = Vec::new();
        if self.router.gate(&mut self.store, id, ctx, now, &mut gated) {
            return gated;
        }
        // Fresh overlay state on every open (the Dart `A`/`w`/`D`
        // branches) — keyed on the action, so a rebound key or a click
        // resets the same way, and it happens even if the dispatch declines.
        match id {
            ids::OVERLAY_OPEN_QUEUE => self.queue_selection = 0,
            ids::OVERLAY_OPEN_WORKSPACE_ADD => self.ws_form.reset(),
            ids::OVERLAY_OPEN_VIEW_PICKER => self.view_picker.reset(),
            _ => {}
        }
        if id == ids::PET_PET {
            self.handle_pet_click(now);
            return Vec::new();
        }
        let ui = OverlayRefs {
            queue_selection: &mut self.queue_selection,
            form: &mut self.ws_form,
            view_picker: &mut self.view_picker,
        };
        let (handled, notice) = overlay_action(&mut self.store, id, ctx, ui);
        if handled {
            if let Some(text) = notice {
                self.router.notices.show(text, 2000, now);
            }
            return Vec::new();
        }
        // `P` (spec tui-pit-pet): a fresh pet starts its motion sim from
        // scratch and renders on the very next tick, rather than carrying
        // over the outgoing (or newly-arriving) species' position/frame
        // counters. `GarageRouter` owns the choice + persistence + notice;
        // it doesn't own the sim, so the reset happens here by comparing
        // before/after.
        let pet_before = self.store.state().pet.clone();
        let mut effects = Vec::new();
        self.router
            .run_action(&mut self.store, id, ctx, now, &mut effects);
        if self.store.state().pet != pet_before
            && let Some(pet) = &mut self.pet {
                pet.reset_sim();
            }
        effects
    }
}
