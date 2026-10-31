//! The `pet`-feature-off host: every method is a no-op, so the runtime needs
//! no `cfg` of its own.

use crate::runtime::Notices;
use crate::state::wall_state::WallState;
use crate::ui::pet::PetClick;
use crate::ui::strip::PetRender;

pub struct PetHost;

impl PetHost {
    /// No pet in this build.
    pub fn new(_started_ms: i64) -> Option<Self> {
        None
    }

    pub fn tick(&mut self, _now: i64, _state: &WallState, _notices: &mut Notices, _filler: u16) -> bool {
        false
    }

    pub fn click(&mut self, _now: i64, _state: &WallState, _notices: &mut Notices) -> PetClick {
        PetClick::Handled
    }

    pub fn reset_sim(&mut self) {}

    pub fn strip_render(&self, _notices: &Notices, _now: i64) -> Option<PetRender> {
        None
    }

    /// `P` in a build without the pet: nothing to cycle.
    pub fn cycle_choice(_current: Option<&str>) -> Option<(Option<String>, String)> {
        None
    }
}
