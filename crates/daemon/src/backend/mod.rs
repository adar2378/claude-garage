//! Session backends. tmux is the only one today; the trait is the seam a
//! Windows host-process backend plugs into later (p26).

pub mod tmux;

use std::collections::HashMap;
use std::path::Path;

use anyhow::Result;
use async_trait::async_trait;

/// One backend session (`id`, `dir`, `attached`, `created_at`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BackendSession {
    pub id: String,
    pub dir: String,
    pub attached: bool,
    /// Epoch milliseconds.
    pub created_at: i64,
}

/// A session's active pane (`cwd`, `title`, `pid`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PaneInfo {
    pub cwd: String,
    pub title: String,
    pub pid: Option<u32>,
}

#[async_trait]
pub trait SessionBackend: Send + Sync {
    async fn list(&self) -> Result<Vec<BackendSession>>; // id, dir, attached, created_at
    async fn panes(&self) -> Result<HashMap<String, PaneInfo>>; // cwd, title, pid
    async fn has(&self, id: &str) -> Result<bool>;
    async fn create(&self, id: &str, dir: &Path, cmd: &str, args: &[String]) -> Result<()>;
    async fn respawn(&self, id: &str, dir: &Path, cmd: &str, args: &[String]) -> Result<()>;
    async fn kill(&self, id: &str) -> Result<()>;
    async fn rename(&self, from: &str, to: &str) -> Result<()>;
    async fn capture(&self, id: &str, lines: usize) -> Result<String>;
    fn attach_command(&self, id: &str) -> Vec<String>; // what the TUI spawns in a PTY
}
