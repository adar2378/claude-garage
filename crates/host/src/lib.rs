//! The session host: the tmux replacement used on Windows (p26).
//!
//! tmux owns garage's sessions on macOS and Linux. It has no native Windows
//! build, so on Windows each session runs inside a small detached
//! `claude-garage host` process instead. A host runs the command on a
//! ConPTY/PTY (`portable-pty`), keeps a `vt100` screen with scrollback of
//! everything the command printed, listens on a loopback TCP port guarded by a
//! per-host token, and registers itself in `<garage dir>/hosts/<id>.json`.
//!
//! - [`proto`]: the wire protocol and the client calls (std only; used by the
//!   TUI and the daemon).
//! - [`registry`]: the `hosts/<id>.json` files.
//! - [`server`]: the host process itself (`claude-garage host ...`).
//!
//! Based on the design in PR #2 by Sabbir Hossain Khan.

pub mod proto;
pub mod registry;
pub mod server;
