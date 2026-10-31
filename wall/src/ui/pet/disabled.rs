//! The `pet`-feature-off host: every method is a no-op, so the runtime needs
//! no `cfg` of its own.

use crate::runtime::Notices;
use crate::slots::SlotProvider;
use crate::state::wall_state::WallState;
use crate::ui::pet::PetClick;

pub struct PetHost;

impl PetHost {
    /// No pet in this build.
    pub fn new(_started_ms: i64) -> Option<Self> {
        None
    }

    pub fn tick(
        &mut self,
        _now: i64,
        _state: &WallState,
        _notices: &mut Notices,
        _filler: u16,
    ) -> bool {
        false
    }

    pub fn click(&mut self, _now: i64, _state: &WallState, _notices: &mut Notices) -> PetClick {
        PetClick::Handled
    }

    pub fn reset_sim(&mut self) {}

    /// Nothing to put on `strip.right`: the slot stays empty.
    pub fn slot_provider(&self) -> Option<Box<dyn SlotProvider>> {
        None
    }

    pub fn publish_frame(&self, _notices: &Notices, _now: i64) {}

    /// `P` in a build without the pet: nothing to cycle.
    pub fn cycle_choice(_current: Option<&str>) -> Option<(Option<String>, String)> {
        None
    }
}
