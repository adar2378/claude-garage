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

pub use garage_protocol::widgets::{Segment, Style, WidgetItem, WidgetPut};
pub use serde_json::{json, Value};

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
        std::io::stdin().read_to_string(&mut s).map_err(|e| Error::Context(format!("stdin: {e}")))?;
        s
    };
    serde_json::from_str(&raw).map_err(|e| Error::Context(e.to_string()))
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
        Ok(Self::new(&get("GARAGE_API")?, &get("GARAGE_PLUGIN_ID")?, &get("GARAGE_PLUGIN_TOKEN")?))
    }

    pub fn new(base: &str, plugin: &str, token: &str) -> Self {
        let agent = ureq::AgentBuilder::new().timeout(std::time::Duration::from_secs(10)).build();
        Self { base: base.trim_end_matches('/').to_owned(), plugin: plugin.to_owned(), token: token.to_owned(), agent }
    }

    /// One call with the plugin token. A non-2xx answer is [`Error::Http`].
    /// The result is the JSON body, or `Null` for an empty one.
    pub fn request(&self, method: &str, path: &str, body: Option<&Value>) -> Result<Value, Error> {
        let req = self
            .agent
            .request(method, &format!("{}{path}", self.base))
            .set("authorization", &format!("Bearer {}", self.token))
            .set("content-type", "application/json");
        let res = match body {
            Some(b) => req.send_string(&b.to_string()),
            None => req.call(),
        };
        match res {
            Ok(r) => {
                let text = r.into_string().map_err(|e| Error::Transport(e.to_string()))?;
                Ok(serde_json::from_str(&text).unwrap_or(Value::Null))
            }
            Err(ureq::Error::Status(status, r)) => Err(Error::Http { status, body: r.into_string().unwrap_or_default() }),
            Err(e) => Err(Error::Transport(e.to_string())),
        }
    }

    /// `PUT /api/plugins/{me}/widgets/{widget}`; returns the new `rev`.
    pub fn put_widget(&self, widget: &str, put: &WidgetPut) -> Result<u64, Error> {
        let body = serde_json::to_value(put).map_err(|e| Error::Context(e.to_string()))?;
        let res = self.request("PUT", &format!("/api/plugins/{}/widgets/{widget}", self.plugin), Some(&body))?;
        Ok(res.get("rev").and_then(Value::as_u64).unwrap_or(0))
    }

    /// `DELETE /api/plugins/{me}/widgets/{widget}`.
    pub fn clear_widget(&self, widget: &str) -> Result<(), Error> {
        self.request("DELETE", &format!("/api/plugins/{}/widgets/{widget}", self.plugin), None).map(|_| ())
    }
}

/// A widget with one plain-text item for every scope (`*`): the common case
/// for a strip item.
pub fn text_widget(text: &str) -> WidgetPut {
    let item = WidgetItem { segments: Some(vec![Segment { text: text.to_owned(), style: None, click: None }]), ..Default::default() };
    WidgetPut { scopes: [("*".to_owned(), vec![item])].into(), ttl_secs: None }
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
        let rev = Api::new(&base, "hello", "tok").put_widget("greeting", &text_widget("hi")).unwrap();
        assert_eq!(rev, 7);
        let seen = h.join().unwrap();
        assert!(seen.starts_with("PUT /api/plugins/hello/widgets/greeting "), "{seen}");
        assert!(seen.to_ascii_lowercase().contains("authorization: bearer tok"), "{seen}");
        assert!(seen.ends_with(r#"{"scopes":{"*":[{"priority":0,"placement":{"kind":"flow"},"segments":[{"text":"hi"}]}]}}"#), "{seen}");
    }

    #[test]
    fn a_refusal_surfaces_status_and_text() {
        let (base, h) = one_shot("403 Forbidden", r#"{"error":"plugin hello lacks permission ui.widgets"}"#);
        match Api::new(&base, "hello", "tok").clear_widget("greeting") {
            Err(Error::Http { status: 403, body }) => assert!(body.contains("ui.widgets"), "{body}"),
            other => panic!("{other:?}"),
        }
        h.join().unwrap();
    }
}
