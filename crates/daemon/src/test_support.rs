//! Test helpers shared by every lane. Compiled for `cargo test` and, for
//! other crates' tests, with `--features test-support`.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{Duration, Instant};

use crate::backend::tmux::TmuxBackend;

static COUNTER: AtomicU32 = AtomicU32::new(0);

/// An isolated tmux server: unique `-L` socket, `-f /dev/null`, `TMUX` /
/// `TMUX_PANE` removed, `TMUX_TMPDIR` a fresh short dir under `/tmp`. The
/// server is killed and the dir removed on drop. It never touches the
/// default tmux server.
pub struct TestTmux {
    socket: String,
    root: PathBuf,
    work: PathBuf,
}

impl TestTmux {
    pub fn new() -> Self {
        let n = COUNTER.fetch_add(1, Ordering::SeqCst);
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .subsec_nanos();
        let tag = format!("gt{}-{n}-{nanos:x}", std::process::id());
        // Short on purpose: unix socket paths are limited to ~104 bytes.
        let root = PathBuf::from("/tmp").join(&tag);
        let work = root.join("w");
        std::fs::create_dir_all(&work).expect("create test tmux dir");
        Self {
            socket: tag,
            root,
            work,
        }
    }

    /// `-L` socket name.
    pub fn socket(&self) -> &str {
        &self.socket
    }

    /// `TMUX_TMPDIR` for this server (pass it to a spawned daemon's env).
    pub fn tmpdir(&self) -> &Path {
        &self.root
    }

    /// A scratch working directory (exists, owned by this helper).
    pub fn dir(&self) -> &Path {
        &self.work
    }

    /// A `TmuxBackend` bound to this server.
    pub fn backend(&self) -> TmuxBackend {
        TmuxBackend::new(Some(self.socket.clone()))
            .with_child_env("TMUX_TMPDIR", &self.root.to_string_lossy())
    }

    fn command(&self) -> Command {
        let mut c = Command::new("tmux");
        c.args(["-L", &self.socket, "-f", "/dev/null"])
            .env_remove("TMUX")
            .env_remove("TMUX_PANE")
            .env("TMUX_TMPDIR", &self.root)
            .stdin(Stdio::null());
        c
    }

    /// Run a raw tmux command on this server; returns stdout (empty on error).
    pub fn tmux(&self, args: &[&str]) -> String {
        let out = self.command().args(args).output().expect("run tmux");
        String::from_utf8_lossy(&out.stdout).into_owned()
    }

    /// Poll `f` every 25 ms for up to 5 s; panics on timeout.
    pub async fn wait_for(&self, f: impl Fn() -> bool) {
        let end = Instant::now() + Duration::from_secs(5);
        while !f() {
            assert!(Instant::now() < end, "timed out waiting for condition");
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    }

    /// Async variant of `wait_for`.
    pub async fn wait_for_async<F, Fut>(&self, f: F)
    where
        F: Fn() -> Fut,
        Fut: std::future::Future<Output = bool>,
    {
        let end = Instant::now() + Duration::from_secs(5);
        while !f().await {
            assert!(Instant::now() < end, "timed out waiting for condition");
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    }
}

impl Default for TestTmux {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for TestTmux {
    fn drop(&mut self) {
        let _ = self.command().arg("kill-server").output();
        let _ = std::fs::remove_dir_all(&self.root);
    }
}
