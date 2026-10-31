//! `GET /api/attach?id=<session id>`: how the TUI attaches to a session on the
//! running backend, as an `AttachSpec` (a tmux argv, or a host socket and
//! token). The session id has slashes, so it rides in the query string.

use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use garage_protocol::AttachSpec;
use serde::Deserialize;

use crate::backend::tmux::GARAGE_PREFIX;
use crate::error::error_response;
use crate::state::AppState;

#[derive(Deserialize)]
pub struct Params {
    id: Option<String>,
}

pub async fn get(State(state): State<AppState>, Query(p): Query<Params>) -> Response {
    let Some(id) = p.id.filter(|i| i.starts_with(GARAGE_PREFIX)) else {
        return error_response(StatusCode::BAD_REQUEST, "id must be a garage session id");
    };
    match state.backend.attach_spec(&id) {
        // A host backend answers with an empty address when it has no such host.
        AttachSpec::Socket { addr, .. } if addr.is_empty() => {
            error_response(StatusCode::NOT_FOUND, format!("no running session {id}"))
        }
        spec => Json(spec).into_response(),
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use axum::body::Body;
    use axum::http::Request;
    use http_body_util::BodyExt;
    use serde_json::{json, Value};
    use tower::ServiceExt;

    use crate::backend::host::HostBackend;
    use crate::backend::tmux::TmuxBackend;
    use crate::backend::SessionBackend;
    use crate::{app, AppState, Config};

    async fn get(backend: Arc<dyn SessionBackend>, uri: &str) -> (StatusCode, Value) {
        let config = Config { port: 0, garage_dir: "/nonexistent".into(), tmux_socket: None, ui_origins: vec![] };
        let state = AppState::new(config, backend, Arc::new(crate::os::UnsupportedOs));
        let res = app(state).oneshot(Request::builder().uri(uri).body(Body::empty()).unwrap()).await.unwrap();
        let status = res.status();
        let bytes = res.into_body().collect().await.unwrap().to_bytes();
        (status, serde_json::from_slice(&bytes).unwrap_or(Value::Null))
    }

    use axum::http::StatusCode;

    #[tokio::test]
    async fn tmux_backend_answers_with_the_attach_argv_including_the_socket() {
        let (s, body) = get(Arc::new(TmuxBackend::new(Some("t1".into()))), "/api/attach?id=garage/ws/a").await;
        assert_eq!(s, StatusCode::OK);
        assert_eq!(
            body,
            json!({"kind": "tmux", "argv": ["tmux", "-L", "t1", "attach", "-t", "=garage/ws/a"],
                   "detachArgv": ["tmux", "-L", "t1", "detach-client", "-t"]})
        );
    }

    #[tokio::test]
    async fn host_backend_without_that_host_is_404_and_bad_ids_are_400() {
        let dir = tempfile::tempdir().unwrap();
        let b = || Arc::new(HostBackend::new(dir.path().to_path_buf(), vec![]));
        let (s, body) = get(b(), "/api/attach?id=garage/ws/none").await;
        assert_eq!(s, StatusCode::NOT_FOUND);
        assert!(body["error"].as_str().unwrap().contains("garage/ws/none"));
        for uri in ["/api/attach", "/api/attach?id=other/thing"] {
            assert_eq!(get(b(), uri).await.0, StatusCode::BAD_REQUEST, "{uri}");
        }
    }

    #[tokio::test]
    async fn host_backend_with_a_record_answers_with_its_socket_and_token() {
        let dir = tempfile::tempdir().unwrap();
        garage_host::registry::write(
            dir.path(),
            &garage_host::registry::Entry {
                id: "garage/ws/a".into(), pid: 1, child_pid: 2, port: 5555, token: "t".repeat(32),
                dir: "/w".into(), created_at: 1, title: String::new(),
            },
        )
        .unwrap();
        let (s, body) = get(Arc::new(HostBackend::new(dir.path().to_path_buf(), vec![])), "/api/attach?id=garage/ws/a").await;
        assert_eq!(s, StatusCode::OK);
        assert_eq!(body, json!({"kind": "socket", "addr": "127.0.0.1:5555", "token": "t".repeat(32)}));
    }
}
