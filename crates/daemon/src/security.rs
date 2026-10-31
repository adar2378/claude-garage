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

/// DNS-rebinding guard: a page on `evil.example` that resolves to 127.0.0.1
/// reaches the daemon with `Host: evil.example:<port>`, which no legitimate
/// client sends. Accept only the loopback names on the daemon's own port. A
/// request with no Host header (HTTP/1.0 tools) passes, like a missing Origin.
/// `port == 0` (tests binding an ephemeral port) skips the port comparison.
pub fn host_allowed(host: &str, port: u16) -> bool {
    let (name, given_port) = match host.strip_prefix('[') {
        // [::1]:4747
        Some(rest) => match rest.split_once(']') {
            Some((ip, tail)) => (format!("[{ip}]"), tail.strip_prefix(':')),
            None => return false,
        },
        None => match host.rsplit_once(':') {
            Some((n, p)) => (n.to_owned(), Some(p)),
            None => (host.to_owned(), None),
        },
    };
    let name = name.to_ascii_lowercase();
    if !matches!(name.as_str(), "127.0.0.1" | "localhost" | "[::1]") {
        return false;
    }
    port == 0 || given_port.and_then(|p| p.parse::<u16>().ok()) == Some(port)
}

pub async fn host_guard(State(state): State<AppState>, req: Request, next: Next) -> Response {
    if let Some(host) = req.headers().get(axum::http::header::HOST) {
        let ok = host.to_str().is_ok_and(|h| host_allowed(h, state.config.port));
        if !ok {
            return error_response(axum::http::StatusCode::FORBIDDEN, "host not allowed");
        }
    }
    next.run(req).await
}

pub async fn origin_guard(
    State(state): State<AppState>,
    req: Request,
    next: Next,
) -> Response {
    let state_changing = matches!(
        *req.method(),
        Method::POST | Method::PUT | Method::PATCH | Method::DELETE
    );
    if state_changing {
        if let Some(origin) = req.headers().get("origin") {
            let ok = origin.to_str().map(|o| state.config.ui_origins.iter().any(|a| a == o)).unwrap_or(false);
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

#[cfg(test)]
mod host_tests {
    use super::host_allowed;

    #[test]
    fn only_loopback_names_on_the_daemons_port_pass() {
        for h in ["127.0.0.1:4747", "localhost:4747", "LocalHost:4747", "[::1]:4747"] {
            assert!(host_allowed(h, 4747), "{h}");
        }
        for h in [
            "evil.example:4747", "127.0.0.1.evil.example:4747", "localhost.evil.example:4747",
            "127.0.0.1:4748", "127.0.0.1", "localhost", "[::1]", "[::1", "", "0.0.0.0:4747", "[::2]:4747",
            "127.0.0.1:4747:1", "127.0.0.1:",
        ] {
            assert!(!host_allowed(h, 4747), "{h:?}");
        }
        assert!(host_allowed("127.0.0.1:55555", 0), "ephemeral test port skips the port check");
        assert!(!host_allowed("evil.example:1", 0), "but never the name check");
    }
}
