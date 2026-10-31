//! The real [`Host`]: ureq, libc, `std::process`. Not unit-tested (it is the
//! system); every decision it feeds lives in `launcher` / `tui_bin`.

use crate::launcher::{parse_listener_pid, Fail, Host, Out, Probe, Reply, Terminate};
use crate::tui_bin::{self, Builder};
use serde_json::Value;
use std::io::{BufRead, IsTerminal, Write};
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;

pub struct RealHost {
    pub port: u16,
}

struct CargoBuilder;
impl Builder for CargoBuilder {
    fn have_cargo(&self) -> bool {
        Command::new("cargo")
            .arg("--version")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .is_ok_and(|s| s.success())
    }
    fn build(&self, root: &Path) -> bool {
        Command::new("cargo")
            .args(["build", "--release", "-p", "garage-wall"])
            .current_dir(root)
            .status()
            .is_ok_and(|s| s.success())
    }
}

/// `GARAGE_DIR` or `~/.garage` (where the daemon keeps its state and log).
pub fn log_dir(get: impl Fn(&str) -> Option<String>) -> Option<PathBuf> {
    match get("GARAGE_DIR").filter(|s| !s.is_empty()) {
        Some(d) => Some(PathBuf::from(d)),
        None => get("HOME").map(|h| PathBuf::from(h).join(".garage")),
    }
}

/// Package/checkout root: the nearest ancestor of the running binary with a
/// `package.json` or `wall/Cargo.toml` (target/release/x -> checkout root).
fn find_root(exe: &Path) -> PathBuf {
    exe.ancestors()
        .skip(1)
        .take(6)
        .find(|d| d.join("package.json").exists() || d.join("wall/Cargo.toml").exists())
        .map(Path::to_path_buf)
        .unwrap_or_else(|| exe.parent().unwrap_or(Path::new(".")).to_path_buf())
}

fn url(port: u16, path: &str) -> String {
    format!("http://127.0.0.1:{port}{path}")
}

fn reply_of(res: Result<ureq::Response, ureq::Error>) -> Result<Reply, String> {
    match res {
        Ok(r) | Err(ureq::Error::Status(_, r)) => {
            let status = r.status();
            let body = r.into_string().ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or(Value::Object(Default::default()));
            Ok(Reply { status, body })
        }
        Err(e) => Err(e.to_string()),
    }
}

impl Out for RealHost {
    fn say(&self, line: &str) {
        println!("{line}");
    }
    fn warn(&self, line: &str) {
        eprintln!("{line}");
    }
}

impl Host for RealHost {
    fn port(&self) -> u16 {
        self.port
    }
    fn version(&self) -> &str {
        env!("CARGO_PKG_VERSION")
    }
    fn fetch_health(&self, timeout_ms: u64) -> Option<Value> {
        let r = ureq::get(&url(self.port, "/api/health")).timeout(Duration::from_millis(timeout_ms)).call().ok()?;
        Some(r.into_string().ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or(Value::Object(Default::default())))
    }
    fn post(&self, path: &str, body: Option<&Value>) -> Result<Reply, String> {
        let req = ureq::post(&url(self.port, path)).timeout(Duration::from_secs(300));
        reply_of(match body {
            Some(b) => req.set("content-type", "application/json").send_string(&b.to_string()),
            None => req.call(),
        })
    }
    fn probe(&self, bin: &str, args: &[&str]) -> Probe {
        match Command::new(bin).args(args).stdin(Stdio::null()).output() {
            Ok(o) if o.status.success() => Probe::Ok,
            Ok(_) => Probe::OtherFail,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Probe::Missing,
            Err(_) => Probe::OtherFail,
        }
    }
    fn interactive(&self) -> bool {
        std::io::stdin().is_terminal() && std::io::stdout().is_terminal()
    }
    fn ask(&self, prompt: &str) -> String {
        print!("{prompt}");
        let _ = std::io::stdout().flush();
        let mut line = String::new();
        let _ = std::io::stdin().lock().read_line(&mut line);
        line
    }
    fn run_inherit(&self, bin: &str, args: &[&str]) -> bool {
        Command::new(bin).args(args).status().is_ok_and(|s| s.success())
    }
    fn listener_pid(&self) -> Option<u32> {
        let out = Command::new("lsof")
            .args(["-nP", &format!("-iTCP:{}", self.port), "-sTCP:LISTEN", "-t"])
            .stdin(Stdio::null())
            .output()
            .ok()?;
        out.status.success().then(|| parse_listener_pid(&String::from_utf8_lossy(&out.stdout))).flatten()
    }
    fn terminate(&self, pid: u32) -> Terminate {
        if unsafe { libc::kill(pid as libc::pid_t, libc::SIGTERM) } == 0 {
            return Terminate::Sent;
        }
        let err = std::io::Error::last_os_error();
        if err.raw_os_error() == Some(libc::ESRCH) {
            Terminate::Gone
        } else {
            Terminate::Error(err.to_string())
        }
    }
    fn spawn_daemon(&self) -> Result<PathBuf, String> {
        let dir = log_dir(|k| std::env::var(k).ok()).ok_or("HOME is not set; set GARAGE_DIR")?;
        std::fs::create_dir_all(&dir).map_err(|e| format!("cannot create {}: {e}", dir.display()))?;
        let log_path = dir.join("daemon.log");
        let log = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&log_path)
            .map_err(|e| format!("cannot open {}: {e}", log_path.display()))?;
        let exe = std::env::current_exe().map_err(|e| format!("cannot locate claude-garage: {e}"))?;
        // Own process group so it outlives the TUI and the terminal's signals.
        Command::new(exe)
            .arg("daemon")
            .stdin(Stdio::null())
            .stdout(log.try_clone().map_err(|e| e.to_string())?)
            .stderr(log)
            .process_group(0)
            .spawn()
            .map_err(|e| format!("failed to start the daemon: {e}"))?;
        Ok(log_path)
    }
    fn sleep_ms(&self, ms: u64) {
        std::thread::sleep(Duration::from_millis(ms));
    }
    fn resolve_tui(&self) -> Result<PathBuf, Fail> {
        let exe = std::env::current_exe().map_err(|e| Fail(format!("cannot locate claude-garage: {e}")))?;
        let inputs = tui_bin::Inputs {
            override_bin: std::env::var("GARAGE_TUI_BIN").ok(),
            root: find_root(&exe),
            exe_dir: exe.parent().map(Path::to_path_buf),
            target: tui_bin::node_target(),
        };
        tui_bin::resolve(&inputs, &CargoBuilder, self)
    }
    fn run_tui(&self, bin: &Path) -> Result<i32, String> {
        let status = Command::new(bin).status().map_err(|e| e.to_string())?;
        Ok(status.code().unwrap_or(1))
    }
}
