//! Port of `daemon/src/tmux.js`: the `SessionBackend` implementation over tmux.
//!
//! Always passes `-L <socket>` when `GARAGE_TMUX_SOCKET` is set (see `Config`).
//! Scaffold: only `attach_spec` is real; the rest lands with the tmux port.

use std::collections::HashMap;
use std::path::Path;

use anyhow::{bail, Result};
use async_trait::async_trait;
use garage_protocol::AttachSpec;

use super::{BackendSession, PaneInfo, SessionBackend};

pub struct TmuxBackend {
    socket: Option<String>,
}

impl TmuxBackend {
    pub fn new(socket: Option<String>) -> Self {
        Self { socket }
    }

    /// `tmux` plus `-L <socket>` when an isolated server is configured.
    fn base(&self) -> Vec<String> {
        let mut v = vec!["tmux".to_string()];
        if let Some(s) = &self.socket {
            v.extend(["-L".to_string(), s.clone()]);
        }
        v
    }
}

fn todo<T>() -> Result<T> {
    bail!("tmux backend is not ported yet")
}

#[async_trait]
impl SessionBackend for TmuxBackend {
    async fn list(&self) -> Result<Vec<BackendSession>> { todo() }
    async fn panes(&self) -> Result<HashMap<String, PaneInfo>> { todo() }
    async fn has(&self, _: &str) -> Result<bool> { todo() }
    async fn create(&self, _: &str, _: &Path, _: &str, _: &[String], _: &[(String, String)]) -> Result<()> { todo() }
    async fn respawn(&self, _: &str, _: &Path, _: &str, _: &[String], _: &[(String, String)]) -> Result<()> { todo() }
    async fn kill(&self, _: &str) -> Result<()> { todo() }
    async fn rename(&self, _: &str, _: &str) -> Result<()> { todo() }
    async fn capture(&self, _: &str, _: usize) -> Result<String> { todo() }

    fn attach_spec(&self, id: &str) -> AttachSpec {
        let mut argv = self.base();
        argv.extend(["attach".into(), "-t".into(), format!("={id}")]);
        let mut detach_argv = self.base();
        detach_argv.extend(["detach-client".into(), "-t".into()]);
        AttachSpec::Tmux { argv, detach_argv }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn attach_spec_carries_the_isolated_socket() {
        let AttachSpec::Tmux { argv, detach_argv } = TmuxBackend::new(Some("t1".into())).attach_spec("garage/ws/a") else {
            panic!("tmux backend must yield a Tmux spec");
        };
        assert_eq!(argv, ["tmux", "-L", "t1", "attach", "-t", "=garage/ws/a"]);
        assert_eq!(detach_argv, ["tmux", "-L", "t1", "detach-client", "-t"]);
    }

    #[test]
    fn attach_spec_uses_default_server_without_socket() {
        let AttachSpec::Tmux { argv, .. } = TmuxBackend::new(None).attach_spec("s") else { panic!() };
        assert_eq!(argv, ["tmux", "attach", "-t", "=s"]);
    }
}
