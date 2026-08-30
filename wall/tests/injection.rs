//! Teardown-injection regression test (task 1.2; spike finding 2 — the
//! recorder-pane byte-proof from spikes/ratatui-wall/SPIKE.md, automated).
//!
//! The bug class: an attach client dying hard (SIGHUP from a kill, or
//! master-close EOF while still attached) can inject `\n`/`^D` into the
//! ATTACHED PANE — an idle interactive shell reads `^D` as EOF and exits,
//! taking its tmux session with it. The fix is detach-first teardown
//! (`TileClient::close` / `Drop`).
//!
//! Proof here, against a REAL tmux server with scratch `p9w1-*` sessions
//! only (never the daemon, never ~/.garage, never user sessions):
//!  1. an idle default-shell session AND a raw-mode `cat -v` recorder pane
//!     survive a clean close with byte-identical pane content;
//!  2. both survive SIGKILL of the process holding the attach PTYs, with
//!     byte-identical pane content (zero injected bytes visible in the
//!     recorder).

use std::io::BufRead;
use std::process::{Command, Stdio};
use std::time::Duration;

fn tmux(args: &[&str]) -> std::process::Output {
    Command::new("tmux")
        .args(args)
        .output()
        .expect("failed to run tmux")
}

fn tmux_available() -> bool {
    Command::new("tmux")
        .arg("-V")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

fn has_session(name: &str) -> bool {
    tmux(&["has-session", "-t", &format!("={name}")])
        .status
        .success()
}

/// Pane content, normalized (trailing per-line whitespace and trailing blank
/// lines stripped) so cursor-position noise can't cause false diffs.
fn capture(name: &str) -> String {
    let out = tmux(&["capture-pane", "-p", "-t", &format!("={name}:")]);
    let text = String::from_utf8_lossy(&out.stdout);
    let mut lines: Vec<&str> = text.lines().map(str::trim_end).collect();
    while lines.last() == Some(&"") {
        lines.pop();
    }
    lines.join("\n")
}

/// Kills the scratch sessions on drop, pass or fail.
struct Scratch(Vec<String>);

impl Drop for Scratch {
    fn drop(&mut self) {
        for name in &self.0 {
            let _ = tmux(&["kill-session", "-t", &format!("={name}")]);
        }
    }
}

#[test]
fn teardown_never_injects_bytes_into_attached_panes() {
    if !tmux_available() {
        eprintln!("SKIP: tmux not available — injection regression not exercised");
        return;
    }

    let suffix = std::process::id();
    let shell = format!("p9w1-shell-{suffix}");
    let rec = format!("p9w1-rec-{suffix}");
    let _scratch = Scratch(vec![shell.clone(), rec.clone()]);

    // Idle interactive shell — the victim of the ^D/EOF injection class.
    assert!(
        tmux(&["new-session", "-d", "-s", &shell, "-x", "80", "-y", "24"])
            .status
            .success(),
        "failed to create scratch shell session"
    );
    // Recorder pane: raw mode + no echo, `cat -v` prints every byte the pane
    // receives (^D shows as ^D instead of acting as EOF) — the spike's
    // byte-level proof.
    assert!(
        tmux(&[
            "new-session",
            "-d",
            "-s",
            &rec,
            "-x",
            "80",
            "-y",
            "24",
            "stty raw -echo; cat -v",
        ])
        .status
        .success(),
        "failed to create scratch recorder session"
    );
    std::thread::sleep(Duration::from_millis(800)); // prompt/raw-mode settle

    let shell_before = capture(&shell);
    let rec_before = capture(&rec);

    let holder = env!("CARGO_BIN_EXE_pty_hold");

    // ── Scenario 1: clean close (detach-first) ──────────────────────────
    let out = Command::new(holder)
        .args(["close", &shell, &rec])
        .output()
        .expect("failed to run pty_hold");
    assert!(
        out.status.success(),
        "pty_hold close failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    std::thread::sleep(Duration::from_millis(500));

    assert!(has_session(&shell), "idle shell session died on clean close");
    assert!(has_session(&rec), "recorder session died on clean close");
    assert_eq!(
        capture(&shell),
        shell_before,
        "clean close injected bytes into the idle shell pane"
    );
    assert_eq!(
        capture(&rec),
        rec_before,
        "clean close injected bytes into the recorder pane"
    );

    // ── Scenario 2: SIGKILL of the process holding the attach PTYs ──────
    let mut child = Command::new(holder)
        .args(["hold", &shell, &rec])
        .stdout(Stdio::piped())
        .spawn()
        .expect("failed to spawn pty_hold hold");
    {
        // Wait for the "ATTACHED" handshake so the kill lands while the
        // attach clients are live.
        let stdout = child.stdout.take().expect("pty_hold stdout");
        let mut line = String::new();
        std::io::BufReader::new(stdout)
            .read_line(&mut line)
            .expect("read pty_hold handshake");
        assert_eq!(line.trim(), "ATTACHED");
    }
    child.kill().expect("SIGKILL pty_hold"); // std kill = SIGKILL on unix
    let _ = child.wait();
    std::thread::sleep(Duration::from_millis(1000)); // let clients die/EOF settle

    assert!(
        has_session(&shell),
        "idle shell session died when the wall process was SIGKILLed"
    );
    assert!(
        has_session(&rec),
        "recorder session died when the wall process was SIGKILLed"
    );
    assert_eq!(
        capture(&shell),
        shell_before,
        "SIGKILL teardown injected bytes into the idle shell pane"
    );
    assert_eq!(
        capture(&rec),
        rec_before,
        "SIGKILL teardown injected bytes into the recorder pane"
    );
}
