//! Tiny helper for plugin binaries (p24 group 4; p27 grows it into the public
//! SDK). It does three things: read the run context, call the daemon with the
//! plugin's token, and push widgets. Everything else a plugin needs is a
//! plain HTTP call through [`Api::request`].
//!
//! The daemon passes the environment described in the plugin API docs:
//! `GARAGE_API`, `GARAGE_PLUGIN_ID`, `GARAGE_PLUGIN_TOKEN`, and the context as
//! `GARAGE_CONTEXT` (or `GARAGE_CONTEXT_FILE` when it is large) and on stdin.

use std::fmt;
use std::io::Read;
use std::path::PathBuf;

use serde::Serialize;
use serde::de::DeserializeOwned;

pub use garage_protocol::widgets::{
    Click, Placement, Segment, Style, WidgetItem, WidgetPut, WidgetSlot, validate_put,
};
pub use serde_json::{Value, json};

#[derive(Debug)]
pub enum Error {
    /// A needed environment variable is missing: not started by the daemon.
    Env(&'static str),
    /// The context could not be read or parsed.
    Context(String),
    /// The daemon answered with a non-2xx status; `body` is its text.
    Http { status: u16, body: String },
    /// The daemon could not be reached.
    Transport(String),
    /// A [`StateLock`] stayed held by another run past the wait. Not a
    /// failure of this run: it has nothing to do and should skip.
    LockHeld(String),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Env(name) => write!(f, "{name} is not set; run this through claude-garage"),
            Error::Context(e) => write!(f, "cannot read the run context: {e}"),
            Error::Http { status, body } => write!(f, "daemon answered {status}: {body}"),
            Error::Transport(e) => write!(f, "cannot reach the daemon: {e}"),
            Error::LockHeld(name) => write!(f, "{name}.lock is held by another run"),
        }
    }
}

impl std::error::Error for Error {}

/// The run context: `GARAGE_CONTEXT`, else the file `GARAGE_CONTEXT_FILE`
/// names, else stdin.
pub fn read_context() -> Result<Value, Error> {
    let raw = if let Ok(v) = std::env::var("GARAGE_CONTEXT") {
        v
    } else if let Ok(path) = std::env::var("GARAGE_CONTEXT_FILE") {
        std::fs::read_to_string(&path).map_err(|e| Error::Context(format!("{path}: {e}")))?
    } else {
        let mut s = String::new();
        std::io::stdin()
            .read_to_string(&mut s)
            .map_err(|e| Error::Context(format!("stdin: {e}")))?;
        s
    };
    serde_json::from_str(&raw).map_err(|e| Error::Context(e.to_string()))
}

/// The plugin's own storage directory (`GARAGE_PLUGIN_STATE_DIR`): created by
/// the daemon, kept across updates and restarts.
pub fn state_dir() -> Result<PathBuf, Error> {
    std::env::var_os("GARAGE_PLUGIN_STATE_DIR")
        .map(PathBuf::from)
        .ok_or(Error::Env("GARAGE_PLUGIN_STATE_DIR"))
}

/// Read `file` (JSON) from the storage directory. `None` when it is missing
/// or does not parse: stored state is a cache of what the plugin can work out
/// again, so a damaged file means "start fresh", never a failed run.
pub fn read_state<T: DeserializeOwned>(file: &str) -> Option<T> {
    read_state_in(&state_dir().ok()?, file)
}

/// Write `file` (JSON) to the storage directory: a temporary file in the same
/// directory, then a rename, so a reader never sees half a file.
pub fn write_state<T: Serialize>(file: &str, value: &T) -> Result<(), Error> {
    write_state_in(&state_dir()?, file, value)
}

pub fn read_state_in<T: DeserializeOwned>(dir: &std::path::Path, file: &str) -> Option<T> {
    serde_json::from_slice(&std::fs::read(dir.join(file)).ok()?).ok()
}

pub fn write_state_in<T: Serialize>(
    dir: &std::path::Path,
    file: &str,
    value: &T,
) -> Result<(), Error> {
    let io = |e: std::io::Error| Error::Context(format!("{file}: {e}"));
    std::fs::create_dir_all(dir).map_err(io)?;
    let tmp = dir.join(format!("{file}.{}.tmp", std::process::id()));
    let bytes = serde_json::to_vec(value).map_err(|e| Error::Context(e.to_string()))?;
    std::fs::write(&tmp, bytes).map_err(io)?;
    std::fs::rename(&tmp, dir.join(file)).map_err(io)
}

/// An exclusive hold on a file in the storage directory. The host may run
/// several runs of one plugin at once; a plugin that reads, changes and
/// rewrites its state takes this around the whole step. A lock older than
/// `stale` belongs to a run that died and is taken over.
pub struct StateLock {
    path: PathBuf,
}

impl StateLock {
    pub fn acquire(
        dir: &std::path::Path,
        name: &str,
        stale: std::time::Duration,
        wait: std::time::Duration,
    ) -> Result<StateLock, Error> {
        let io = |e: std::io::Error| Error::Context(format!("{name}.lock: {e}"));
        std::fs::create_dir_all(dir).map_err(io)?;
        let path = dir.join(format!("{name}.lock"));
        let start = std::time::Instant::now();
        loop {
            match std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&path)
            {
                Ok(_) => return Ok(StateLock { path }),
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                    let age = std::fs::metadata(&path)
                        .and_then(|m| m.modified())
                        .ok()
                        .and_then(|t| t.elapsed().ok());
                    if age.is_some_and(|a| a > stale) {
                        let _ = std::fs::remove_file(&path);
                        continue;
                    }
                    if start.elapsed() > wait {
                        return Err(Error::LockHeld(name.to_owned()));
                    }
                    std::thread::sleep(std::time::Duration::from_millis(15));
                }
                Err(e) => return Err(io(e)),
            }
        }
    }
}

impl Drop for StateLock {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

/// A client for the daemon's plugin routes.
pub struct Api {
    base: String,
    plugin: String,
    token: String,
    agent: ureq::Agent,
}

impl Api {
    pub fn from_env() -> Result<Self, Error> {
        let get = |k: &'static str| std::env::var(k).map_err(|_| Error::Env(k));
        Ok(Self::new(
            &get("GARAGE_API")?,
            &get("GARAGE_PLUGIN_ID")?,
            &get("GARAGE_PLUGIN_TOKEN")?,
        ))
    }

    pub fn new(base: &str, plugin: &str, token: &str) -> Self {
        let agent: ureq::Agent = ureq::Agent::config_builder()
            .timeout_global(Some(std::time::Duration::from_secs(10)))
            .http_status_as_error(false)
            .build()
            .into();
        Self {
            base: base.trim_end_matches('/').to_owned(),
            plugin: plugin.to_owned(),
            token: token.to_owned(),
            agent,
        }
    }

    /// One call with the plugin token. A non-2xx answer is [`Error::Http`].
    /// The result is the JSON body, or `Null` for an empty one.
    pub fn request(&self, method: &str, path: &str, body: Option<&Value>) -> Result<Value, Error> {
        let builder = ureq::http::Request::builder()
            .method(method)
            .uri(format!("{}{path}", self.base))
            .header("authorization", format!("Bearer {}", self.token))
            .header("content-type", "application/json");
        let req = builder
            .body(body.map(Value::to_string).unwrap_or_default())
            .map_err(|e| Error::Transport(e.to_string()))?;
        match self.agent.run(req) {
            Ok(mut r) => {
                let status = r.status();
                let text = r
                    .body_mut()
                    .read_to_string()
                    .map_err(|e| Error::Transport(e.to_string()))?;
                if status.is_success() {
                    Ok(serde_json::from_str(&text).unwrap_or(Value::Null))
                } else {
                    Err(Error::Http {
                        status: status.as_u16(),
                        body: text,
                    })
                }
            }
            Err(e) => Err(Error::Transport(e.to_string())),
        }
    }

    /// `GET path` (a core route such as `/api/usage` or `/api/sessions`).
    pub fn get(&self, path: &str) -> Result<Value, Error> {
        self.request("GET", path, None)
    }

    /// `PUT /api/plugins/{me}/widgets/{widget}`; returns the new `rev`.
    pub fn put_widget(&self, widget: &str, put: &WidgetPut) -> Result<u64, Error> {
        let body = serde_json::to_value(put).map_err(|e| Error::Context(e.to_string()))?;
        let res = self.request(
            "PUT",
            &format!("/api/plugins/{}/widgets/{widget}", self.plugin),
            Some(&body),
        )?;
        Ok(res.get("rev").and_then(Value::as_u64).unwrap_or(0))
    }

    /// `POST /api/plugins/{me}/notice`: a short toast for the user (needs the
    /// `ui.notice` permission; one a second, bursts of three).
    pub fn notice(&self, text: &str, ttl_ms: Option<u32>) -> Result<(), Error> {
        let mut body = json!({"text": text});
        if let Some(ms) = ttl_ms {
            body["ttl_ms"] = json!(ms);
        }
        self.request(
            "POST",
            &format!("/api/plugins/{}/notice", self.plugin),
            Some(&body),
        )
        .map(|_| ())
    }

    /// `DELETE /api/plugins/{me}/widgets/{widget}`.
    pub fn clear_widget(&self, widget: &str) -> Result<(), Error> {
        self.request(
            "DELETE",
            &format!("/api/plugins/{}/widgets/{widget}", self.plugin),
            None,
        )
        .map(|_| ())
    }
}

/// A widget with one plain-text item for every scope (`*`): the common case
/// for a strip item.
pub fn text_widget(text: &str) -> WidgetPut {
    let item = WidgetItem {
        segments: Some(vec![Segment {
            text: text.to_owned(),
            style: None,
            click: None,
        }]),
        ..Default::default()
    };
    WidgetPut {
        scopes: [("*".to_owned(), vec![item])].into(),
        ttl_secs: None,
    }
}

#[doc(hidden)]
pub mod fake;

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufRead, BufReader, Write};
    use std::net::TcpListener;

    /// Serve one canned response; returns the base URL and the request head+body.
    fn one_shot(status: &str, reply: &'static str) -> (String, std::thread::JoinHandle<String>) {
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}", l.local_addr().unwrap());
        let status = status.to_owned();
        let h = std::thread::spawn(move || {
            let (mut s, _) = l.accept().unwrap();
            let mut r = BufReader::new(s.try_clone().unwrap());
            let (mut head, mut len) = (String::new(), 0usize);
            loop {
                let mut line = String::new();
                r.read_line(&mut line).unwrap();
                if let Some(v) = line.to_ascii_lowercase().strip_prefix("content-length:") {
                    len = v.trim().parse().unwrap();
                }
                if line == "\r\n" {
                    break;
                }
                head.push_str(&line);
            }
            let mut body = vec![0; len];
            r.read_exact(&mut body).unwrap();
            write!(s, "HTTP/1.1 {status}\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{reply}", reply.len()).unwrap();
            format!("{head}\n{}", String::from_utf8(body).unwrap())
        });
        (base, h)
    }

    #[test]
    fn put_widget_sends_the_token_and_body_and_returns_the_rev() {
        let (base, h) = one_shot("200 OK", r#"{"ok":true,"rev":7}"#);
        let rev = Api::new(&base, "hello", "tok")
            .put_widget("greeting", &text_widget("hi"))
            .unwrap();
        assert_eq!(rev, 7);
        let seen = h.join().unwrap();
        assert!(
            seen.starts_with("PUT /api/plugins/hello/widgets/greeting "),
            "{seen}"
        );
        assert!(
            seen.to_ascii_lowercase()
                .contains("authorization: bearer tok"),
            "{seen}"
        );
        let body: serde_json::Value =
            serde_json::from_str(seen.rsplit('\n').next().unwrap()).unwrap();
        assert_eq!(
            body,
            serde_json::json!({"scopes":{"*":[{"priority":0,"placement":{"kind":"flow"},"segments":[{"text":"hi"}]}]}}),
            "{seen}"
        );
    }

    #[test]
    fn the_state_lock_excludes_until_released_and_takes_over_a_dead_one() {
        use std::time::Duration;
        let dir = std::env::temp_dir().join(format!("sdk-lock-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let a = StateLock::acquire(&dir, "s", Duration::from_secs(60), Duration::ZERO).unwrap();
        assert!(
            matches!(
                StateLock::acquire(
                    &dir,
                    "s",
                    Duration::from_secs(60),
                    Duration::from_millis(40)
                ),
                Err(Error::LockHeld(name)) if name == "s"
            ),
            "held: a distinct error, so a plugin can skip instead of failing"
        );
        drop(a);
        let b = StateLock::acquire(&dir, "s", Duration::from_secs(60), Duration::ZERO).unwrap();
        // A lock nobody released (its run died) is stale after the timeout.
        std::mem::forget(b);
        std::thread::sleep(Duration::from_millis(30));
        assert!(StateLock::acquire(&dir, "s", Duration::from_millis(10), Duration::ZERO).is_ok());
    }

    #[test]
    fn state_files_round_trip_and_a_damaged_one_reads_as_absent() {
        let dir = std::env::temp_dir().join(format!("sdk-state-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        write_state_in(&dir, "x.json", &json!({"a": 1})).unwrap();
        assert_eq!(
            read_state_in::<Value>(&dir, "x.json"),
            Some(json!({"a": 1}))
        );
        std::fs::write(dir.join("x.json"), "{not json").unwrap();
        assert_eq!(read_state_in::<Value>(&dir, "x.json"), None);
        assert_eq!(read_state_in::<Value>(&dir, "missing.json"), None);
    }

    #[test]
    fn a_refusal_surfaces_status_and_text() {
        let (base, h) = one_shot(
            "403 Forbidden",
            r#"{"error":"plugin hello lacks permission ui.widgets"}"#,
        );
        match Api::new(&base, "hello", "tok").clear_widget("greeting") {
            Err(Error::Http { status: 403, body }) => {
                assert!(body.contains("ui.widgets"), "{body}")
            }
            other => panic!("{other:?}"),
        }
        h.join().unwrap();
    }
}
