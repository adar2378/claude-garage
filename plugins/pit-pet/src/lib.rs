//! pit-pet: the pit pet, run as a plugin. The plugin simulates the pet and
//! pushes it as an animated, moving filler item for `strip.right`; the TUI
//! only cycles the frames. See `openspec/changes/p25-first-party-plugins/
//! design.md` ("pit-pet").

pub mod engine;
pub mod plan;
pub mod run;
