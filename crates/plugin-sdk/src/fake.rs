//! A scripted stand-in for the daemon, for plugin tests: answers the routes it
//! was given and records every request. Plain std, loopback only.

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::{Arc, Mutex};

/// One request the fake saw.
#[derive(Debug, Clone, PartialEq)]
pub struct Seen {
    pub method: String,
    pub path: String,
    pub body: serde_json::Value,
}

type RouteMap = Mutex<HashMap<(String, String), (u16, String)>>;
type Routes = Arc<RouteMap>;

pub struct FakeDaemon {
    pub base: String,
    seen: Arc<Mutex<Vec<Seen>>>,
    routes: Routes,
}

impl FakeDaemon {
    pub fn start() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        let base = format!("http://{}", listener.local_addr().unwrap());
        let seen = Arc::new(Mutex::new(Vec::new()));
        let routes: Routes = Arc::default();
        let (s, r) = (seen.clone(), routes.clone());
        std::thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                serve(stream, &s, &r);
            }
        });
        FakeDaemon { base, seen, routes }
    }

    /// Answer `method path` with `status` and a JSON `body`. Unrouted requests
    /// get `200 {"ok":true}`.
    pub fn route(&self, method: &str, path: &str, status: u16, body: &str) {
        self.routes
            .lock()
            .unwrap()
            .insert((method.into(), path.into()), (status, body.into()));
    }

    pub fn seen(&self) -> Vec<Seen> {
        self.seen.lock().unwrap().clone()
    }

    /// The bodies of every request to `method path`, oldest first.
    pub fn bodies(&self, method: &str, path: &str) -> Vec<serde_json::Value> {
        self.seen()
            .into_iter()
            .filter(|s| s.method == method && s.path == path)
            .map(|s| s.body)
            .collect()
    }
}

fn serve(stream: TcpStream, seen: &Mutex<Vec<Seen>>, routes: &RouteMap) {
    let mut reader = BufReader::new(stream.try_clone().unwrap());
    let mut first = String::new();
    if reader.read_line(&mut first).unwrap_or(0) == 0 {
        return;
    }
    let mut parts = first.split_whitespace();
    let (method, path) = (
        parts.next().unwrap_or("").to_owned(),
        parts.next().unwrap_or("").to_owned(),
    );
    let mut len = 0usize;
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line).unwrap_or(0) == 0 || line == "\r\n" {
            break;
        }
        if let Some(v) = line.to_ascii_lowercase().strip_prefix("content-length:") {
            len = v.trim().parse().unwrap_or(0);
        }
    }
    let mut body = vec![0; len];
    let _ = reader.read_exact(&mut body);
    let body = serde_json::from_slice(&body).unwrap_or(serde_json::Value::Null);
    let (status, reply) = routes
        .lock()
        .unwrap()
        .get(&(method.clone(), path.clone()))
        .cloned()
        .unwrap_or((200, r#"{"ok":true,"rev":1}"#.to_owned()));
    seen.lock().unwrap().push(Seen { method, path, body });
    let mut out = stream;
    let _ = write!(
        out,
        "HTTP/1.1 {status} X\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{reply}",
        reply.len()
    );
}
