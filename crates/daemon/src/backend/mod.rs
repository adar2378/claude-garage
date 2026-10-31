//! Session backends behind one trait: tmux (macOS, Linux) and per-session
//! host processes (Windows; `GARAGE_BACKEND=host` forces it anywhere).

pub mod host;
pub mod tmux;

use std::time::Duration;

/// Upper bound for any external command (tmux, git, `claude agents --json`).
/// A hung child must never stall the poll tick or an API call.
pub const COMMAND_TIMEOUT: Duration = Duration::from_secs(5);

use std::collections::HashMap;
use std::path::Path;

use anyhow::Result;
use async_trait::async_trait;
use garage_protocol::AttachSpec;

/// One backend session (`id`, `dir`, `attached`, `created_at`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BackendSession {
    pub id: String,
    pub dir: String,
    pub attached: bool,
    /// Epoch milliseconds; `None` when the backend reports 0 or garbage.
    pub created_at: Option<i64>,
}

/// A session's active pane (`cwd`, `title`, `pid`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PaneInfo {
    pub cwd: String,
    pub title: String,
    pub pid: Option<u32>,
}

/// Where a session's cursor is in its scrollback, in absolute rows (0 is the
/// oldest retained history line): what a turn marker needs to stay on its
/// line while output scrolls past.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PaneAnchor {
    /// History lines above the visible screen.
    pub history_size: u64,
    /// The cursor's row on the visible screen (0 is the top).
    pub cursor_y: u64,
    /// The most history lines the backend keeps.
    pub history_limit: Option<u64>,
}

impl PaneAnchor {
    /// The cursor's absolute scrollback row.
    pub fn row(&self) -> u64 {
        self.history_size + self.cursor_y
    }
}

#[async_trait]
pub trait SessionBackend: Send + Sync {
    /// One-time server setup at daemon boot (tmux: extended keys). Idempotent.
    async fn init(&self) -> Result<()>;
    async fn list(&self) -> Result<Vec<BackendSession>>; // id, dir, attached, created_at
    async fn panes(&self) -> Result<HashMap<String, PaneInfo>>; // cwd, title, pid
    async fn has(&self, id: &str) -> Result<bool>;
    /// `env` is set for the session's process; backends must not bake it into
    /// backend-specific flags the caller could not reproduce elsewhere.
    async fn create(
        &self,
        id: &str,
        dir: &Path,
        cmd: &str,
        args: &[String],
        env: &[(String, String)],
    ) -> Result<()>;
    async fn respawn(
        &self,
        id: &str,
        dir: &Path,
        cmd: &str,
        args: &[String],
        env: &[(String, String)],
    ) -> Result<()>;
    async fn kill(&self, id: &str) -> Result<()>;
    async fn rename(&self, from: &str, to: &str) -> Result<()>;
    async fn capture(&self, id: &str, lines: usize) -> Result<String>;
    /// The session's cursor in absolute scrollback rows, or `None` when the
    /// backend cannot say.
    async fn pane_anchor(&self, _id: &str) -> Result<Option<PaneAnchor>> {
        Ok(None)
    }
    fn attach_spec(&self, id: &str) -> AttachSpec; // how the TUI attaches a PTY
    /// Type `text` into the session as keystrokes, then press Enter. Backends
    /// without keystroke injection keep this default.
    async fn send_text(&self, id: &str, _text: &str) -> Result<()> {
        anyhow::bail!("this session backend cannot send text to {id}")
    }
    /// Mark the just-created session `id` as a pane of `plugin`, so a later
    /// [`SessionBackend::plugin_pane_sessions`] finds it and nothing else. A
    /// backend whose sessions are all the daemon's own keeps this default.
    async fn mark_plugin_pane(&self, _id: &str, _plugin: &str) -> Result<()> {
        Ok(())
    }
    /// Ids of the plugin panes (`plugin/...`) that exist on this backend, for
    /// the daemon to adopt or clean up after a restart. They are not in
    /// [`SessionBackend::list`], which only knows `garage/` sessions. Only
    /// sessions the daemon marked as panes count: a session of the user's
    /// that happens to be named `plugin/...` is never adopted or killed.
    async fn plugin_pane_sessions(&self) -> Result<Vec<String>> {
        Ok(Vec::new())
    }
}

/// Which backend a daemon runs, chosen once at start.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Tmux,
    Host,
}

impl Kind {
    /// The OS default: tmux on macOS and Linux, host processes on Windows.
    pub fn os_default() -> Kind {
        if cfg!(windows) {
            Kind::Host
        } else {
            Kind::Tmux
        }
    }

    /// `GARAGE_BACKEND` (`tmux` or `host`; unset or empty is the OS default).
    pub fn from_env_value(value: Option<&str>) -> Result<Kind> {
        match value.map(str::trim).filter(|v| !v.is_empty()) {
            None => Ok(Kind::os_default()),
            Some("tmux") => Ok(Kind::Tmux),
            Some("host") => Ok(Kind::Host),
            Some(other) => {
                anyhow::bail!("GARAGE_BACKEND must be \"tmux\" or \"host\", got {other:?}")
            }
        }
    }
}

/// Build the backend for `kind`. The host backend starts hosts by running
/// this same executable as `claude-garage host`.
pub fn select(kind: Kind, config: &crate::Config) -> Result<std::sync::Arc<dyn SessionBackend>> {
    Ok(match kind {
        Kind::Tmux => std::sync::Arc::new(tmux::TmuxBackend::new(config.tmux_socket.clone())),
        Kind::Host => {
            let exe = std::env::current_exe()
                .map_err(|e| anyhow::anyhow!("cannot locate claude-garage: {e}"))?;
            std::sync::Arc::new(host::HostBackend::new(
                config.garage_dir.join("hosts"),
                vec![exe.to_string_lossy().into_owned(), "host".into()],
            ))
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn garage_backend_selects_the_kind_and_rejects_typos() {
        assert_eq!(Kind::from_env_value(Some("host")).unwrap(), Kind::Host);
        assert_eq!(Kind::from_env_value(Some(" tmux ")).unwrap(), Kind::Tmux);
        assert_eq!(Kind::from_env_value(None).unwrap(), Kind::os_default());
        assert_eq!(Kind::from_env_value(Some("")).unwrap(), Kind::os_default());
        let e = Kind::from_env_value(Some("screen"))
            .unwrap_err()
            .to_string();
        assert!(e.contains("GARAGE_BACKEND") && e.contains("screen"), "{e}");
    }
}
