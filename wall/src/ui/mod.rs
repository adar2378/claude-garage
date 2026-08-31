//! UI surfaces (wave 4 of p9-ratatui-port, mirroring `tui/lib/ui/`): pure
//! geometry (`grid_layout`, `layout`, `hit_targets`), the status vocabulary
//! (`theme`), the rendered surfaces (`tile`, `rail`, `strip`, `help`,
//! `triage`, `workspace_add`), the tile PTY registry with its reattach loop
//! (`registry`), the capture-pane frozen scrollback (`scroll`), and the
//! off-screen escalation policy (`escalation`).

pub mod escalation;
pub mod grid_layout;
pub mod help;
pub mod hit_targets;
pub mod layout;
pub mod rail;
pub mod registry;
pub mod scroll;
pub mod strip;
pub mod theme;
pub mod tile;
pub mod triage;
pub mod view_picker;
pub mod view_strip;
pub mod workspace_add;
