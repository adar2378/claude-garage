//! The one action dispatcher: keys and clicks both end here (p22
//! tui-actions "Named actions"). `GarageRouter::on_garage_key` resolves a
//! key to an action and hands it over; `hit_targets::action_at` does the
//! same for a click. What the router does not own (the overlay forms and
//! queue selection, the pit pet) is handled here, keyed on the action id.

use crate::actions::{ActionCtx, ActionId, ids};

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
        if self.store.state().pet != pet_before {
            if let Some(pet) = &mut self.pet {
                pet.reset_sim();
            }
        }
        effects
    }
}
