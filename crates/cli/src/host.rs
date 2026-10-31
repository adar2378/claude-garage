//! The real [`Host`]: ureq, libc, `std::process`. Not unit-tested (it is the
//! system); every decision it feeds lives in `launcher` / `tui_bin`.

use crate::launcher::{
    Fail, Host, Out, Probe, Reply, Starting, Terminate, checked_pid, parse_listener_pid,
};
use crate::tui_bin::{self, Builder};
use serde_json::Value;
use std::io::{BufRead, IsTerminal, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;

/// Start the child in its own session (Unix) or without a console (Windows),
/// so the daemon outlives the TUI and the terminal window.
fn detach(cmd: &mut Command) {
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
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
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const DETACHED_PROCESS: u32 = 0x0000_0008;
        const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
        cmd.creation_flags(DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP);
    }
}

/// Never flash a console window for a helper program.
#[cfg(windows)]
fn hide_console(cmd: &mut Command) {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    cmd.creation_flags(CREATE_NO_WINDOW);
}

#[cfg(unix)]
fn terminate_pid(pid: i32) -> Terminate {
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

/// `taskkill /F`: a console-less daemon ignores the polite form.
#[cfg(windows)]
fn terminate_pid(pid: i32) -> Terminate {
    let mut cmd = Command::new("taskkill");
    cmd.args(["/PID", &pid.to_string(), "/F"])
        .stdin(Stdio::null());
    hide_console(&mut cmd);
    match cmd.output() {
        Ok(o) if o.status.success() => Terminate::Sent,
        Ok(o)
            if taskkill_says_gone(
                &String::from_utf8_lossy(&o.stdout),
                &String::from_utf8_lossy(&o.stderr),
            ) =>
        {
            Terminate::Gone
        }
        Ok(o) => Terminate::Error(String::from_utf8_lossy(&o.stderr).trim().to_owned()),
        Err(e) => Terminate::Error(e.to_string()),
    }
}

/// taskkill reports a vanished process as "ERROR: The process "N" not found."
#[cfg_attr(not(windows), allow(dead_code))]
pub fn taskkill_says_gone(stdout: &str, stderr: &str) -> bool {
    format!("{stdout}{stderr}")
        .to_lowercase()
        .contains("not found")
}

/// The pid listening on `port` in `netstat -ano -p TCP` output (rows like
/// `  TCP    127.0.0.1:4747    0.0.0.0:0    LISTENING    1234`): exactly one
/// distinct pid above 1, else `None`, like the `lsof` path.
///
/// Locale-independent: the State column is translated on non-English Windows
/// ("ABHÖREN", "ÉCOUTE"), so a row counts as a listener by its shape instead:
/// the foreign address has port 0 (only a listening socket has no peer), and
/// the pid is the last column. Established rows carry a real foreign port.
#[cfg_attr(not(windows), allow(dead_code))]
pub fn parse_netstat_listener(netstat: &str, port: u16) -> Option<u32> {
    let suffix = format!(":{port}");
    let pids: Vec<String> = netstat
        .lines()
        .filter_map(|l| {
            let f: Vec<&str> = l.split_whitespace().collect();
            (f.len() == 5
                && f[0].eq_ignore_ascii_case("TCP")
                && f[1].ends_with(&suffix)
                && f[2].ends_with(":0"))
            .then(|| f[4].to_owned())
        })
        .collect();
    parse_listener_pid(&pids.join("\n"))
}

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

/// Any answer is a [`Reply`] (a non-2xx status is the caller's to read);
/// only a transport failure is an error. Requests are built with
/// `http_status_as_error(false)`.
fn reply_of(res: Result<ureq::http::Response<ureq::Body>, ureq::Error>) -> Result<Reply, String> {
    let mut r = res.map_err(|e| e.to_string())?;
    let status = r.status().as_u16();
    let body = r
        .body_mut()
        .read_to_string()
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or(Value::Object(Default::default()));
    Ok(Reply { status, body })
}

impl Out for RealHost {
    fn say(&self, line: &str) {
        println!("{line}");
    }
    fn warn(&self, line: &str) {
        eprintln!("{line}");
    }
    fn verbose(&self) -> bool {
        crate::launcher::verbose_from(std::env::var("GARAGE_VERBOSE").ok().as_deref())
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
        let mut r = ureq::get(&url(self.port, "/api/health"))
            .config()
            .timeout_global(Some(Duration::from_millis(timeout_ms)))
            .build()
            .call()
            .ok()?;
        Some(
            r.body_mut()
                .read_to_string()
                .ok()
                .and_then(|s| serde_json::from_str(&s).ok())
                .unwrap_or(Value::Object(Default::default())),
        )
    }
    fn post(&self, path: &str, body: Option<&Value>) -> Result<Reply, String> {
        let req = ureq::post(&url(self.port, path))
            .config()
            .timeout_global(Some(Duration::from_secs(300)))
            .http_status_as_error(false)
            .build();
        reply_of(match body {
            Some(b) => req
                .header("content-type", "application/json")
                .send(b.to_string()),
            None => req.send_empty(),
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
        Command::new(bin)
            .args(args)
            .status()
            .is_ok_and(|s| s.success())
    }
    fn uses_tmux(&self) -> bool {
        use garage_daemon::backend::Kind;
        Kind::from_env_value(std::env::var("GARAGE_BACKEND").ok().as_deref())
            .map_or(true, |k| k == Kind::Tmux)
    }
    #[cfg(windows)]
    fn listener_pid(&self) -> Option<u32> {
        let mut cmd = Command::new("netstat");
        cmd.args(["-ano", "-p", "TCP"]).stdin(Stdio::null());
        hide_console(&mut cmd);
        let out = cmd.output().ok()?;
        out.status
            .success()
            .then(|| parse_netstat_listener(&String::from_utf8_lossy(&out.stdout), self.port))
            .flatten()
    }
    #[cfg(not(windows))]
    fn listener_pid(&self) -> Option<u32> {
        let out = Command::new("lsof")
            .args(["-nP", &format!("-iTCP:{}", self.port), "-sTCP:LISTEN", "-t"])
            .stdin(Stdio::null())
            .output()
            .ok()?;
        out.status
            .success()
            .then(|| parse_listener_pid(&String::from_utf8_lossy(&out.stdout)))
            .flatten()
    }
    fn terminate(&self, pid: u32) -> Terminate {
        let pid = match checked_pid(pid) {
            Ok(p) => p,
            Err(e) => return Terminate::Error(e),
        };
        terminate_pid(pid)
    }
    fn spawn_daemon(&self) -> Result<Starting, String> {
        let dir = log_dir(|k| std::env::var(k).ok()).ok_or("HOME is not set; set GARAGE_DIR")?;
        std::fs::create_dir_all(&dir)
            .map_err(|e| format!("cannot create {}: {e}", dir.display()))?;
        let log_path = dir.join("daemon.log");
        let log = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&log_path)
            .map_err(|e| format!("cannot open {}: {e}", log_path.display()))?;
        let exe =
            std::env::current_exe().map_err(|e| format!("cannot locate claude-garage: {e}"))?;
        let mut cmd = Command::new(exe);
        cmd.arg("daemon")
            .stdin(Stdio::null())
            .stdout(log.try_clone().map_err(|e| e.to_string())?)
            .stderr(log);
        detach(&mut cmd);
        let mut child = cmd
            .spawn()
            .map_err(|e| format!("failed to start the daemon: {e}"))?;
        let pid = child.id();
        // Reap it if it dies while we are alive: an unreaped child stays a
        // zombie, and the wall's liveness check would pass.
        std::thread::spawn(move || {
            let _ = child.wait();
        });
        Ok(Starting {
            log: log_path,
            pid,
            port: self.port,
        })
    }
    fn sleep_ms(&self, ms: u64) {
        std::thread::sleep(Duration::from_millis(ms));
    }
    fn now_ms(&self) -> u64 {
        static START: std::sync::OnceLock<std::time::Instant> = std::sync::OnceLock::new();
        START
            .get_or_init(std::time::Instant::now)
            .elapsed()
            .as_millis() as u64
    }
    fn resolve_tui(&self) -> Result<PathBuf, Fail> {
        let exe = std::env::current_exe()
            .map_err(|e| Fail(format!("cannot locate claude-garage: {e}")))?;
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
    fn run_tui(&self, bin: &Path, starting: Option<&Starting>) -> Result<i32, String> {
        let mut cmd = Command::new(bin);
        if let Some(s) = starting {
            cmd.envs(s.env());
        }
        let status = cmd.status().map_err(|e| e.to_string())?;
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
    fn netstat_rows_give_the_single_listener_of_our_port() {
        let out = "\
  Proto  Local Address          Foreign Address        State           PID
  TCP    0.0.0.0:135            0.0.0.0:0              LISTENING       1000
  TCP    127.0.0.1:4747         0.0.0.0:0              LISTENING       4242
  TCP    [::1]:4747             [::]:0                 LISTENING       4242
  TCP    127.0.0.1:4747         127.0.0.1:50000        ESTABLISHED     5555
  TCP    127.0.0.1:47470        0.0.0.0:0              LISTENING       7777
";
        assert_eq!(parse_netstat_listener(out, 4747), Some(4242));
        assert_eq!(
            parse_netstat_listener(out, 4748),
            None,
            "nothing listens there"
        );
        let two = format!(
            "{out}  TCP    0.0.0.0:4747           0.0.0.0:0              LISTENING       9999\n"
        );
        assert_eq!(
            parse_netstat_listener(&two, 4747),
            None,
            "ambiguous: never guess"
        );
        assert_eq!(parse_netstat_listener("", 4747), None);
        // A German Windows prints the state as ABHÖREN; the pid is still found.
        let de = "  TCP    127.0.0.1:4747         0.0.0.0:0              ABHÖREN         4242\n  TCP    127.0.0.1:4747         127.0.0.1:50000        HERGESTELLT     5555\n";
        assert_eq!(parse_netstat_listener(de, 4747), Some(4242));
    }

    #[test]
    fn taskkill_not_found_means_the_process_is_already_gone() {
        assert!(taskkill_says_gone(
            "",
            "ERROR: The process \"4242\" not found."
        ));
        assert!(!taskkill_says_gone("", "ERROR: Access is denied."));
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
