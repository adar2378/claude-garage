//! The error body every non-2xx response carries:
//! `{"error": "<message>", "code": "<stable_snake_code>"}`, plus
//! `"permission"` when a plugin lacks one. `code` is the contract (see
//! docs/plugins/api.md); `error` is for people and may be reworded.

use axum::Json;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use garage_protocol::{ErrorBody, ErrorCode};

/// What a status means when the route has nothing more specific to say.
pub fn default_code(status: StatusCode) -> ErrorCode {
    match status.as_u16() {
        400 => ErrorCode::BadRequest,
        401 => ErrorCode::Unauthorized,
        403 => ErrorCode::Forbidden,
        404 => ErrorCode::NotFound,
        409 => ErrorCode::Conflict,
        413 => ErrorCode::PayloadTooLarge,
        422 => ErrorCode::InvalidRequest,
        429 => ErrorCode::RateLimited,
        500 => ErrorCode::InternalError,
        501 => ErrorCode::NotImplemented,
        503 => ErrorCode::Unavailable,
        _ => ErrorCode::Unspecified,
    }
}

/// An error with the default code of its status.
pub fn error_response(status: StatusCode, message: impl Into<String>) -> Response {
    coded(status, default_code(status), message)
}

/// An error with a specific stable `code`.
pub fn coded(status: StatusCode, code: ErrorCode, message: impl Into<String>) -> Response {
    body(status, code, message, None)
}

/// 403 for a plugin that lacks `permission`.
pub fn permission_denied(permission: &str, message: impl Into<String>) -> Response {
    body(
        StatusCode::FORBIDDEN,
        ErrorCode::PermissionDenied,
        message,
        Some(permission),
    )
}

fn body(
    status: StatusCode,
    code: ErrorCode,
    message: impl Into<String>,
    permission: Option<&str>,
) -> Response {
    (
        status,
        Json(ErrorBody {
            error: message.into(),
            code: code.as_str().to_owned(),
            permission: permission.map(str::to_owned),
        }),
    )
        .into_response()
}

/// The body a test expects for a plain error of `status`.
#[cfg(test)]
pub fn expected(status: StatusCode, message: impl Into<String>) -> serde_json::Value {
    serde_json::json!({"error": message.into(), "code": default_code(status).as_str()})
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn json_of(r: Response) -> serde_json::Value {
        let bytes = axum::body::to_bytes(r.into_body(), 1 << 16).await.unwrap();
        serde_json::from_slice(&bytes).unwrap()
    }

    #[tokio::test]
    async fn a_plain_error_carries_its_status_code_and_no_permission() {
        let b = json_of(error_response(StatusCode::NOT_FOUND, "gone")).await;
        assert_eq!(b, serde_json::json!({"error": "gone", "code": "not_found"}));
    }

    #[tokio::test]
    async fn a_missing_permission_is_named_in_the_body() {
        let r = permission_denied("ui.widgets", "plugin x lacks permission ui.widgets");
        assert_eq!(r.status(), StatusCode::FORBIDDEN);
        let b = json_of(r).await;
        assert_eq!(b["code"], "permission_denied");
        assert_eq!(b["permission"], "ui.widgets");
    }
}
