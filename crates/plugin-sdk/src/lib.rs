//! Tiny helper for plugin binaries (p24 group 4; p27 grows it into the public
//! SDK). It does three things: read the run context, call the daemon with the
//! plugin's token, and push widgets. Everything else a plugin needs is a
//! plain HTTP call through [`Api::request`].
//!
//! The daemon passes the environment described in the plugin API docs:
//! `GARAGE_API`, `GARAGE_PLUGIN_ID`, `GARAGE_PLUGIN_TOKEN`, and the context as
//! `GARAGE_CONTEXT` (or `GARAGE_CONTEXT_FILE` when it is large) and on stdin.

pub mod fuzzy;
#[cfg(feature = "testing")]
pub mod testing;
#[cfg(feature = "tui")]
pub mod tui;

use std::fmt;
use std::io::Read;

pub use garage_protocol::widgets::{Click, Segment, Style, WidgetItem, WidgetPut};
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
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Env(name) => write!(f, "{name} is not set; run this through claude-garage"),
            Error::Context(e) => write!(f, "cannot read the run context: {e}"),
            Error::Http { status, body } => write!(f, "daemon answered {status}: {body}"),
            Error::Transport(e) => write!(f, "cannot reach the daemon: {e}"),
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
pub enum Target<'a> {
    Session(&'a str),
    Workspace(&'a str),
}

/// A typed view of the run context. Every accessor returns `None` for a
/// `null` or missing field, as the context itself never omits keys.
#[derive(Debug, Clone)]
pub struct Context(pub Value);

impl Context {
    pub fn read() -> Result<Self, Error> {
        read_context().map(Self)
    }

    fn text(&self, pointer: &str) -> Option<&str> {
        self.0
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

    /// The event name of an event hook run.
    pub fn event_name(&self) -> Option<&str> {
        self.text("/event/name")
    }

    /// The event payload of an event hook run.
    pub fn event_data(&self) -> &Value {
        self.0.pointer("/event/data").unwrap_or(&Value::Null)
    }

    /// The words after `claude-garage` of a CLI run.
    pub fn argv(&self) -> Vec<String> {
        self.0
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
            &format!("/api/plugins/{}/widgets/{widget}", self.plugin),
            Some(&body),
        )?;
        Ok(res.get("rev").and_then(Value::as_u64).unwrap_or(0))
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
        let c = Context(json!({
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

        let empty = Context(json!({"focus": {"workspace": null, "session": null}, "event": null}));
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
}
