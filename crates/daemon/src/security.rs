//! Port of `daemon/src/security.js` (origin guard) and `daemon/src/token-auth.js`
//! (constant-time hook token check; not ported yet).
//!
//! Browser pages can fire requests at 127.0.0.1 and always send `Origin`; the
//! TUI, Claude's hook posts and curl send none. So a missing `Origin` passes,
//! and any `Origin` is refused on state-changing methods unless listed.

use axum::extract::{Request, State};
use axum::http::Method;
use axum::middleware::Next;
use axum::response::Response;

use crate::error::error_response;
use crate::state::AppState;

pub async fn origin_guard(State(state): State<AppState>, req: Request, next: Next) -> Response {
    let state_changing = matches!(
        *req.method(),
        Method::POST | Method::PUT | Method::PATCH | Method::DELETE
    );
    if state_changing {
        if let Some(origin) = req.headers().get("origin") {
            let ok = origin
                .to_str()
                .map(|o| state.config.ui_origins.iter().any(|a| a == o))
                .unwrap_or(false);
            if !ok {
                return error_response(axum::http::StatusCode::FORBIDDEN, "origin not allowed");
            }
        }
    }
    next.run(req).await
}

/// Port of `daemon/src/token-auth.js`: constant-time check of a URL query
/// token against the per-install hook token, so response timing cannot leak
/// the expected token a byte at a time. A missing token never matches.
pub fn token_matches(given: Option<&str>, expected: &str) -> bool {
    let Some(given) = given else { return false };
    let (a, b) = (given.as_bytes(), expected.as_bytes());
    if a.len() != b.len() {
        return false;
    }
    a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

#[cfg(test)]
mod token_tests {
    use super::token_matches;

    #[test]
    fn matches_only_the_exact_token() {
        assert!(token_matches(Some("abc"), "abc"));
        assert!(!token_matches(Some("abd"), "abc"));
        assert!(!token_matches(Some("ab"), "abc"));
        assert!(!token_matches(Some(""), "abc"));
        assert!(!token_matches(None, "abc"));
    }
}
