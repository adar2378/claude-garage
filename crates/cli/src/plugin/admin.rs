//! Registry writes for the CLI lane. The daemon owns `plugins.json`: while it
//! is up, `link`, `install`, `enable`, `disable`, `remove` and `bind` go
//! through its UI-token routes, so its own writes (a first-party grant, a
//! manifest hash) and ours are serialised by one process. With the daemon
//! down they use the same locked `PluginsFile::update` helpers it does.
//! Reading is always direct.

use std::path::Path;
use std::time::Duration;

use garage_daemon::plugins::admin as direct;
pub use garage_daemon::plugins::admin::Registration;
use garage_daemon::plugins::layout::{ui_token_path, Paths, Source};
use serde_json::{json, Value};

pub trait Admin {
    /// Where writes went, for messages: "the running daemon" or "plugins.json".
    fn via(&self) -> &'static str;
    fn register(&self, r: &Registration) -> Result<(), String>;
    fn set_enabled(&self, id: &str, enabled: bool) -> Result<(), String>;
    /// Drops the entry (the daemon also tears the plugin down). Files are the
    /// caller's job.
    fn remove(&self, id: &str) -> Result<(), String>;
    fn bind(&self, id: &str, action: &str, key: Option<&str>) -> Result<(), String>;
}

pub struct DirectAdmin {
    pub paths: Paths,
}

impl Admin for DirectAdmin {
    fn via(&self) -> &'static str {
        "plugins.json (daemon not running)"
    }
    fn register(&self, r: &Registration) -> Result<(), String> {
        direct::register(&self.paths, r)
    }
    fn set_enabled(&self, id: &str, enabled: bool) -> Result<(), String> {
        direct::set_enabled(&self.paths, id, enabled)
    }
    fn remove(&self, id: &str) -> Result<(), String> {
        direct::remove_entry(&self.paths, id).map(|_| ())
    }
    fn bind(&self, id: &str, action: &str, key: Option<&str>) -> Result<(), String> {
        direct::set_binding(&self.paths, id, action, key)
    }
}

pub struct DaemonAdmin {
    pub base: String,
    pub token: String,
}

fn source_json(s: &Source) -> Value {
    serde_json::to_value(s).expect("Source serialises")
}

impl DaemonAdmin {
    fn call(&self, method: &str, path: &str, body: Option<Value>) -> Result<(), String> {
        let req = ureq::request(method, &format!("{}{path}", self.base))
            .timeout(Duration::from_secs(30))
            .set("Authorization", &format!("Bearer {}", self.token));
        let res = match body {
            Some(b) => req.set("Content-Type", "application/json").send_string(&b.to_string()),
            None => req.call(),
        };
        match res {
            Ok(_) => Ok(()),
            Err(ureq::Error::Status(code, r)) => {
                let text = r.into_string().unwrap_or_default();
                let msg = serde_json::from_str::<Value>(&text).ok().and_then(|v| v["error"].as_str().map(str::to_owned)).unwrap_or(text);
                Err(format!("daemon answered {code}: {msg}"))
            }
            Err(e) => Err(format!("cannot reach the daemon: {e}")),
        }
    }
}

impl Admin for DaemonAdmin {
    fn via(&self) -> &'static str {
        "the running daemon"
    }
    fn register(&self, r: &Registration) -> Result<(), String> {
        let body = json!({
            "source": source_json(&r.source),
            "grantedPermissions": r.granted_permissions,
            "manifestSha256": r.manifest_sha256,
            "installedAt": r.installed_at,
        });
        self.call("PUT", &format!("/api/plugins/{}/registration", r.id), Some(body))?;
        // The route reloads; the data directory and token are created by it too,
        // but the data dir is cheap to make sure of from here.
        Ok(())
    }
    fn set_enabled(&self, id: &str, enabled: bool) -> Result<(), String> {
        self.call("POST", &format!("/api/plugins/{id}/{}", if enabled { "enable" } else { "disable" }), None)
    }
    fn remove(&self, id: &str) -> Result<(), String> {
        self.call("DELETE", &format!("/api/plugins/{id}"), None)
    }
    fn bind(&self, id: &str, action: &str, key: Option<&str>) -> Result<(), String> {
        self.call("POST", &format!("/api/plugins/{id}/bind"), Some(json!({"action": action, "key": key})))
    }
}

/// Is a garage daemon answering on `port`?
pub fn daemon_up(port: u16) -> bool {
    ureq::get(&format!("http://127.0.0.1:{port}/api/plugin/features"))
        .timeout(Duration::from_millis(700))
        .call()
        .ok()
        .and_then(|r| r.into_string().ok())
        .is_some_and(|b| b.contains("api_versions"))
}

/// The daemon when it is up (and the UI token is readable), else direct.
pub fn choose(garage_dir: &Path, port: u16) -> Result<Box<dyn Admin>, String> {
    let paths = Paths::new(garage_dir);
    if daemon_up(port) {
        let tok = ui_token_path(garage_dir);
        let token = std::fs::read_to_string(&tok)
            .map_err(|e| format!("the daemon is running but {} is unreadable ({e}); fix that or stop the daemon", tok.display()))?;
        return Ok(Box::new(DaemonAdmin { base: format!("http://127.0.0.1:{port}"), token: token.trim().to_owned() }));
    }
    Ok(Box::new(DirectAdmin { paths }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use garage_daemon::plugins::layout::PluginsFile;
    use std::io::{Read, Write};
    use std::net::TcpListener;

    /// A one-shot HTTP server that records the request and answers `status`.
    fn serve_once(status: &str, body: &str) -> (u16, std::thread::JoinHandle<String>) {
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = l.local_addr().unwrap().port();
        let (status, body) = (status.to_owned(), body.to_owned());
        let h = std::thread::spawn(move || {
            let (mut s, _) = l.accept().unwrap();
            let mut buf = vec![0u8; 16384];
            let mut got = Vec::new();
            loop {
                let n = s.read(&mut buf).unwrap();
                got.extend_from_slice(&buf[..n]);
                let text = String::from_utf8_lossy(&got).to_string();
                if let Some(i) = text.find("\r\n\r\n") {
                    let len = text.to_lowercase().split("content-length:").nth(1).and_then(|r| r.split("\r\n").next()).and_then(|v| v.trim().parse::<usize>().ok()).unwrap_or(0);
                    if got.len() >= i + 4 + len {
                        break;
                    }
                }
                if n == 0 {
                    break;
                }
            }
            write!(s, "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).unwrap();
            String::from_utf8_lossy(&got).to_string()
        });
        (port, h)
    }

    fn admin(port: u16) -> DaemonAdmin {
        DaemonAdmin { base: format!("http://127.0.0.1:{port}"), token: "tok123".into() }
    }

    #[test]
    fn registering_through_the_daemon_sends_the_ui_token_and_the_grant() {
        let (port, h) = serve_once("200 OK", r#"{"ok":true}"#);
        admin(port)
            .register(&Registration {
                id: "demo-one".into(),
                source: Source::Github { repo: "o/r".into(), subdir: None, git_ref: Some("v1".into()), commit: Some("abc".into()) },
                granted_permissions: vec!["core.actions".into()],
                manifest_sha256: Some("ff".into()),
                installed_at: Some(9),
            })
            .unwrap();
        let req = h.join().unwrap();
        assert!(req.starts_with("PUT /api/plugins/demo-one/registration HTTP/1.1"), "{req}");
        assert!(req.to_lowercase().contains("authorization: bearer tok123"));
        let body: Value = serde_json::from_str(req.split("\r\n\r\n").nth(1).unwrap()).unwrap();
        assert_eq!(body["grantedPermissions"], json!(["core.actions"]));
        assert_eq!(body["source"], json!({"kind": "github", "repo": "o/r", "subdir": null, "ref": "v1", "commit": "abc"}));
        assert_eq!(body["manifestSha256"], "ff");
    }

    #[test]
    fn enable_remove_and_bind_use_the_documented_routes_and_errors_carry_the_daemons_message() {
        let (port, h) = serve_once("200 OK", "{}");
        admin(port).set_enabled("p-one", false).unwrap();
        assert!(h.join().unwrap().starts_with("POST /api/plugins/p-one/disable "));
        let (port, h) = serve_once("200 OK", "{}");
        admin(port).remove("p-one").unwrap();
        assert!(h.join().unwrap().starts_with("DELETE /api/plugins/p-one "));
        let (port, h) = serve_once("200 OK", "{}");
        admin(port).bind("p-one", "p-one:go", Some("g")).unwrap();
        let req = h.join().unwrap();
        assert!(req.starts_with("POST /api/plugins/p-one/bind ") && req.ends_with(r#"{"action":"p-one:go","key":"g"}"#), "{req}");
        let (port, h) = serve_once("409 Conflict", r#"{"error":"usage-bar is a bundled plugin and cannot be removed"}"#);
        let e = admin(port).remove("usage-bar").unwrap_err();
        h.join().unwrap();
        assert!(e.contains("409") && e.contains("bundled plugin"), "{e}");
    }

    #[test]
    fn with_nothing_listening_writes_go_straight_to_the_locked_registry() {
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = l.local_addr().unwrap().port();
        drop(l);
        assert!(!daemon_up(port));
        let d = tempfile::tempdir().unwrap();
        let a = choose(d.path(), port).unwrap();
        assert!(a.via().contains("daemon not running"));
        a.register(&Registration { id: "direct-one".into(), source: Source::Link { path: "/x".into() }, granted_permissions: vec![], manifest_sha256: None, installed_at: None }).unwrap();
        assert_eq!(PluginsFile::read(&Paths::new(d.path())).unwrap().plugins[0].id, "direct-one");
    }

    #[test]
    fn something_else_on_the_port_is_not_mistaken_for_the_daemon() {
        let (port, h) = serve_once("200 OK", "<html>hello</html>");
        assert!(!daemon_up(port));
        let _ = h.join();
    }
}
