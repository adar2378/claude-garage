//! `GET /api/attach?id=<session id>`: how the TUI attaches to a session on the
//! running backend, as an `AttachSpec` (a tmux socket name, or the id of a session host; never a host's token, which the
//! client reads from the user-private registry file). The session id has slashes, so it rides in the query string.

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
        // A host backend answers with an empty id when it has no such host.
        AttachSpec::Host { id: found } if found.is_empty() => {
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
        let config = Config { port: 0, garage_dir: "/nonexistent".into(), tmux_socket: None, tmux_control: false, ui_origins: vec![] };
        let state = AppState::new(config, backend, Arc::new(crate::os::UnsupportedOs));
        let res = app(state).oneshot(Request::builder().uri(uri).body(Body::empty()).unwrap()).await.unwrap();
        let status = res.status();
        let bytes = res.into_body().collect().await.unwrap().to_bytes();
        (status, serde_json::from_slice(&bytes).unwrap_or(Value::Null))
    }

    use axum::http::StatusCode;

    #[tokio::test]
    async fn tmux_backend_answers_with_data_only_the_socket_name_when_there_is_one() {
        let (s, body) = get(Arc::new(TmuxBackend::new(Some("t1".into()))), "/api/attach?id=garage/ws/a").await;
        assert_eq!((s, body), (StatusCode::OK, json!({"kind": "tmux", "socket": "t1"})));
        let (s, body) = get(Arc::new(TmuxBackend::new(None)), "/api/attach?id=garage/ws/a").await;
        assert_eq!((s, body), (StatusCode::OK, json!({"kind": "tmux"})));
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
    async fn host_backend_with_a_record_answers_with_the_id_only_never_the_token() {
        let dir = tempfile::tempdir().unwrap();
        let token = "s3cr3t".repeat(6)[..32].to_string();
        garage_host::registry::write(
            dir.path(),
            &garage_host::registry::Entry {
                id: "garage/ws/a".into(), pid: 1, child_pid: 2, port: 5555, token: token.clone(),
                dir: "/w".into(), created_at: 1, title: String::new(),
            },
        )
        .unwrap();
        let (s, body) = get(Arc::new(HostBackend::new(dir.path().to_path_buf(), vec![])), "/api/attach?id=garage/ws/a").await;
        assert_eq!(s, StatusCode::OK);
        assert_eq!(body, json!({"kind": "host", "id": "garage/ws/a"}));
        assert!(!body.to_string().contains(&token) && !body.to_string().contains("5555"));
    }
}
