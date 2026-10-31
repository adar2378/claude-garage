//! The real [`Host`]: ureq, libc, `std::process`. Not unit-tested (it is the
//! system); every decision it feeds lives in `launcher` / `tui_bin`.

use crate::launcher::{checked_pid, parse_listener_pid, Fail, Host, Out, Probe, Reply, Terminate};
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

/// A package/checkout root: `package.json` named "claude-garage" (an
/// unrelated `~/package.json` does not count) or `wall/Cargo.toml`.
fn is_root(dir: &Path) -> bool {
    let named = std::fs::read_to_string(dir.join("package.json"))
        .ok()
        .and_then(|s| serde_json::from_str::<Value>(&s).ok())
        .is_some_and(|v| v.get("name").and_then(Value::as_str) == Some("claude-garage"));
    named || dir.join("wall/Cargo.toml").exists()
}

/// Package/checkout root: the nearest ancestor of the (canonical) binary path
/// that is a root (target/release/x -> checkout root).
fn find_root(exe: &Path) -> PathBuf {
    exe.ancestors()
        .skip(1)
        .take(6)
        .find(|d| is_root(d))
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
        let pid = match checked_pid(pid) {
            Ok(p) => p,
            Err(e) => return Terminate::Error(e),
        };
        if unsafe { libc::kill(pid, libc::SIGTERM) } == 0 {
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
        let mut cmd = Command::new(exe);
        cmd.arg("daemon")
            .stdin(Stdio::null())
            .stdout(log.try_clone().map_err(|e| e.to_string())?)
            .stderr(log);
        // SAFETY: runs between fork and exec in the child; setsid is
        // async-signal-safe and touches no Rust state. A new session (not just
        // a process group, like Node's `detached: true`) drops the controlling
        // terminal, so the daemon outlives the TUI and the terminal's SIGHUP.
        unsafe {
            cmd.pre_exec(|| {
                libc::setsid();
                Ok(())
            });
        }
        cmd.spawn()
            .map_err(|e| format!("failed to start the daemon: {e}"))?;
        Ok(log_path)
    }
    fn sleep_ms(&self, ms: u64) {
        std::thread::sleep(Duration::from_millis(ms));
    }
    fn now_ms(&self) -> u64 {
        static START: std::sync::OnceLock<std::time::Instant> = std::sync::OnceLock::new();
        START.get_or_init(std::time::Instant::now).elapsed().as_millis() as u64
    }
    fn resolve_tui(&self) -> Result<PathBuf, Fail> {
        let exe = std::env::current_exe().map_err(|e| Fail(format!("cannot locate claude-garage: {e}")))?;
        // npm/homebrew expose a bin symlink; resolve it to the real package.
        let exe = exe.canonicalize().unwrap_or(exe);
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

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("garage-host-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn find_root_skips_an_unrelated_package_json_and_finds_the_package() {
        let top = scratch("root");
        // top/package.json is unrelated; top/pkg is the real package.
        std::fs::write(top.join("package.json"), r#"{"name":"someone-elses"}"#).unwrap();
        let pkg = top.join("pkg");
        std::fs::create_dir_all(pkg.join("bin")).unwrap();
        std::fs::write(pkg.join("package.json"), r#"{"name":"claude-garage"}"#).unwrap();
        assert_eq!(find_root(&pkg.join("bin/claude-garage")), pkg);
        // nested sub-package.json with another name does not stop the walk
        std::fs::create_dir_all(pkg.join("bin/inner")).unwrap();
        std::fs::write(pkg.join("bin/package.json"), r#"{"name":"nested"}"#).unwrap();
        assert_eq!(find_root(&pkg.join("bin/inner/x")), pkg);
        std::fs::remove_dir_all(&top).unwrap();
    }

    #[test]
    fn find_root_accepts_a_checkout_by_wall_cargo_toml() {
        let top = scratch("checkout");
        std::fs::create_dir_all(top.join("wall")).unwrap();
        std::fs::write(top.join("wall/Cargo.toml"), "").unwrap();
        assert_eq!(find_root(&top.join("target/release/claude-garage")), top);
        std::fs::remove_dir_all(&top).unwrap();
    }
}
