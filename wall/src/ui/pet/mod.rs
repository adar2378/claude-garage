//! The pit pet behind one boundary (p22 tui-actions "Optional pit pet").
//!
//! With the default-on `pet` cargo feature this module is the sprite/sim/
//! chatter engine (`engine`) plus [`PetHost`], the one object the runtime
//! talks to. Without the feature only a no-op [`PetHost`] exists, the wall
//! builds and runs without the pet, and the strip draws none. p25 replaces
//! the host with a plugin that fills the same place.

#[cfg(not(feature = "pet"))]
mod disabled;
#[cfg(feature = "pet")]
mod engine;
#[cfg(feature = "pet")]
mod host;

#[cfg(not(feature = "pet"))]
pub use disabled::PetHost;
#[cfg(feature = "pet")]
pub use engine::*;
#[cfg(feature = "pet")]
pub use host::PetHost;

/// What a click on the pet asks the caller to do (the host cannot touch the
/// store).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PetClick {
    /// The pet is alerting: do what `a` does.
    JumpToBlocked,
    /// Petting (or a race with `P` turning the pet off): nothing more.
    Handled,
}
