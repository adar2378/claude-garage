//! A fake daemon for plugin tests (feature `testing`): a local HTTP server
//! that answers from a closure and records every request, so a plugin's
//! whole flow can be run against it without a daemon or tmux.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;

use serde_json::Value;

use crate::Api;

/// One request the fake saw.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct Req {
    /// HTTP method.
    pub method: String,
    /// Path with the query string.
    pub path: String,
    /// The `Authorization` header, if any.
    pub authorization: Option<String>,
    /// The JSON body, or `Null`.
    pub body: Value,
}

type Responder = dyn Fn(&Req) -> (u16, String) + Send + Sync;

/// A local HTTP server that records every request.
pub struct FakeDaemon {
    /// Its base URL, for `GARAGE_API`.
    pub base: String,
    seen: Arc<Mutex<Vec<Req>>>,
    stop: Arc<AtomicBool>,
    handle: Option<JoinHandle<()>>,
}

impl FakeDaemon {
    /// `reply` maps a request to `(status, json text)`.
    pub fn start(reply: impl Fn(&Req) -> (u16, String) + Send + Sync + 'static) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        let base = format!("http://{}", listener.local_addr().expect("addr"));
        let seen = Arc::new(Mutex::new(Vec::new()));
        let stop = Arc::new(AtomicBool::new(false));
        let responder: Arc<Responder> = Arc::new(reply);
        let (s2, stop2) = (seen.clone(), stop.clone());
        let handle = std::thread::spawn(move || {
            for conn in listener.incoming() {
                if stop2.load(Ordering::SeqCst) {
                    break;
                }
                if let Ok(conn) = conn {
                    serve(conn, &responder, &s2);
                }
            }
        });
        Self {
            base,
            seen,
            stop,
            handle: Some(handle),
        }
    }

    /// An [`Api`] pointed at this fake with the token `tok`.
    pub fn api(&self, plugin: &str) -> Api {
        Api::new(&self.base, plugin, "tok")
    }

    /// Every request seen so far.
    pub fn requests(&self) -> Vec<Req> {
        self.seen.lock().expect("lock").clone()
    }

    /// `METHOD path` of every request, in order.
    pub fn log(&self) -> Vec<String> {
        self.requests()
            .iter()
            .map(|r| format!("{} {}", r.method, r.path))
            .collect()
    }
}

impl Drop for FakeDaemon {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        let _ = TcpStream::connect(self.base.trim_start_matches("http://"));
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
    }
}

fn serve(conn: TcpStream, reply: &Arc<Responder>, seen: &Arc<Mutex<Vec<Req>>>) {
    let Ok(read_half) = conn.try_clone() else {
        return;
    };
    let mut conn = conn;
    let mut r = BufReader::new(read_half);
    let mut first = String::new();
    if r.read_line(&mut first).unwrap_or(0) == 0 {
        return;
    }
    let mut parts = first.split_whitespace();
    let (method, path) = (
        parts.next().unwrap_or("").to_owned(),
        parts.next().unwrap_or("").to_owned(),
    );
    let (mut len, mut authorization) = (0usize, None);
    loop {
        let mut line = String::new();
        if r.read_line(&mut line).unwrap_or(0) == 0 || line == "\r\n" {
            break;
        }
        let lower = line.to_ascii_lowercase();
        if let Some(v) = lower.strip_prefix("content-length:") {
            len = v.trim().parse().unwrap_or(0);
        }
        if lower.starts_with("authorization:") {
            authorization = line.split_once(':').map(|(_, v)| v.trim().to_owned());
        }
    }
    let mut body = vec![0; len];
    let _ = r.read_exact(&mut body);
    let req = Req {
        method,
        path,
        authorization,
        body: serde_json::from_slice(&body).unwrap_or(Value::Null),
    };
    let (status, text) = reply(&req);
    seen.lock().expect("lock").push(req);
    let _ = write!(
        conn,
        "HTTP/1.1 {status} X\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{text}",
        text.len()
    );
}
