//! Port of `daemon/src/security.js` (origin guard) and `daemon/src/token-auth.js`
//! (constant-time hook token check; not ported yet).
//!
//! Browser pages can fire requests at 127.0.0.1 and always send `Origin`; the
//! TUI, Claude's hook posts and curl send none. So a missing `Origin` passes,
//! and any `Origin` is refused on state-changing methods unless listed.

use std::sync::Arc;

use axum::extract::{Request, State};
use axum::http::Method;
use axum::middleware::Next;
use axum::response::Response;

use crate::error::error_response;

pub async fn origin_guard(
    State(allowed): State<Arc<Vec<String>>>,
    req: Request,
    next: Next,
) -> Response {
    let state_changing = matches!(
        *req.method(),
        Method::POST | Method::PUT | Method::PATCH | Method::DELETE
    );
    if state_changing {
        if let Some(origin) = req.headers().get("origin") {
            let ok = origin.to_str().map(|o| allowed.iter().any(|a| a == o)).unwrap_or(false);
            if !ok {
                return error_response(axum::http::StatusCode::FORBIDDEN, "origin not allowed");
            }
        }
    }
    next.run(req).await
}
