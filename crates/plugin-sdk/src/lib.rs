#![doc = include_str!("../README.md")]
#![forbid(unsafe_code)]
#![warn(missing_docs)]

pub mod folder;
pub mod fuzzy;
#[cfg(feature = "testing")]
pub mod testing;
#[cfg(feature = "tui")]
pub mod tui;

use std::fmt;
use std::io::Read;
use std::path::{Path, PathBuf};

use serde::Serialize;
use serde::de::DeserializeOwned;

pub use garage_protocol::api::{ClosePaneResponse, OpenPaneResponse};
pub use garage_protocol::plugins::{PanesResponse, PluginList, WidgetsSnapshot};
pub use garage_protocol::widgets::{
    Click, Placement, Segment, Style, WidgetItem, WidgetPut, WidgetSlot, validate_put,
};
pub use serde_json::{Value, json};

/// Everything that can go wrong in a plugin run. More kinds may be added in a
/// minor release, so match with a catch-all arm.
#[derive(Debug)]
#[non_exhaustive]
pub enum Error {
    /// A needed environment variable is missing: not started by the daemon.
    Env(&'static str),
    /// The context could not be read or parsed.
    Context(String),
    /// The daemon answered with a non-2xx status; `body` is its text.
    Http {
        /// The HTTP status.
        status: u16,
        /// The reply text.
        body: String,
    },
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

/// What a `ui/action` applies to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum Target<'a> {
    /// A session id, `garage/<workspace>/<label>`.
    Session(&'a str),
    /// A workspace name.
    Workspace(&'a str),
}

/// A typed view of the run context. Every accessor returns `None` for a
/// `null` or missing field, as the context itself never omits keys.
#[derive(Debug, Clone)]
pub struct Context {
    raw: Value,
}

impl Context {
    /// Read the context of this run ([`read_context`]).
    pub fn read() -> Result<Self, Error> {
        read_context().map(Self::from_value)
    }

    /// A view over an already-parsed context (tests, or a context read some
    /// other way).
    pub fn from_value(raw: Value) -> Self {
        Self { raw }
    }

    /// The whole context, for what the accessors do not cover.
    pub fn raw(&self) -> &Value {
        &self.raw
    }

    /// The whole context, owned.
    pub fn into_raw(self) -> Value {
        self.raw
    }

    fn text(&self, pointer: &str) -> Option<&str> {
        self.raw
            .pointer(pointer)
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
    }

    /// The focused workspace; failing that the one the focused session is in.
    pub fn workspace(&self) -> Option<&str> {
        self.text("/focus/workspace")
    }

    /// The focused session id (`garage/<workspace>/<label>`).
    pub fn session(&self) -> Option<&str> {
        self.text("/focus/session/id")
    }

    /// Why this run started: `key`, `button`, `widget`, `palette`, `event`,
    /// `timer`, `pane` or `cli`.
    pub fn trigger_kind(&self) -> Option<&str> {
        self.text("/trigger/kind")
    }

    /// The `args` of the button or widget click that started the run (`{}`
    /// when there were none).
    pub fn trigger_args(&self) -> &Value {
        self.raw.pointer("/trigger/args").unwrap_or(&Value::Null)
    }

    /// The status of the focused session (`idle`, `working`, `needs-input`...).
    pub fn session_status(&self) -> Option<&str> {
        self.text("/focus/session/status")
    }

    /// The git branch of the focused session.
    pub fn session_branch(&self) -> Option<&str> {
        self.text("/focus/session/branch")
    }

    /// The directory of the focused session.
    pub fn session_dir(&self) -> Option<&str> {
        self.text("/focus/session/dir")
    }

    /// The event name of an event hook run.
    pub fn event_name(&self) -> Option<&str> {
        self.text("/event/name")
    }

    /// The event payload of an event hook run.
    pub fn event_data(&self) -> &Value {
        self.raw.pointer("/event/data").unwrap_or(&Value::Null)
    }

    /// The words after `claude-garage` of a CLI run.
    pub fn argv(&self) -> Vec<String> {
        self.raw
            .pointer("/trigger/args/argv")
            .and_then(Value::as_array)
            .map(|a| {
                a.iter()
                    .filter_map(Value::as_str)
                    .map(str::to_owned)
                    .collect()
            })
            .unwrap_or_default()
    }

    /// The plugin's own writable directory.
    pub fn state_dir(&self) -> Option<&str> {
        self.text("/plugin/state_dir")
    }

    /// The caller's working directory of a CLI run.
    pub fn cwd(&self) -> Option<&str> {
        self.text("/focus/dir")
    }
}

/// Ask the launcher to open the wall once this command has finished: writes
/// `{"open_wall": true}` to the file named by `GARAGE_HANDOVER` (set for
/// `[[cli]]` runs). The plugin then exits 0; the launcher starts the wall in
/// its own environment. `Ok(false)` when there is no launcher to ask (the
/// command was run by hand), so the caller can tell the user what to type.
pub fn request_wall() -> Result<bool, Error> {
    match std::env::var_os("GARAGE_HANDOVER") {
        Some(path) => request_wall_at(Path::new(&path)).map(|()| true),
        None => Ok(false),
    }
}

/// [`request_wall`] with the hand-over file given.
pub fn request_wall_at(path: &Path) -> Result<(), Error> {
    std::fs::write(path, r#"{"open_wall": true}"#)
        .map_err(|e| Error::Context(format!("{}: {e}", path.display())))
}

/// `workspace` of a session id `garage/<workspace>/<label>`.
pub fn workspace_of(session_id: &str) -> Option<&str> {
    let mut parts = session_id.split('/');
    (parts.next()? == "garage").then_some(())?;
    parts.next().filter(|w| !w.is_empty())
}

/// Does `sessions` (the `GET /api/sessions` array) hold any entry of
/// `workspace`? Restorable entries count: they are crashed sessions waiting to
/// be resumed, so the workspace is not empty.
pub fn workspace_has_sessions(sessions: &Value, workspace: &str) -> bool {
    sessions.as_array().into_iter().flatten().any(|s| {
        s.get("workspace").and_then(Value::as_str) == Some(workspace)
            || s.get("id").and_then(Value::as_str).and_then(workspace_of) == Some(workspace)
    })
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

/// [`read_state`] in a given directory.
pub fn read_state_in<T: DeserializeOwned>(dir: &std::path::Path, file: &str) -> Option<T> {
    serde_json::from_slice(&std::fs::read(dir.join(file)).ok()?).ok()
}

/// [`write_state`] in a given directory.
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
    /// Take the lock `name`, waiting up to `wait`. A hold older than `stale`
    /// is taken over. [`Error::LockHeld`] when the wait ends first. Dropping
    /// the returned value releases it.
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
    /// A client from the environment garage gives every run:
    /// `GARAGE_API`, `GARAGE_PLUGIN_ID` and `GARAGE_PLUGIN_TOKEN`.
    pub fn from_env() -> Result<Self, Error> {
        let get = |k: &'static str| std::env::var(k).map_err(|_| Error::Env(k));
        Ok(Self::new(
            &get("GARAGE_API")?,
            &get("GARAGE_PLUGIN_ID")?,
            &get("GARAGE_PLUGIN_TOKEN")?,
        ))
    }

    /// A client for the daemon at `base` (for example
    /// `http://127.0.0.1:4747`), acting as plugin `plugin` with `token`.
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

    /// `GET`; the JSON body.
    pub fn get(&self, path: &str) -> Result<Value, Error> {
        self.request("GET", path, None)
    }

    /// `POST` a JSON body.
    pub fn post(&self, path: &str, body: &Value) -> Result<Value, Error> {
        self.request("POST", path, Some(body))
    }

    /// `PUT` a JSON body.
    pub fn put(&self, path: &str, body: &Value) -> Result<Value, Error> {
        self.request("PUT", path, Some(body))
    }

    /// `DELETE`.
    pub fn delete(&self, path: &str) -> Result<Value, Error> {
        self.request("DELETE", path, None)
    }

    /// `GET /api/plugins/features`: the API versions and optional capabilities
    /// this garage has. The body is returned as JSON so that features added
    /// later never break an older plugin.
    pub fn features(&self) -> Result<Value, Error> {
        self.get("/api/plugins/features")
    }

    /// `GET /api/health`.
    pub fn health(&self) -> Result<Value, Error> {
        self.get("/api/health")
    }

    /// `GET /api/sessions`: live sessions, and restorable ones.
    pub fn sessions(&self) -> Result<Value, Error> {
        self.get("/api/sessions")
    }

    /// `GET /api/workspaces`.
    pub fn workspaces(&self) -> Result<Value, Error> {
        self.get("/api/workspaces")
    }

    /// `GET /api/usage`: the 5 hour and weekly rate-limit windows.
    pub fn usage(&self) -> Result<Value, Error> {
        self.get("/api/usage")
    }

    /// `GET /api/workspaces/{name}/closed`: the closed chats kept for a
    /// workspace. Needs `sessions.read`.
    pub fn closed_chats(&self, workspace: &str) -> Result<Value, Error> {
        self.get(&format!("/api/workspaces/{}/closed", encode(workspace)))
    }

    /// `GET /api/plugins`: the installed plugins with their actions, buttons,
    /// widgets and panes.
    pub fn plugins(&self) -> Result<PluginList, Error> {
        self.get_as("/api/plugins")
    }

    /// `GET /api/plugins/widgets`: what every plugin currently shows.
    pub fn widgets(&self) -> Result<WidgetsSnapshot, Error> {
        self.get_as("/api/plugins/widgets")
    }

    /// `GET /api/plugins/panes`: the open plugin panes.
    pub fn panes(&self) -> Result<PanesResponse, Error> {
        self.get_as("/api/plugins/panes")
    }

    /// `POST /api/plugins/{me}/panes/{pane}/open`. Needs `ui.panes`. `args`
    /// is passed to the pane's run context as `trigger.args`.
    pub fn open_pane(&self, pane: &str, args: Option<&Value>) -> Result<OpenPaneResponse, Error> {
        let empty = json!({});
        let res = self.post(
            &format!(
                "/api/plugins/{}/panes/{}/open",
                encode(&self.plugin),
                encode(pane)
            ),
            args.unwrap_or(&empty),
        )?;
        serde_json::from_value(res).map_err(|e| Error::Context(e.to_string()))
    }

    /// `DELETE /api/plugins/{me}/panes/{pane}`. Needs `ui.panes`. `true` when
    /// a pane was open and is now closed.
    pub fn close_pane(&self, pane: &str) -> Result<bool, Error> {
        let res = self.delete(&format!(
            "/api/plugins/{}/panes/{}",
            encode(&self.plugin),
            encode(pane)
        ))?;
        Ok(res.get("closed").and_then(Value::as_bool).unwrap_or(false))
    }

    fn get_as<T: DeserializeOwned>(&self, path: &str) -> Result<T, Error> {
        serde_json::from_value(self.get(path)?).map_err(|e| Error::Context(format!("{path}: {e}")))
    }

    /// `POST /api/plugins/ui/action`: ask the wall to run `action` (a core id
    /// or one of this plugin's actions) on `target`. Needs `ui.invoke`, and
    /// `core.actions` for a core id.
    pub fn ui_action(&self, action: &str, target: Target<'_>) -> Result<(), Error> {
        let t = match target {
            Target::Session(s) => json!({"session": s}),
            Target::Workspace(w) => json!({"workspace": w}),
        };
        self.post(
            "/api/plugins/ui/action",
            &json!({"action": action, "target": t}),
        )
        .map(|_| ())
    }

    /// `POST /api/sessions/input`: type `text` into the session and press
    /// Enter. Needs `sessions.input` (dangerous).
    pub fn session_input(&self, id: &str, text: &str) -> Result<(), Error> {
        self.post("/api/sessions/input", &json!({"id": id, "text": text}))
            .map(|_| ())
    }

    /// Like [`Api::session_input`], but the daemon refuses (409, `Error::Http`)
    /// when the session is in one of the `not` statuses at the moment the keys
    /// would go out, e.g. `["needs-input"]`.
    pub fn session_input_unless(&self, id: &str, text: &str, not: &[&str]) -> Result<(), Error> {
        self.post(
            "/api/sessions/input",
            &json!({"id": id, "text": text, "if_status": {"not": not}}),
        )
        .map(|_| ())
    }

    /// `PUT /api/plugins/{me}/widgets/{widget}`; returns the new `rev`.
    pub fn put_widget(&self, widget: &str, put: &WidgetPut) -> Result<u64, Error> {
        let body = serde_json::to_value(put).map_err(|e| Error::Context(e.to_string()))?;
        let res = self.request(
            "PUT",
            &format!(
                "/api/plugins/{}/widgets/{}",
                encode(&self.plugin),
                encode(widget)
            ),
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
            &format!("/api/plugins/{}/notice", encode(&self.plugin)),
            Some(&body),
        )
        .map(|_| ())
    }

    /// `DELETE /api/plugins/{me}/widgets/{widget}`.
    pub fn clear_widget(&self, widget: &str) -> Result<(), Error> {
        self.request(
            "DELETE",
            &format!(
                "/api/plugins/{}/widgets/{}",
                encode(&self.plugin),
                encode(widget)
            ),
            None,
        )
        .map(|_| ())
    }
}

/// Percent-encode one path segment (everything but unreserved characters).
fn encode(segment: &str) -> String {
    let mut out = String::with_capacity(segment.len());
    for b in segment.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~') {
            out.push(char::from(b));
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// A widget with one plain-text item for every scope (`*`): the common case
/// for a strip item.
pub fn text_widget(text: &str) -> WidgetPut {
    WidgetPut::new().scope(
        "*",
        vec![WidgetItem::new().segments(vec![Segment::new(text)])],
    )
}

#[doc(hidden)]
pub mod fake;

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufRead, BufReader, Write};

    #[test]
    fn asking_for_the_wall_writes_exactly_the_documented_request() {
        let d = tempfile::tempdir().unwrap();
        let f = d.path().join("handover.json");
        request_wall_at(&f).unwrap();
        let v: Value = serde_json::from_str(&std::fs::read_to_string(&f).unwrap()).unwrap();
        assert_eq!(v, json!({"open_wall": true}));
        assert!(
            request_wall_at(&d.path().join("no/such/dir/h.json")).is_err(),
            "a write that fails is reported, not swallowed"
        );
    }
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

    #[test]
    fn ui_action_and_session_input_send_the_documented_bodies() {
        let (base, h) = one_shot("200 OK", r#"{"ok":true}"#);
        Api::new(&base, "p", "tok")
            .ui_action("session.focus", Target::Session("garage/a/main"))
            .unwrap();
        let seen = h.join().unwrap();
        assert!(seen.starts_with("POST /api/plugins/ui/action "), "{seen}");
        let body: Value = serde_json::from_str(seen.rsplit('\n').next().unwrap()).unwrap();
        assert_eq!(
            body,
            json!({"action": "session.focus", "target": {"session": "garage/a/main"}})
        );

        let (base, h) = one_shot("200 OK", r#"{"ok":true}"#);
        Api::new(&base, "p", "tok")
            .session_input("garage/a/main", "ls -la")
            .unwrap();
        let seen = h.join().unwrap();
        assert!(seen.starts_with("POST /api/sessions/input "), "{seen}");
        let body: Value = serde_json::from_str(seen.rsplit('\n').next().unwrap()).unwrap();
        assert_eq!(body, json!({"id": "garage/a/main", "text": "ls -la"}));
    }

    #[test]
    fn the_context_view_reads_focus_event_and_argv_and_treats_null_as_none() {
        let c = Context::from_value(json!({
            "focus": {"workspace": "app", "session": {"id": "garage/app/main"}, "dir": "/work"},
            "event": {"name": "session.closed", "data": {"reason": "user"}},
            "trigger": {"args": {"argv": ["~/dev/x", "--no-open"]}},
        }));
        assert_eq!(c.workspace(), Some("app"));
        assert_eq!(c.session(), Some("garage/app/main"));
        assert_eq!(c.event_name(), Some("session.closed"));
        assert_eq!(c.event_data()["reason"], "user");
        assert_eq!(c.argv(), ["~/dev/x", "--no-open"]);
        assert_eq!(c.cwd(), Some("/work"));

        let empty = Context::from_value(
            json!({"focus": {"workspace": null, "session": null}, "event": null}),
        );
        assert_eq!(empty.workspace(), None);
        assert_eq!(empty.session(), None);
        assert_eq!(empty.event_name(), None);
        assert!(empty.event_data().is_null());
        assert!(empty.argv().is_empty());
    }

    #[test]
    fn a_workspace_has_sessions_when_any_entry_names_it_live_or_restorable() {
        let list = json!([
            {"id": "garage/other/a", "workspace": "other"},
            {"id": "garage/app/b", "workspace": "app", "restorable": true},
            {"id": "garage/third/c"},
        ]);
        assert!(workspace_has_sessions(&list, "app"), "restorable counts");
        assert!(
            workspace_has_sessions(&list, "third"),
            "matched by id when no field"
        );
        assert!(!workspace_has_sessions(&list, "ap"), "no prefix matching");
        assert!(!workspace_has_sessions(&json!([]), "app"));
        assert!(!workspace_has_sessions(&Value::Null, "app"));
    }

    #[test]
    fn workspace_of_reads_garage_session_ids_only() {
        assert_eq!(workspace_of("garage/app/main"), Some("app"));
        assert_eq!(workspace_of("other/app/main"), None);
        assert_eq!(workspace_of("garage//main"), None);
        assert_eq!(workspace_of("garage"), None);
    }

    #[test]
    fn the_read_and_pane_calls_use_the_documented_routes_and_encode_ids() {
        let fake = fake::FakeDaemon::start();
        fake.route("GET", "/api/sessions", 200, "[]");
        fake.route("GET", "/api/workspaces/my%20ws/closed", 200, "[]");
        fake.route(
            "POST",
            "/api/plugins/me/panes/details/open",
            200,
            r#"{"ok":true,"session":"plugin/me/details","reused":true}"#,
        );
        fake.route(
            "DELETE",
            "/api/plugins/me/panes/details",
            200,
            r#"{"ok":true,"closed":true}"#,
        );
        fake.route("GET", "/api/plugins/panes", 200, r#"{"panes":[]}"#);
        let api = Api::new(&fake.base, "me", "tok");
        assert_eq!(api.sessions().unwrap(), json!([]));
        assert_eq!(api.closed_chats("my ws").unwrap(), json!([]));
        let opened = api.open_pane("details", None).unwrap();
        assert_eq!(
            (opened.session.as_str(), opened.reused),
            ("plugin/me/details", true)
        );
        assert!(api.close_pane("details").unwrap());
        assert!(api.panes().unwrap().panes.is_empty());
        assert_eq!(
            fake.bodies("POST", "/api/plugins/me/panes/details/open"),
            [json!({})]
        );
    }

    #[test]
    fn path_segments_are_percent_encoded_except_unreserved_characters() {
        assert_eq!(encode("a-b_c.d~e"), "a-b_c.d~e");
        assert_eq!(encode("a b/c"), "a%20b%2Fc");
        assert_eq!(encode("\u{e9}"), "%C3%A9");
    }

    #[test]
    fn the_context_view_reads_trigger_and_session_details() {
        let ctx = Context::from_value(json!({
            "trigger": {"kind": "widget", "args": {"n": 1}},
            "focus": {"session": {"id": "garage/w/a", "status": "needs-input", "branch": "main", "dir": "/d"}}
        }));
        assert_eq!(ctx.trigger_kind(), Some("widget"));
        assert_eq!(ctx.trigger_args(), &json!({"n": 1}));
        assert_eq!(ctx.session_status(), Some("needs-input"));
        assert_eq!(ctx.session_branch(), Some("main"));
        assert_eq!(ctx.session_dir(), Some("/d"));
        assert_eq!(Context::from_value(json!({})).trigger_kind(), None);
    }
}
