//! garage-wall library: everything except the thin `main.rs` entrypoint.
//!
//! Module map mirrors the Dart TUI (`tui/lib/`) per the p9-ratatui-port
//! design: `api/` (daemon client + SSE), `state/` (wall state, store,
//! salience, armed actions, workspace form), `input/` (key encoder, paste),
//! `ui/` (grid math, hit targets;
//! wave-2 surfaces land here too), plus the Rust-specific `pty` (TileClient —
//! attach PTY lifecycle with detach-first teardown), `term` (raw-mode /
//! restore plumbing) and `runtime` (tokio event channel + state loop).

pub mod api;
pub mod input;
pub mod pty;
pub mod runtime;
pub mod state;
pub mod term;
pub mod ui;
