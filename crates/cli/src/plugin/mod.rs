//! `claude-garage plugin ...` and plugin-provided subcommands (p24, design
//! section 3 and the `[[cli]]` part of section 4).
//!
//! The pieces are separated so each decision is testable without a terminal,
//! the network or a daemon:
//! - [`source`]: what `owner/repo[/subdir]` means, and the clone (a `Git` seam);
//! - [`fetch`]: release assets and checksums (a `Fetch` seam) and `[[build]]`;
//! - [`preview`]: the consent screen and the update policy;
//! - [`install`]: install, link, update and remove, as sequences over the seams;
//! - [`admin`]: registry writes, through the daemon when it is up;
//! - [`registry`]: loading what is installed (for `list` and `[[cli]]`);
//! - [`external`]: routing an unknown first word to the plugin that claims it;
//! - [`cmd`]: the clap subcommands that tie these to real stdio.

pub mod admin;
pub mod cmd;
pub mod external;
pub mod fetch;
pub mod install;
pub mod preview;
pub mod registry;
pub mod source;

#[cfg(test)]
pub mod testkit;
