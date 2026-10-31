//! The `{"error": "..."}` body every non-2xx response carries.

use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use garage_protocol::ErrorBody;

pub fn error_response(status: StatusCode, message: impl Into<String>) -> Response {
    (
        status,
        Json(ErrorBody {
            error: message.into(),
        }),
    )
        .into_response()
}
