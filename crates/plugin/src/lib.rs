//! The claude-garage plugin contract (p24): the `garage-plugin.toml` manifest
//! types, their validation, the committed JSON schema and the versioned
//! feature table. The daemon and the CLI depend on this crate; nothing here
//! runs a plugin or touches the network.

pub mod argv;
pub mod core_actions;
pub mod features;
pub mod manifest;
pub mod schema;
pub mod validate;

pub use manifest::*;
pub use validate::{CliConflict, Problem, check_cli_conflicts, check_host, parse};
