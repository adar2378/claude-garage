//! UI surfaces (wave 4 of p9-ratatui-port, mirroring `tui/lib/ui/`): pure
//! geometry (`grid_layout`, `layout`, `hit_targets`), the status vocabulary
//! (`theme`), the rendered surfaces (`tile`, `rail`, `strip`, `help`,
//! `triage`, `workspace_add`), the tile PTY registry with its reattach loop
//! (`registry`), the capture-pane frozen scrollback (`scroll`), the
//! off-screen escalation policy (`escalation`), the `t` standalone-window
//! opener (`window_open`, p12-standalone-window), and the opt-in pit pet
//! (`pet`, p15-pit-pet — sprites/mood/motion/chatter; pure, no ratatui),
//! and the p17 worktree merge/discard/keep modal (`worktree_finish`).

pub mod escalation;
pub mod grid_layout;
pub mod help;
pub mod hit_targets;
pub mod layout;
pub mod pet;
pub mod rail;
pub mod registry;
pub mod scroll;
pub mod strip;
pub mod theme;
pub mod tile;
pub mod triage;
pub mod view_picker;
pub mod view_strip;
pub mod window_open;
pub mod workspace_add;
pub mod worktree_finish;
