//! TileClient: owns one tmux attach client's PTY lifecycle (task 1.2 of
//! p9-ratatui-port; the spike's `spawn_tile` grown into a type).
//!
//! Teardown discipline (spike finding 2 — "teardown injection"): killing
//! attach clients via SIGHUP (portable-pty's `ChildKiller::kill`!) or
//! letting them die on master-close EOF can inject `\n`/`^D` into the
//! ATTACHED PANE; an idle interactive shell reads `^D` as EOF and exits 0 —
//! taking its tmux session with it. Therefore:
//!  - the attach client is NEVER signalled;
//!  - `tmux detach-client` runs BEFORE the PTY master is closed, on every
//!    exit path ([`TileClient::close`] and `Drop` alike);
//!  - the detach targets only OUR client (matched by `#{client_pid}` — the
//!    spike used `detach-client -s`, which detaches every client of the
//!    session; its findings note a co-attaching user needs the per-client
//!    form, so the pid-matched `-t <client_tty>` is used here with `-s` as
//!    the fallback).
//!
//! The regression proof lives in `tests/injection.rs` (recorder-pane
//! byte-proof: an idle shell survives clean close AND SIGKILL of the
//! process, with zero injected bytes).

use std::io::{Read, Write};
use std::process::Command;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use portable_pty::{native_pty_system, Child, CommandBuilder, MasterPty, PtySize};

fn other(e: impl std::fmt::Display) -> std::io::Error {
    std::io::Error::other(e.to_string())
}

pub struct TileClient {
    session: String,
    parser: Arc<Mutex<vt100::Parser>>,
    /// `Some` until teardown; dropping the master closes the PTY, so it must
    /// outlive the detach.
    master: Option<Box<dyn MasterPty + Send>>,
    writer: Option<Box<dyn Write + Send>>,
    child: Box<dyn Child + Send + Sync>,
    child_pid: Option<u32>,
    /// Last size pushed to the PTY (cols, rows) so we only TIOCSWINSZ on
    /// change.
    pty_size: (u16, u16),
    detached: bool,
}

impl TileClient {
    /// Spawn `tmux attach -t =<session>` on a fresh PTY. `TMUX` is stripped
    /// (nested-client refusal) and `TERM` forced to xterm-256color, per the
    /// spike. `on_output` fires after each chunk is parsed — wire it to the
    /// app event channel (a "dirty" ping).
    pub fn new(
        session: &str,
        cols: u16,
        rows: u16,
        scrollback: usize,
        on_output: impl Fn() + Send + 'static,
    ) -> std::io::Result<TileClient> {
        let pty = native_pty_system();
        let pair = pty
            .openpty(PtySize {
                rows,
                cols,
                pixel_width: 0,
                pixel_height: 0,
            })
            .map_err(other)?;
        let mut cmd = CommandBuilder::new("tmux");
        cmd.args(["attach", "-t", &format!("={session}")]);
        cmd.env_remove("TMUX"); // nested-client refusal (nocterm lesson)
        cmd.env("TERM", "xterm-256color");
        let child = pair.slave.spawn_command(cmd).map_err(other)?;
        let child_pid = child.process_id();
        drop(pair.slave);

        let parser = Arc::new(Mutex::new(vt100::Parser::new(rows, cols, scrollback)));
        let mut reader = pair.master.try_clone_reader().map_err(other)?;
        let writer = pair.master.take_writer().map_err(other)?;
        {
            let parser = parser.clone();
            std::thread::spawn(move || {
                let mut buf = [0u8; 65536];
                loop {
                    match reader.read(&mut buf) {
                        Ok(0) | Err(_) => {
                            // Final ping so the state loop notices the EOF
                            // promptly (reattach loop, task 4.1) instead of
                            // waiting for its idle timeout.
                            on_output();
                            break;
                        }
                        Ok(n) => {
                            parser.lock().unwrap().process(&buf[..n]);
                            on_output();
                        }
                    }
                }
            });
        }
        Ok(TileClient {
            session: session.to_owned(),
            parser,
            master: Some(pair.master),
            writer: Some(writer),
            child,
            child_pid,
            pty_size: (cols, rows),
            detached: false,
        })
    }

    pub fn session(&self) -> &str {
        &self.session
    }

    pub fn parser(&self) -> &Arc<Mutex<vt100::Parser>> {
        &self.parser
    }

    pub fn size(&self) -> (u16, u16) {
        self.pty_size
    }

    /// Resize the PTY (TIOCSWINSZ via portable-pty `MasterPty::resize`) and
    /// the vt100 parser under the same code path — on startup and every
    /// layout change (the resize bug class).
    pub fn resize(&mut self, cols: u16, rows: u16) {
        if self.pty_size == (cols, rows) {
            return;
        }
        self.pty_size = (cols, rows);
        if let Some(master) = self.master.as_ref() {
            let _ = master.resize(PtySize {
                rows,
                cols,
                pixel_width: 0,
                pixel_height: 0,
            });
        }
        self.parser.lock().unwrap().screen_mut().set_size(rows, cols);
    }

    /// Non-blocking exit poll for the reattach loop (spec tui-wall "Tile PTY
    /// death recovery"): `Some(code)` once the attach client has exited
    /// (`None` code unavailable is reported as `Some(1)` — portable-pty's
    /// ExitStatus always carries one). `None` while still running.
    pub fn poll_exited(&mut self) -> Option<u32> {
        match self.child.try_wait() {
            Ok(Some(status)) => Some(status.exit_code()),
            Ok(None) => None,
            Err(_) => Some(1),
        }
    }

    /// Verbatim byte passthrough into the attach client (engaged typing).
    pub fn write_bytes(&mut self, bytes: &[u8]) -> std::io::Result<()> {
        let Some(writer) = self.writer.as_mut() else {
            return Ok(());
        };
        writer.write_all(bytes)?;
        writer.flush()
    }

    /// The tty of OUR attach client, found by matching `#{client_pid}`
    /// against the spawned `tmux attach` pid.
    fn own_client_tty(&self) -> Option<String> {
        let pid = self.child_pid?;
        let out = Command::new("tmux")
            .args([
                "list-clients",
                "-t",
                &format!("={}", self.session),
                "-F",
                "#{client_pid} #{client_tty}",
            ])
            .output()
            .ok()?;
        let text = String::from_utf8_lossy(&out.stdout).into_owned();
        for line in text.lines() {
            let mut parts = line.split_whitespace();
            if parts.next() == Some(pid.to_string().as_str()) {
                return parts.next().map(str::to_owned);
            }
        }
        None
    }

    /// Detach-first teardown (spike finding 2): gracefully detach our attach
    /// client via tmux, then wait (bounded) for it to exit — only then is it
    /// safe to close the PTY master. Idempotent.
    fn detach(&mut self) {
        if self.detached {
            return;
        }
        self.detached = true;
        // Per-client detach when our client is identifiable; -s (all clients
        // of the session — safe for our own spawned clients, but it would
        // also detach a co-attached user) only as the fallback.
        // `.output()` (never `.status()`): these run while the TUI owns the
        // terminal — an inherited stderr would print tmux errors ("can't
        // find session" after a kill) straight into the live frame.
        let detached = match self.own_client_tty() {
            Some(tty) => Command::new("tmux")
                .args(["detach-client", "-t", &tty])
                .output()
                .map(|o| o.status.success())
                .unwrap_or(false),
            None => false,
        };
        if !detached {
            let _ = Command::new("tmux")
                .args(["detach-client", "-s", &format!("={}", self.session)])
                .output();
        }
        // Wait for the client to exit so the master close below can no
        // longer race the detach. Bounded: never hang teardown.
        let deadline = Instant::now() + Duration::from_millis(1500);
        while Instant::now() < deadline {
            match self.child.try_wait() {
                Ok(Some(_)) | Err(_) => break,
                Ok(None) => std::thread::sleep(Duration::from_millis(20)),
            }
        }
    }

    /// Orderly teardown: detach first, then close writer and master.
    pub fn close(&mut self) {
        self.detach();
        self.writer.take();
        self.master.take();
    }
}

impl Drop for TileClient {
    /// Drop is detach-first too — an unwind (panic, early return) must never
    /// take the master down while the attach client is still connected.
    fn drop(&mut self) {
        self.close();
    }
}
