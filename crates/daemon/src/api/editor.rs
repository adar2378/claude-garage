//! Port of `daemon/src/editor.js`: `POST /api/open-editor`.
//!
//! Runs `GARAGE_EDITOR_CMD` (default `code`) detached on the workspace
//! directory, or with `--goto <abs file>:<line>`, through `Os::open_window`.

use std::path::{Component, Path, PathBuf};
use std::sync::OnceLock;

use axum::Json;
use axum::body::Bytes;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde_json::{Value, json};

use super::hooks::parse_body;
use super::workspaces::js_display;
use crate::error::error_response;
use crate::os::OsError;
use crate::state::AppState;

/// `GARAGE_EDITOR_CMD`, read once like the JS module constant.
pub fn editor_cmd() -> &'static str {
    static CMD: OnceLock<String> = OnceLock::new();
    CMD.get_or_init(|| std::env::var("GARAGE_EDITOR_CMD").unwrap_or_else(|_| "code".into()))
}

/// Node's `path.resolve(dir, file)`: an absolute `file` replaces `dir`, and
/// `.`/`..` are folded lexically (symlinks are not consulted), so traversal is
/// caught however it is spelled.
pub fn resolve(dir: &str, file: &str) -> String {
    let mut base = PathBuf::new();
    if Path::new(file).is_absolute() {
        base.push(file);
    } else {
        let d = Path::new(dir);
        if d.is_absolute() {
            base.push(d);
        } else {
            base.push(std::env::current_dir().unwrap_or_else(|_| PathBuf::from("/")));
            base.push(d);
        }
        base.push(file);
    }
    let mut out = PathBuf::from("/");
    for c in base.components() {
        match c {
            Component::Normal(p) => out.push(p),
            Component::ParentDir => {
                out.pop();
            }
            Component::RootDir | Component::CurDir | Component::Prefix(_) => {}
        }
    }
    out.to_string_lossy().into_owned()
}

/// `line ?? 1` as JS would print it.
fn line_text(line: Option<&Value>) -> String {
    match line {
        None | Some(Value::Null) => "1".into(),
        Some(Value::Number(n)) => match n.as_f64() {
            Some(f) if f.fract() == 0.0 && f.abs() < 1e15 => format!("{}", f as i64),
            _ => n.to_string(),
        },
        other => js_display(other),
    }
}

pub async fn open(State(state): State<AppState>, body: Bytes) -> Response {
    let body = match parse_body(&body) {
        Ok(b) => b,
        Err(r) => return *r,
    };
    let workspace = body.get("workspace");
    let registered = match workspace.and_then(Value::as_str) {
        Some(name) => state.registry.get_workspace(name).await,
        None => None,
    };
    let Some(registered) = registered else {
        return error_response(
            StatusCode::NOT_FOUND,
            format!("unknown workspace: {}", js_display(workspace)),
        );
    };
    let Some(dir) = registered.dir else {
        return error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            "workspace has no directory",
        );
    };

    let cmd = editor_cmd();
    let args: Vec<String> = match body.get("file") {
        None | Some(Value::Null) => vec![dir.clone()],
        Some(Value::String(f)) if f.is_empty() => vec![dir.clone()],
        Some(Value::String(f)) => {
            // Check the RESOLVED path, not the raw string, so "../" traversal
            // and absolute-path overrides are both caught. The prefix test is
            // on the registered dir exactly as stored (JS quirk: a dir stored
            // with a trailing slash matches nothing but itself).
            let abs = resolve(&dir, f);
            if abs != dir && !abs.starts_with(&format!("{dir}/")) {
                return error_response(StatusCode::BAD_REQUEST, "file escapes workspace directory");
            }
            vec![
                "--goto".into(),
                format!("{abs}:{}", line_text(body.get("line"))),
            ]
        }
        Some(_) => return error_response(StatusCode::BAD_REQUEST, "file must be a string"),
    };

    let mut full = vec![cmd.to_owned()];
    full.extend(args);
    match state.os.open_window(&full).await {
        Ok(()) => Json(json!({"ok": true})).into_response(),
        Err(OsError::NotFound | OsError::Unsupported) => error_response(
            StatusCode::NOT_IMPLEMENTED,
            format!("editor CLI not found ({cmd})"),
        ),
        Err(OsError::Failed(m)) => error_response(StatusCode::INTERNAL_SERVER_ERROR, m),
    }
}
