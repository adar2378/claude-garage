//! Test harness: runs a daemon in complete isolation and talks to it over HTTP.
//!
//! Isolation (nothing here may reach the developer's tmux server, daemon, or
//! home directory):
//! - a free port (never 4747), a private GARAGE_DIR / GARAGE_CLAUDE_HOME / HOME
//! - a private tmux socket (`GARAGE_TMUX_SOCKET`) inside a private TMUX_TMPDIR,
//!   with `TMUX`/`TMUX_PANE` removed from the daemon's environment
//! - stub `claude`, `osascript` and `terminal-notifier` first on PATH, so the
//!   daemon never runs the real Claude CLI and never raises a notification
//! - `GARAGE_CLAUDE_CMD=sh`, so "claude" sessions are plain shells

#![allow(dead_code)]

use serde_json::Value;
use std::fs;
use std::io::{BufRead, BufReader};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::mpsc::{self, Receiver};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

static COUNTER: AtomicU32 = AtomicU32::new(0);

pub struct Daemon {
    pub port: u16,
    pub base: String,
    /// Private root: contains garage/, claude/, home/, tmux/, bin/, daemon.log
    pub root: PathBuf,
    pub garage_dir: PathBuf,
    pub socket: String,
    child: Child,
    agent: ureq::Agent,
}

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("repo root")
}

/// Short on purpose: unix socket paths are limited to ~104 bytes on macOS.
fn scratch_root() -> PathBuf {
    let base = if Path::new("/tmp").is_dir() {
        PathBuf::from("/tmp")
    } else {
        std::env::temp_dir()
    };
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .subsec_nanos();
    let n = COUNTER.fetch_add(1, Ordering::SeqCst);
    let dir = base.join(format!("gc-{}-{}-{:x}", std::process::id(), n, nanos));
    fs::create_dir_all(&dir).unwrap();
    dir
}

fn free_port() -> u16 {
    let l = TcpListener::bind("127.0.0.1:0").unwrap();
    l.local_addr().unwrap().port()
}

fn write_stub(dir: &Path, name: &str, body: &str) {
    let p = dir.join(name);
    fs::write(&p, format!("#!/bin/sh\n{body}\n")).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&p, fs::Permissions::from_mode(0o755)).unwrap();
    }
}

impl Daemon {
    pub fn start() -> Daemon {
        let root = scratch_root();
        let garage_dir = root.join("garage");
        let claude_home = root.join("claude");
        let home = root.join("home");
        let tmux_tmp = root.join("tmux");
        let bin = root.join("bin");
        for d in [&garage_dir, &claude_home, &home, &tmux_tmp, &bin] {
            fs::create_dir_all(d).unwrap();
        }
        write_stub(&bin, "claude", "echo '[]'");
        write_stub(&bin, "osascript", "exit 0");
        write_stub(&bin, "terminal-notifier", "exit 0");

        let port = free_port();
        let socket = format!("gc-{}", root.file_name().unwrap().to_string_lossy());
        let cmd = std::env::var("GARAGE_DAEMON_CMD")
            .unwrap_or_else(|_| "node daemon/src/index.js".to_string());
        let path = format!(
            "{}:{}",
            bin.display(),
            std::env::var("PATH").unwrap_or_default()
        );
        let log = fs::File::create(root.join("daemon.log")).unwrap();

        let child = Command::new("sh")
            .arg("-c")
            .arg(format!("exec {cmd}"))
            .current_dir(repo_root())
            .env_remove("TMUX")
            .env_remove("TMUX_PANE")
            .env("PATH", path)
            .env("HOME", &home)
            .env("GARAGE_PORT", port.to_string())
            .env("GARAGE_DIR", &garage_dir)
            .env("GARAGE_CLAUDE_HOME", &claude_home)
            .env("GARAGE_TMUX_SOCKET", &socket)
            .env("TMUX_TMPDIR", &tmux_tmp)
            .env("GARAGE_CLAUDE_CMD", "sh")
            .stdin(Stdio::null())
            .stdout(log.try_clone().unwrap())
            .stderr(log)
            .spawn()
            .expect("start daemon (set GARAGE_DAEMON_CMD?)");

        let agent: ureq::Agent = ureq::Agent::config_builder()
            .timeout_global(Some(Duration::from_secs(30)))
            // every status is an answer the tests assert on
            .http_status_as_error(false)
            .build()
            .into();
        let mut d = Daemon {
            port,
            base: format!("http://127.0.0.1:{port}"),
            root,
            garage_dir,
            socket,
            child,
            agent,
        };
        d.wait_healthy();
        d
    }

    fn wait_healthy(&mut self) {
        let deadline = Instant::now() + Duration::from_secs(20);
        loop {
            if let Ok(Some(status)) = self.child.try_wait() {
                panic!("daemon exited early ({status}); log:\n{}", self.log());
            }
            if let Ok(r) = self.agent.get(&format!("{}/api/health", self.base)).call() {
                if r.status().as_u16() == 200 {
                    return;
                }
            }
            if Instant::now() > deadline {
                panic!("daemon not healthy in 20s; log:\n{}", self.log());
            }
            thread::sleep(Duration::from_millis(100));
        }
    }

    pub fn log(&self) -> String {
        fs::read_to_string(self.root.join("daemon.log")).unwrap_or_default()
    }

    /// Send a request; returns (status, parsed JSON body or Null for empty / non-JSON).
    pub fn request(&self, method: &str, path: &str, body: Option<Value>) -> (u16, Value) {
        let mut builder = ureq::http::Request::builder()
            .method(method)
            .uri(format!("{}{}", self.base, path));
        if body.is_some() {
            builder = builder.header("Content-Type", "application/json");
        }
        let req = builder
            .body(body.map(|b| b.to_string()).unwrap_or_default())
            .unwrap();
        let mut resp = match self.agent.run(req) {
            Ok(r) => r,
            Err(e) => panic!("{method} {path}: transport error: {e}"),
        };
        let status = resp.status().as_u16();
        let text = resp.body_mut().read_to_string().unwrap_or_default();
        (status, serde_json::from_str(&text).unwrap_or(Value::Null))
    }

    pub fn get(&self, path: &str) -> (u16, Value) {
        self.request("GET", path, None)
    }
    pub fn post(&self, path: &str, body: Value) -> (u16, Value) {
        self.request("POST", path, Some(body))
    }
    pub fn put(&self, path: &str, body: Value) -> (u16, Value) {
        self.request("PUT", path, Some(body))
    }
    pub fn patch(&self, path: &str, body: Value) -> (u16, Value) {
        self.request("PATCH", path, Some(body))
    }
    pub fn delete(&self, path: &str) -> (u16, Value) {
        self.request("DELETE", path, None)
    }

    /// Make a directory to register as a workspace. Canonicalised, because
    /// tmux reports resolved paths (/tmp is a symlink on macOS).
    pub fn mkdir(&self, name: &str) -> PathBuf {
        let p = self.root.join(name);
        fs::create_dir_all(&p).unwrap();
        p.canonicalize().unwrap()
    }

    /// A git repo with one commit, for worktree tests.
    pub fn git_repo(&self, name: &str) -> PathBuf {
        let p = self.mkdir(name);
        let git = |args: &[&str]| {
            let st = Command::new("git")
                .args(["-c", "user.email=t@t", "-c", "user.name=t"])
                .args(args)
                .current_dir(&p)
                .env("HOME", self.root.join("home"))
                .env("GIT_CONFIG_NOSYSTEM", "1")
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status()
                .unwrap();
            assert!(st.success(), "git {args:?} failed");
        };
        git(&["init", "-q"]);
        git(&["commit", "-q", "--allow-empty", "-m", "init"]);
        p
    }

    pub fn add_workspace(&self, name: &str, dir: &Path) {
        let (s, _) = self.put(
            "/api/workspaces",
            serde_json::json!({"name": name, "dir": dir}),
        );
        assert_eq!(s, 200, "register workspace {name}");
    }

    /// Create a session; returns the response body.
    pub fn create_session(&self, ws: &str, label: &str, worktree: bool) -> Value {
        let (s, b) = self.post(
            "/api/sessions",
            serde_json::json!({"workspace": ws, "label": label, "worktree": worktree}),
        );
        assert_eq!(s, 201, "create session: {b}");
        b
    }

    pub fn sessions(&self) -> Vec<Value> {
        let (s, b) = self.get("/api/sessions");
        assert_eq!(s, 200);
        b.as_array().expect("sessions array").clone()
    }

    pub fn session(&self, id: &str) -> Option<Value> {
        self.sessions().into_iter().find(|s| s["id"] == id)
    }

    /// The per-install token, read from the hook snippet's URL.
    pub fn hook_token(&self) -> String {
        let (s, b) = self.get("/api/hooks/snippet");
        assert_eq!(s, 200);
        let url = b["hooks"]["Stop"][0]["hooks"][0]["url"].as_str().unwrap();
        url.split("token=").nth(1).unwrap().to_string()
    }

    /// Poll until `f` returns Some, up to `secs`.
    pub fn wait_for<T>(&self, secs: u64, mut f: impl FnMut() -> Option<T>) -> Option<T> {
        let deadline = Instant::now() + Duration::from_secs(secs);
        loop {
            if let Some(v) = f() {
                return Some(v);
            }
            if Instant::now() > deadline {
                return None;
            }
            thread::sleep(Duration::from_millis(150));
        }
    }

    /// Run tmux against this daemon's private server only.
    pub fn tmux(&self, args: &[&str]) -> bool {
        self.tmux_cmd()
            .args(args)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
    }

    fn tmux_cmd(&self) -> Command {
        let mut c = Command::new("tmux");
        c.env_remove("TMUX")
            .env_remove("TMUX_PANE")
            .env("TMUX_TMPDIR", self.root.join("tmux"))
            .args(["-L", &self.socket]);
        c
    }

    /// Open /api/events; every line the stream emits arrives on the receiver.
    pub fn events(&self) -> Receiver<String> {
        let (tx, rx) = mpsc::channel();
        let url = format!("{}/api/events", self.base);
        // no overall timeout: the stream is meant to stay open
        let agent: ureq::Agent = ureq::Agent::config_builder().build().into();
        let (ready_tx, ready_rx) = mpsc::channel();
        thread::spawn(move || {
            let resp = match agent.get(&url).call() {
                Ok(r) => r,
                Err(_) => return,
            };
            let _ = ready_tx.send(());
            for line in BufReader::new(resp.into_body().into_reader()).lines() {
                match line {
                    Ok(l) => {
                        if tx.send(l).is_err() {
                            return;
                        }
                    }
                    Err(_) => return,
                }
            }
        });
        ready_rx
            .recv_timeout(Duration::from_secs(10))
            .expect("/api/events did not open");
        rx
    }
}

/// A raw HTTP/1.1 connection to /api/events, for byte-exact assertions on the
/// stream. ureq hides headers' framing and strips line endings, so this talks
/// TCP directly. Reads accumulate in `buf` (headers included, chunk framing
/// left in; whole SSE frames are single writes so they stay contiguous).
pub struct RawEvents {
    stream: std::net::TcpStream,
    pub buf: Vec<u8>,
}

impl RawEvents {
    /// Read until `needle` shows up in the accumulated bytes (or `secs` pass).
    pub fn wait_for(&mut self, needle: &str, secs: u64) -> bool {
        use std::io::Read;
        let deadline = Instant::now() + Duration::from_secs(secs);
        loop {
            if String::from_utf8_lossy(&self.buf).contains(needle) {
                return true;
            }
            if Instant::now() > deadline {
                return false;
            }
            let mut chunk = [0u8; 4096];
            match self.stream.read(&mut chunk) {
                Ok(0) => return false,
                Ok(n) => self.buf.extend_from_slice(&chunk[..n]),
                Err(_) => {} // read timeout: loop and re-check the deadline
            }
        }
    }

    pub fn text(&self) -> String {
        String::from_utf8_lossy(&self.buf).into_owned()
    }
}

impl Daemon {
    pub fn events_raw(&self) -> RawEvents {
        use std::io::Write;
        let mut stream =
            std::net::TcpStream::connect(("127.0.0.1", self.port)).expect("connect /api/events");
        stream
            .set_read_timeout(Some(Duration::from_millis(200)))
            .unwrap();
        write!(
            stream,
            "GET /api/events HTTP/1.1\r\nHost: 127.0.0.1:{}\r\nAccept: text/event-stream\r\n\r\n",
            self.port
        )
        .unwrap();
        let mut ev = RawEvents {
            stream,
            buf: vec![],
        };
        assert!(
            ev.wait_for(": connected\n\n", 10),
            "no connected frame: {}",
            ev.text()
        );
        ev
    }
}

impl Drop for Daemon {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        // only ever our private socket in our private TMUX_TMPDIR
        let _ = self
            .tmux_cmd()
            .arg("kill-server")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
        let _ = fs::remove_dir_all(&self.root);
    }
}
