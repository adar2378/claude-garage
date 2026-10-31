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
use std::net::{Shutdown, TcpStream};
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use portable_pty::{Child, CommandBuilder, MasterPty, PtySize, native_pty_system};

use crate::attach::Target;

fn other(e: impl std::fmt::Display) -> std::io::Error {
    std::io::Error::other(e.to_string())
}

/// Attach over a loopback socket to a session host (the Windows backend):
/// no process to signal and no tmux to detach from; closing the socket is the
/// whole teardown.
struct HostLink {
    stream: TcpStream,
    exited: Arc<AtomicBool>,
}

pub struct TileClient {
    session: String,
    /// What this client attached with; tmux calls made on its behalf
    /// (list-clients, detach) use the target's tmux flags.
    target: Target,
    parser: Arc<Mutex<vt100::Parser>>,
    /// `Some` until teardown; dropping the master closes the PTY, so it must
    /// outlive the detach. `None` for a host link.
    master: Option<Box<dyn MasterPty + Send>>,
    writer: Option<Box<dyn Write + Send>>,
    /// `None` for a host link.
    child: Option<Box<dyn Child + Send + Sync>>,
    host: Option<HostLink>,
    child_pid: Option<u32>,
    /// Last size pushed to the PTY (cols, rows) so we only TIOCSWINSZ on
    /// change.
    pty_size: (u16, u16),
    detached: bool,
}

impl TileClient {
    /// Attach to `session`. A tmux target spawns the fixed argv
    /// (`tmux [-L sock] attach -t =<session>`) on a fresh PTY; `TMUX` is
    /// stripped (nested-client refusal) and `TERM` forced to xterm-256color,
    /// per the spike. A socket target connects to the session host. `on_output`
    /// fires after each chunk is parsed — wire it to the app event channel (a
    /// "dirty" ping).
    pub fn new(
        session: &str,
        target: &Target,
        cols: u16,
        rows: u16,
        scrollback: usize,
        on_output: impl Fn() + Send + 'static,
    ) -> std::io::Result<TileClient> {
        let argv = match target {
            Target::Tmux { socket } => {
                if socket
                    .as_deref()
                    .is_some_and(|s| !garage_protocol::is_valid_tmux_socket(s))
                {
                    return Err(other("refusing an invalid tmux socket name"));
                }
                target.attach_argv(session)
            }
            Target::Socket { addr, token } => {
                return Self::new_host(
                    session, target, addr, token, cols, rows, scrollback, on_output,
                );
            }
        };
        let pty = native_pty_system();
        let pair = pty
            .openpty(PtySize {
                rows,
                cols,
                pixel_width: 0,
                pixel_height: 0,
            })
            .map_err(other)?;
        let mut cmd = CommandBuilder::new(&argv[0]);
        cmd.args(&argv[1..]);
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
            target: target.clone(),
            parser,
            master: Some(pair.master),
            writer: Some(writer),
            child: Some(child),
            host: None,
            child_pid,
            pty_size: (cols, rows),
            detached: false,
        })
    }

    /// Attach to a session host over loopback (see `garage_host::proto`).
    #[allow(clippy::too_many_arguments)]
    fn new_host(
        session: &str,
        target: &Target,
        addr: &str,
        token: &str,
        cols: u16,
        rows: u16,
        scrollback: usize,
        on_output: impl Fn() + Send + 'static,
    ) -> std::io::Result<TileClient> {
        let stream = garage_host::proto::connect_attach(addr, token, cols, rows)?;
        let mut reader = stream.try_clone()?;
        let parser = Arc::new(Mutex::new(vt100::Parser::new(rows, cols, scrollback)));
        let exited = Arc::new(AtomicBool::new(false));
        {
            let parser = parser.clone();
            let exited = exited.clone();
            std::thread::spawn(move || {
                let mut buf = [0u8; 65536];
                loop {
                    match reader.read(&mut buf) {
                        Ok(0) | Err(_) => {
                            exited.store(true, Ordering::SeqCst);
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
            target: target.clone(),
            parser,
            master: None,
            writer: None,
            child: None,
            host: Some(HostLink { stream, exited }),
            child_pid: None,
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
        if let Some(host) = self.host.as_mut() {
            let mut p = Vec::with_capacity(4);
            p.extend_from_slice(&cols.to_le_bytes());
            p.extend_from_slice(&rows.to_le_bytes());
            let _ =
                garage_host::proto::write_frame(&mut host.stream, garage_host::proto::F_RESIZE, &p);
        }
        if let Some(master) = self.master.as_ref() {
            let _ = master.resize(PtySize {
                rows,
                cols,
                pixel_width: 0,
                pixel_height: 0,
            });
        }
        self.parser
            .lock()
            .unwrap()
            .screen_mut()
            .set_size(rows, cols);
    }

    /// Non-blocking exit poll for the reattach loop (spec tui-wall "Tile PTY
    /// death recovery"): `Some(code)` once the attach client has exited
    /// (`None` code unavailable is reported as `Some(1)` — portable-pty's
    /// ExitStatus always carries one). `None` while still running.
    pub fn poll_exited(&mut self) -> Option<u32> {
        if let Some(host) = self.host.as_ref() {
            return host.exited.load(Ordering::SeqCst).then_some(0);
        }
        let Some(child) = self.child.as_mut() else {
            return Some(1);
        };
        match child.try_wait() {
            Ok(Some(status)) => Some(status.exit_code()),
            Ok(None) => None,
            Err(_) => Some(1),
        }
    }

    /// Verbatim byte passthrough into the attach client (engaged typing).
    pub fn write_bytes(&mut self, bytes: &[u8]) -> std::io::Result<()> {
        if let Some(host) = self.host.as_mut() {
            return garage_host::proto::write_frame(
                &mut host.stream,
                garage_host::proto::F_INPUT,
                bytes,
            );
        }
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
        let base = self.target.tmux_base();
        let out = Command::new(&base[0])
            .args(&base[1..])
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
        if let Some(host) = self.host.as_ref() {
            // Nothing to detach from: the host just sees the connection end.
            let _ = host.stream.shutdown(Shutdown::Both);
            return;
        }
        let base = self.target.tmux_base();
        // Per-client detach when our client is identifiable; -s (all clients
        // of the session — safe for our own spawned clients, but it would
        // also detach a co-attached user) only as the fallback.
        // `.output()` (never `.status()`): these run while the TUI owns the
        // terminal — an inherited stderr would print tmux errors ("can't
        // find session" after a kill) straight into the live frame.
        let detached = match self.own_client_tty() {
            Some(tty) => Command::new(&base[0])
                .args(&base[1..])
                .args(["detach-client", "-t", &tty])
                .output()
                .map(|o| o.status.success())
                .unwrap_or(false),
            None => false,
        };
        if !detached {
            let _ = Command::new(&base[0])
                .args(&base[1..])
                .args(["detach-client", "-s", &format!("={}", self.session)])
                .output();
        }
        // Wait for the client to exit so the master close below can no
        // longer race the detach. Bounded: never hang teardown.
        let deadline = Instant::now() + Duration::from_millis(1500);
        while Instant::now() < deadline {
            match self.child.as_mut().map(|c| c.try_wait()) {
                None | Some(Ok(Some(_))) | Some(Err(_)) => break,
                Some(Ok(None)) => std::thread::sleep(Duration::from_millis(20)),
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

#[cfg(all(test, unix))]
mod host_tests {
    //! A tile attached to a real session host (in a thread of this process)
    //! over the socket spec: no tmux involved.
    use super::*;
    use crate::ui::scroll::{capture_frozen, history_size};
    use garage_host::{proto, registry, server};

    fn eventually(what: &str, mut f: impl FnMut() -> bool) {
        let deadline = Instant::now() + Duration::from_secs(10);
        while !f() {
            assert!(Instant::now() < deadline, "timed out waiting for {what}");
            std::thread::sleep(Duration::from_millis(25));
        }
    }

    fn screen_text(c: &TileClient) -> String {
        c.parser().lock().unwrap().screen().contents()
    }

    #[test]
    fn a_socket_spec_attaches_types_scrolls_and_ends_with_the_host() {
        let hosts = tempfile::tempdir().unwrap();
        let opts = server::Options {
            id: "garage/ws/t".into(),
            dir: std::env::temp_dir().to_string_lossy().into_owned(),
            hosts_dir: hosts.path().to_path_buf(),
            cols: 60,
            rows: 6,
            command: [
                "sh",
                "-c",
                "i=0; while [ $i -lt 20 ]; do echo old-$i; i=$((i+1)); done; echo hello-tile; cat",
            ]
            .map(String::from)
            .to_vec(),
        };
        let host = std::thread::spawn(move || server::run(opts));
        let mut entry = None;
        eventually("the host record", || {
            entry = registry::read(hosts.path(), "garage/ws/t");
            entry.is_some()
        });
        let e = entry.unwrap();
        let spec = Target::Socket {
            addr: e.addr(),
            token: e.token.clone(),
        };

        let mut tile = TileClient::new("garage/ws/t", &spec, 60, 6, 0, || {}).unwrap();
        eventually("the snapshot", || screen_text(&tile).contains("hello-tile"));
        assert_eq!(tile.poll_exited(), None);

        tile.write_bytes(b"typed-in-tile\n").unwrap();
        eventually("the echo", || screen_text(&tile).contains("typed-in-tile"));

        tile.resize(70, 8);
        eventually("the host to apply the resize", || {
            capture_frozen(&spec, "garage/ws/t", 0, 70, 1).is_some()
        });

        // Frozen scrollback comes from the host's own history.
        let hist = history_size(&spec, "garage/ws/t").unwrap();
        assert!(hist >= 10, "history {hist}");
        let top = capture_frozen(&spec, "garage/ws/t", 0, 70, 3).unwrap();
        assert!(
            top.screen().contents().contains("old-0"),
            "{:?}",
            top.screen().contents()
        );

        proto::shutdown(&e.addr(), &e.token).unwrap();
        eventually("the tile to notice the host ended", || {
            tile.poll_exited().is_some()
        });
        tile.close(); // no tmux to detach from: must not hang or panic
        host.join().unwrap().unwrap();
    }

    #[test]
    fn a_dead_socket_or_an_empty_argv_is_an_error_not_a_panic() {
        let gone = Target::Socket {
            addr: "127.0.0.1:1".into(),
            token: "t".repeat(32),
        };
        assert!(TileClient::new("s", &gone, 80, 24, 0, || {}).is_err());
        let off_loopback = Target::Socket {
            addr: "203.0.113.9:80".into(),
            token: "t".repeat(32),
        };
        let e = TileClient::new("s", &off_loopback, 80, 24, 0, || {})
            .err()
            .unwrap();
        assert!(e.to_string().contains("non-loopback"), "{e}");
        let bad_socket = Target::Tmux {
            socket: Some("x -f /evil".into()),
        };
        assert!(TileClient::new("s", &bad_socket, 80, 24, 0, || {}).is_err());
    }
}
