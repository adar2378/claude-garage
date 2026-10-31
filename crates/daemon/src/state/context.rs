//! Port of the context store in `daemon/src/statusline.js` (per-session
//! context, account-wide rate limits) plus the `Telemetry` bundle that lane C
//! hangs off `AppState`.
//!
//! Like `status.js`, the stores are in-memory with no persistence. Two slots:
//! per-session context percentage, and ONE account-wide rate-limits record
//! (every statusline post carries the whole account's limits).

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use anyhow::{anyhow, Result};
use async_trait::async_trait;
use garage_protocol::SessionStatus;
use serde_json::{json, Value};

use crate::transcript::TranscriptCache;

/// What lane C needs from the registry (lane A), status store (lane A) and
/// session resolution (lane B). The merge replaces `UnwiredDeps` with an
/// adapter over the real stores.
#[async_trait]
pub trait TelemetryDeps: Send + Sync {
    /// `getHookToken()`: the per-install secret, generated and persisted once.
    async fn hook_token(&self) -> Result<String>;
    /// `setStatus(id, state, message)`. `message` is three-valued like the JS
    /// argument: `None` = `undefined` (not passed), `Some(None)` = `null`,
    /// `Some(Some(m))` = text.
    fn set_status(&self, id: &str, state: SessionStatus, message: Option<Option<String>>);
    /// `resolveSessionIds({session_id, cwd})` from `session-resolve.js`.
    async fn resolve_session_ids(&self, session_id: Option<&str>, cwd: Option<&str>) -> Vec<String>;
}

/// Default until the merge wires real stores: nothing resolves, and the token
/// is an error (routes answer 500) rather than a guessable constant.
pub struct UnwiredDeps;

#[async_trait]
impl TelemetryDeps for UnwiredDeps {
    async fn hook_token(&self) -> Result<String> {
        Err(anyhow!("hook token store is not wired"))
    }
    fn set_status(&self, _: &str, _: SessionStatus, _: Option<Option<String>>) {}
    async fn resolve_session_ids(&self, _: Option<&str>, _: Option<&str>) -> Vec<String> {
        Vec::new()
    }
}

/// `clampPercentage`: finite numbers clamped into `[0, 100]`, else `None`.
pub fn clamp_percentage(value: &Value) -> Option<f64> {
    let n = value.as_f64().filter(|n| n.is_finite())?;
    Some(n.clamp(0.0, 100.0))
}

/// Whole numbers serialize as integers (`42`, not `42.0`), as JS does.
pub fn number(n: f64) -> Value {
    if n.fract() == 0.0 && n.abs() < 9.0e15 {
        json!(n as i64)
    } else {
        json!(n)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Limit {
    pub used_percentage: f64,
    /// `resets_at ?? null`, passed through untouched.
    pub resets_at: Value,
}

impl Limit {
    fn to_json(&self) -> Value {
        json!({"usedPercentage": number(self.used_percentage), "resetsAt": self.resets_at})
    }
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct RateLimits {
    pub five_hour: Option<Limit>,
    pub seven_day: Option<Limit>,
}

impl RateLimits {
    /// The `GET /api/usage` body.
    pub fn to_json(&self) -> Value {
        json!({
            "fiveHour": self.five_hour.as_ref().map(Limit::to_json),
            "sevenDay": self.seven_day.as_ref().map(Limit::to_json),
        })
    }
}

fn normalize_limit(limit: Option<&Value>) -> Option<Limit> {
    let obj = limit?.as_object()?;
    let used_percentage = clamp_percentage(obj.get("used_percentage")?)?;
    let resets_at = obj.get("resets_at").cloned().unwrap_or(Value::Null);
    Some(Limit { used_percentage, resets_at })
}

#[derive(Default)]
struct Inner {
    contexts: HashMap<String, f64>,
    rate_limits: RateLimits,
}

#[derive(Clone, Default)]
pub struct ContextStore {
    inner: Arc<Mutex<Inner>>,
}

impl ContextStore {
    pub fn set_context(&self, id: &str, used_percentage: f64) {
        self.inner.lock().unwrap().contexts.insert(id.to_owned(), used_percentage);
    }

    /// `{usedPercentage, source: "statusline"}` or `None`, the shape sessions.js
    /// puts on the listing entry.
    pub fn statusline_context(&self, id: &str) -> Option<Value> {
        let pct = *self.inner.lock().unwrap().contexts.get(id)?;
        Some(json!({"usedPercentage": number(pct), "source": "statusline"}))
    }

    /// Called when a session disappears so a reused id starts clean.
    pub fn drop_session(&self, id: &str) {
        self.inner.lock().unwrap().contexts.remove(id);
    }

    /// Partial-update semantics: a post carrying only one bucket updates that
    /// bucket and leaves the other exactly as it was.
    pub fn set_rate_limits(&self, raw: &Value) {
        let Some(obj) = raw.as_object() else { return };
        let five = normalize_limit(obj.get("five_hour"));
        let seven = normalize_limit(obj.get("seven_day"));
        let mut inner = self.inner.lock().unwrap();
        if five.is_some() {
            inner.rate_limits.five_hour = five;
        }
        if seven.is_some() {
            inner.rate_limits.seven_day = seven;
        }
    }

    pub fn rate_limits(&self) -> RateLimits {
        self.inner.lock().unwrap().rate_limits.clone()
    }
}

/// Everything lane C owns, shared by the hooks / statusline / usage routes and
/// (for context and the transcript cache) by the sessions routes.
#[derive(Clone)]
pub struct Telemetry {
    /// Claude Code's config root (settings.json, wrapper script, transcripts).
    pub claude_home: PathBuf,
    pub context: ContextStore,
    pub transcripts: TranscriptCache,
    pub deps: Arc<dyn TelemetryDeps>,
    /// Serialises the hooks and statusline installs (both read-merge-write
    /// settings.json), so concurrent installs cannot lose a merge.
    pub install_lock: Arc<tokio::sync::Mutex<()>>,
}

impl Telemetry {
    pub fn new(claude_home: PathBuf, deps: Arc<dyn TelemetryDeps>) -> Self {
        Self {
            transcripts: TranscriptCache::new(claude_home.clone()),
            claude_home,
            context: ContextStore::default(),
            deps,
            install_lock: Arc::default(),
        }
    }

    pub fn settings_path(&self) -> PathBuf {
        self.claude_home.join("settings.json")
    }

    pub fn wrapper_path(&self) -> PathBuf {
        self.claude_home.join("garage-statusline.sh")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store() -> ContextStore {
        ContextStore::default()
    }

    // JS: statusline.test.js "clampPercentage clamps into [0,100] and rejects non-numbers"
    #[test]
    fn clamp_percentage_clamps_and_rejects_non_numbers() {
        assert_eq!(clamp_percentage(&json!(42)), Some(42.0));
        assert_eq!(clamp_percentage(&json!(150)), Some(100.0));
        assert_eq!(clamp_percentage(&json!(-5)), Some(0.0));
        assert_eq!(clamp_percentage(&json!("42")), None);
        assert_eq!(clamp_percentage(&Value::Null), None);
    }

    #[test]
    fn statusline_context_is_none_for_an_id_that_never_posted() {
        assert_eq!(store().statusline_context("garage/never/heard-of"), None);
    }

    #[test]
    fn set_context_round_trips_with_source_statusline() {
        let s = store();
        s.set_context("a", 42.0);
        assert_eq!(s.statusline_context("a"), Some(json!({"usedPercentage": 42, "source": "statusline"})));
    }

    #[test]
    fn drop_session_clears_stored_context() {
        let s = store();
        s.set_context("a", 88.0);
        s.drop_session("a");
        assert_eq!(s.statusline_context("a"), None);
    }

    #[test]
    fn rate_limits_start_null() {
        assert_eq!(store().rate_limits().to_json(), json!({"fiveHour": null, "sevenDay": null}));
    }

    #[test]
    fn set_rate_limits_stores_both_buckets() {
        let s = store();
        s.set_rate_limits(&json!({
            "five_hour": {"used_percentage": 23.5, "resets_at": "2026-08-30T12:00:00Z"},
            "seven_day": {"used_percentage": 61, "resets_at": "2026-09-01T00:00:00Z"},
        }));
        assert_eq!(
            s.rate_limits().to_json(),
            json!({
                "fiveHour": {"usedPercentage": 23.5, "resetsAt": "2026-08-30T12:00:00Z"},
                "sevenDay": {"usedPercentage": 61, "resetsAt": "2026-09-01T00:00:00Z"},
            })
        );
    }

    #[test]
    fn set_rate_limits_partial_update_preserves_the_untouched_bucket() {
        let s = store();
        s.set_rate_limits(&json!({"five_hour": {"used_percentage": 10, "resets_at": "a"}}));
        s.set_rate_limits(&json!({"seven_day": {"used_percentage": 20, "resets_at": "b"}}));
        let l = s.rate_limits();
        assert_eq!(l.five_hour.unwrap().used_percentage, 10.0);
        assert_eq!(l.seven_day.unwrap().used_percentage, 20.0);
    }

    #[test]
    fn set_rate_limits_ignores_malformed_input() {
        let s = store();
        for bad in [json!(null), json!("not an object"), json!({"five_hour": {"used_percentage": "nope"}})] {
            s.set_rate_limits(&bad);
        }
        assert_eq!(s.rate_limits(), RateLimits::default());
    }

    #[test]
    fn missing_resets_at_becomes_null() {
        let s = store();
        s.set_rate_limits(&json!({"five_hour": {"used_percentage": 5}}));
        assert_eq!(s.rate_limits().five_hour.unwrap().resets_at, Value::Null);
    }
}
