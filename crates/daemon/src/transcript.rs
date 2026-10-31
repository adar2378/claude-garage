//! Port of `daemon/src/transcript.js`: transcript tail reader and context cache.
//!
//! The fallback context percentage for sessions that never posted a
//! statusline: the last assistant `usage` in Claude Code's transcript, over
//! the model's window. A hint, not billing.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::{json, Value};
use tokio::io::{AsyncReadExt, AsyncSeekExt};

const TAIL_BYTES: u64 = 256 * 1024;
const TTL_MS: i64 = 15_000;
const DEFAULT_WINDOW: u64 = 1_000_000;

/// Claude Code's own project-directory naming: the cwd with every `/` and `.`
/// replaced by `-` (leading slash included), so `/Users/x/.garage/wt` becomes
/// `-Users-x--garage-wt`.
pub fn slugify_cwd(cwd: &str) -> String {
    cwd.replace(['/', '.'], "-")
}

pub fn transcript_path(claude_home: &Path, dir: &str, claude_session_id: &str) -> PathBuf {
    claude_home
        .join("projects")
        .join(slugify_cwd(dir))
        .join(format!("{claude_session_id}.jsonl"))
}

/// Model ids known to have a 200k window: Haiku, the 3.x family, and
/// everything before Opus/Sonnet 4.6. Prefix-matched so dated snapshot ids
/// land in the right bucket.
const SMALL_WINDOW_PREFIXES: &[&str] = &[
    "claude-haiku",
    "claude-3",
    "claude-opus-4-0",
    "claude-opus-4-1",
    "claude-opus-4-2", // dated Opus 4 ids: claude-opus-4-20250514
    "claude-opus-4-5",
    "claude-sonnet-4-0",
    "claude-sonnet-4-2", // dated Sonnet 4 ids: claude-sonnet-4-20250514
    "claude-sonnet-4-5",
];

/// `{tokens, model}` of the last assistant usage block.
#[derive(Debug, Clone, PartialEq)]
pub struct Usage {
    pub tokens: u64,
    pub model: Option<String>,
}

/// JS `Number(x) || 0` for the shapes a usage field can take.
fn num(v: Option<&Value>) -> f64 {
    let n = match v {
        Some(Value::Number(n)) => n.as_f64().unwrap_or(0.0),
        Some(Value::String(s)) => s.trim().parse::<f64>().unwrap_or(0.0),
        Some(Value::Bool(true)) => 1.0,
        _ => 0.0,
    };
    if n.is_finite() {
        n
    } else {
        0.0
    }
}

/// Scans `text` backward for the last assistant message carrying a `usage`
/// block, skipping malformed JSON lines and non-assistant/usage entries.
pub fn parse_last_assistant_usage(text: &str) -> Option<Usage> {
    for line in text.lines().rev() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        // A malformed line (or a partial line from the tail cut) is skipped.
        let Ok(entry) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        if entry.get("type").and_then(Value::as_str) != Some("assistant") {
            continue;
        }
        let Some(message) = entry.get("message") else {
            continue;
        };
        let Some(usage) = message.get("usage").filter(|u| u.is_object()) else {
            continue;
        };
        let tokens = num(usage.get("input_tokens"))
            + num(usage.get("cache_read_input_tokens"))
            + num(usage.get("cache_creation_input_tokens"));
        let model = message
            .get("model")
            .and_then(Value::as_str)
            .map(str::to_owned);
        return Some(Usage {
            tokens: tokens as u64,
            model,
        });
    }
    None
}

/// Reads only the last <=256KB (a multi-MB transcript must never cost a full
/// read). `None` if the file does not exist or cannot be read.
async fn read_tail(path: &Path) -> Option<String> {
    let mut file = tokio::fs::File::open(path).await.ok()?;
    let size = file.metadata().await.ok()?.len();
    let start = size.saturating_sub(TAIL_BYTES);
    file.seek(std::io::SeekFrom::Start(start)).await.ok()?;
    let mut buf = Vec::with_capacity((size - start) as usize);
    file.take(size - start).read_to_end(&mut buf).await.ok()?;
    Some(String::from_utf8_lossy(&buf).into_owned())
}

#[derive(Debug, Clone, PartialEq)]
pub struct CachedContext {
    pub used_percentage: u32,
}

impl CachedContext {
    /// `{usedPercentage, source: "transcript"}`
    pub fn to_json(&self) -> Value {
        json!({"usedPercentage": self.used_percentage, "source": "transcript"})
    }
}

#[derive(Debug, Clone, Default)]
struct Entry {
    value: Option<CachedContext>,
    at: i64,
    pending: bool,
}

type Clock = Arc<dyn Fn() -> i64 + Send + Sync>;

fn system_now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// Learned model windows plus the per-session context cache. Cheap to clone.
#[derive(Clone)]
pub struct TranscriptCache {
    claude_home: PathBuf,
    clock: Clock,
    windows: Arc<Mutex<HashMap<String, u64>>>,
    cache: Arc<Mutex<HashMap<String, Entry>>>,
}

impl TranscriptCache {
    pub fn new(claude_home: PathBuf) -> Self {
        Self::with_clock(claude_home, Arc::new(system_now_ms))
    }

    /// A settable clock lets tests observe the TTL without waiting.
    pub fn with_clock(claude_home: PathBuf, clock: Clock) -> Self {
        Self {
            claude_home,
            clock,
            windows: Default::default(),
            cache: Default::default(),
        }
    }

    /// Teach a model id's real window, fed by statusline posts (Claude Code's
    /// own `context_window_size`): the only authoritative local source.
    pub fn record_model_window(&self, model: Option<&str>, size: Option<f64>) {
        let (Some(model), Some(size)) = (model.filter(|m| !m.is_empty()), size) else {
            return;
        };
        if !size.is_finite() || size < 1000.0 {
            return;
        }
        self.windows
            .lock()
            .unwrap()
            .insert(model.to_owned(), size as u64);
    }

    /// Precedence: a learned window, then the `[1m]` marker, then the known
    /// 200k list, then the 1M default.
    pub fn window_for_model(&self, model: Option<&str>) -> u64 {
        let Some(model) = model else {
            return DEFAULT_WINDOW;
        };
        if let Some(w) = self.windows.lock().unwrap().get(model) {
            return *w;
        }
        if model.contains("[1m]") {
            return 1_000_000;
        }
        if SMALL_WINDOW_PREFIXES.iter().any(|p| model.starts_with(p)) {
            return 200_000;
        }
        DEFAULT_WINDOW
    }

    /// Real file IO; the hot path must use `cached_context` / `refresh_context`.
    pub async fn compute_context(
        &self,
        dir: &str,
        claude_session_id: &str,
    ) -> Option<CachedContext> {
        if dir.is_empty() || claude_session_id.is_empty() {
            return None;
        }
        let tail = read_tail(&transcript_path(&self.claude_home, dir, claude_session_id)).await?;
        let usage = parse_last_assistant_usage(&tail)?;
        let window = self.window_for_model(usage.model.as_deref()) as f64;
        let pct = (usage.tokens as f64 / window * 100.0)
            .round()
            .clamp(0.0, 100.0);
        Some(CachedContext {
            used_percentage: pct as u32,
        })
    }

    pub fn cached_context(&self, id: &str) -> Option<CachedContext> {
        self.cache.lock().unwrap().get(id)?.value.clone()
    }

    /// `(value, at)` for tests to observe the TTL.
    pub fn cache_entry(&self, id: &str) -> (Option<CachedContext>, Option<i64>) {
        match self.cache.lock().unwrap().get(id) {
            Some(e) => (e.value.clone(), Some(e.at)),
            None => (None, None),
        }
    }

    pub fn clear_cache(&self, id: &str) {
        self.cache.lock().unwrap().remove(id);
    }

    /// Kicks a background read when the entry is stale and none is in flight;
    /// returns immediately in every case so it can never block a request.
    /// Must be called inside a tokio runtime.
    pub fn refresh_context(&self, id: &str, dir: &str, claude_session_id: &str) {
        {
            let mut cache = self.cache.lock().unwrap();
            let entry = cache.get(id);
            if let Some(e) = entry {
                if e.pending || (self.clock)() - e.at < TTL_MS {
                    return;
                }
            }
            let (value, at) = entry.map(|e| (e.value.clone(), e.at)).unwrap_or((None, 0));
            cache.insert(
                id.to_owned(),
                Entry {
                    value,
                    at,
                    pending: true,
                },
            );
        }
        let this = self.clone();
        let (id, dir, csid) = (id.to_owned(), dir.to_owned(), claude_session_id.to_owned());
        tokio::spawn(async move {
            // A missing/unreadable transcript caches null, as in JS.
            let value = this.compute_context(&dir, &csid).await;
            let at = (this.clock)();
            this.cache.lock().unwrap().insert(
                id,
                Entry {
                    value,
                    at,
                    pending: false,
                },
            );
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicI64, Ordering};
    use std::time::Duration;

    // JS: transcript.test.js (one Rust test per JS test; mapping in TESTS.md).

    fn cache() -> (tempfile::TempDir, TranscriptCache) {
        let dir = tempfile::tempdir().unwrap();
        let c = TranscriptCache::new(dir.path().to_path_buf());
        (dir, c)
    }

    fn assistant_line(input: u64, cache_read: u64, cache_creation: u64, model: &str) -> String {
        json!({"type": "assistant", "message": {"model": model, "usage": {
            "input_tokens": input,
            "cache_read_input_tokens": cache_read,
            "cache_creation_input_tokens": cache_creation,
        }}})
        .to_string()
    }

    fn write_transcript(home: &Path, dir: &str, csid: &str, content: &str) {
        let p = transcript_path(home, dir, csid);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, content).unwrap();
    }

    async fn settle() {
        tokio::time::sleep(Duration::from_millis(50)).await;
    }

    #[test]
    fn slugify_cwd_replaces_every_slash() {
        assert_eq!(slugify_cwd("/Users/me/repo"), "-Users-me-repo");
        assert_eq!(slugify_cwd("/a/b/c"), "-a-b-c");
    }

    #[test]
    fn slugify_cwd_maps_dots_to_dashes_too() {
        assert_eq!(slugify_cwd("/Users/x/.garage/wt"), "-Users-x--garage-wt");
        assert_eq!(slugify_cwd("/a/b.c/d"), "-a-b-c-d");
    }

    #[test]
    fn unknown_or_missing_model_defaults_to_1m() {
        let (_d, c) = cache();
        for m in ["claude-opus-5", "claude-fable-5", "claude-sonnet-4-6"] {
            assert_eq!(c.window_for_model(Some(m)), 1_000_000, "{m}");
        }
        assert_eq!(c.window_for_model(None), 1_000_000);
    }

    #[test]
    fn known_small_window_models_get_200k() {
        let (_d, c) = cache();
        for m in [
            "claude-haiku-4-5-20251001",
            "claude-sonnet-4-5-20250929",
            "claude-opus-4-5-20251101",
            "claude-sonnet-4-20250514",
            "claude-3-5-sonnet-20241022",
        ] {
            assert_eq!(c.window_for_model(Some(m)), 200_000, "{m}");
        }
    }

    #[test]
    fn a_1m_marker_model_id_gets_1m() {
        let (_d, c) = cache();
        assert_eq!(
            c.window_for_model(Some("claude-sonnet-4-5-20250929[1m]")),
            1_000_000
        );
    }

    #[test]
    fn a_learned_window_wins_over_every_heuristic() {
        let (_d, c) = cache();
        c.record_model_window(Some("claude-sonnet-4-5-20250929"), Some(1_000_000.0));
        assert_eq!(
            c.window_for_model(Some("claude-sonnet-4-5-20250929")),
            1_000_000
        );
        c.record_model_window(Some("claude-window-test"), Some(400_000.0));
        assert_eq!(c.window_for_model(Some("claude-window-test")), 400_000);
    }

    #[test]
    fn record_model_window_ignores_junk() {
        let (_d, c) = cache();
        c.record_model_window(None, Some(500_000.0));
        c.record_model_window(Some("claude-junk"), Some(f64::NAN));
        c.record_model_window(Some("claude-junk"), Some(0.0));
        c.record_model_window(Some("claude-junk"), None);
        assert_eq!(c.window_for_model(Some("claude-junk")), 1_000_000);
    }

    #[test]
    fn parse_last_assistant_usage_finds_the_last_skipping_malformed_and_non_assistant() {
        let text = [
            assistant_line(10, 0, 0, "claude-x"),
            "{ this is not valid json".into(),
            json!({"type": "system", "subtype": "noise"}).to_string(),
            assistant_line(5, 100, 20, "claude-y"),
        ]
        .join("\n");
        assert_eq!(
            parse_last_assistant_usage(&text),
            Some(Usage {
                tokens: 125,
                model: Some("claude-y".into())
            })
        );
    }

    #[test]
    fn parse_last_assistant_usage_returns_none_when_nothing_usable() {
        let text = [
            "not json at all".to_string(),
            json!({"type": "system"}).to_string(),
        ]
        .join("\n");
        assert_eq!(parse_last_assistant_usage(&text), None);
    }

    #[tokio::test]
    async fn compute_context_percentage_is_tokens_over_window() {
        let (d, c) = cache();
        let content = [
            json!({"type": "system"}).to_string(),
            assistant_line(50_000, 50_000, 0, "claude-haiku-4-5"),
        ]
        .join("\n");
        write_transcript(d.path(), "/scratch/proj-a", "session-a", &content);
        assert_eq!(
            c.compute_context("/scratch/proj-a", "session-a").await,
            Some(CachedContext {
                used_percentage: 50
            })
        );
    }

    #[tokio::test]
    async fn compute_context_missing_file_or_ids_is_none() {
        let (_d, c) = cache();
        assert_eq!(c.compute_context("/nope", "no-such-id").await, None);
        assert_eq!(c.compute_context("", "x").await, None);
        assert_eq!(c.compute_context("/x", "").await, None);
    }

    #[tokio::test]
    async fn compute_context_skips_malformed_trailing_lines() {
        let (d, c) = cache();
        let content = [
            assistant_line(20_000, 0, 0, "claude-haiku-4-5"),
            "{{{ garbage, not json".into(),
            "".into(),
        ]
        .join("\n");
        write_transcript(d.path(), "/scratch/proj-malformed", "s", &content);
        assert_eq!(
            c.compute_context("/scratch/proj-malformed", "s")
                .await
                .unwrap()
                .used_percentage,
            10
        );
    }

    #[tokio::test]
    async fn compute_context_tail_bounded_read_finds_usage_in_a_multi_mb_file() {
        let (d, c) = cache();
        let pad = json!({"type": "user", "junk": "x".repeat(1024)}).to_string();
        let padding = vec![pad; 4000].join("\n"); // ~4MB
        let content = format!(
            "{padding}\n{}\n",
            assistant_line(30_000, 0, 0, "claude-haiku-4-5")
        );
        write_transcript(d.path(), "/scratch/proj-big", "s", &content);
        assert_eq!(
            c.compute_context("/scratch/proj-big", "s")
                .await
                .unwrap()
                .used_percentage,
            15
        );
    }

    #[tokio::test]
    async fn compute_context_a_usage_line_outside_the_tail_is_not_found() {
        let (d, c) = cache();
        let pad = json!({"type": "user", "junk": "x".repeat(1024)}).to_string();
        let trailing = vec![pad; 400].join("\n"); // ~400KB after the target
        let content = format!(
            "{}\n{trailing}\n",
            assistant_line(199_999, 0, 0, "claude-x")
        );
        write_transcript(d.path(), "/scratch/proj-outside-tail", "s", &content);
        assert_eq!(
            c.compute_context("/scratch/proj-outside-tail", "s").await,
            None
        );
    }

    #[test]
    fn cached_context_is_none_before_any_refresh() {
        let (_d, c) = cache();
        assert_eq!(c.cached_context("cache-never-refreshed"), None);
    }

    #[tokio::test]
    async fn refresh_context_populates_the_cache_without_blocking_the_caller() {
        let (d, c) = cache();
        write_transcript(
            d.path(),
            "/scratch/proj-cache",
            "s",
            &assistant_line(100_000, 0, 0, "claude-haiku-4-5"),
        );
        c.refresh_context("id", "/scratch/proj-cache", "s");
        assert_eq!(
            c.cached_context("id"),
            None,
            "the read has not completed yet"
        );
        settle().await;
        assert_eq!(
            c.cached_context("id"),
            Some(CachedContext {
                used_percentage: 50
            })
        );
        assert_eq!(
            c.cached_context("id").unwrap().to_json(),
            json!({"usedPercentage": 50, "source": "transcript"})
        );
    }

    #[tokio::test]
    async fn refresh_context_respects_the_ttl_and_refreshes_once_stale() {
        let dir = tempfile::tempdir().unwrap();
        let now = Arc::new(AtomicI64::new(1_000_000));
        let clock = {
            let n = now.clone();
            Arc::new(move || n.load(Ordering::SeqCst))
        };
        let c = TranscriptCache::with_clock(dir.path().to_path_buf(), clock);
        write_transcript(
            dir.path(),
            "/scratch/proj-ttl",
            "s",
            &assistant_line(100_000, 0, 0, "claude-haiku-4-5"),
        );

        c.refresh_context("id", "/scratch/proj-ttl", "s");
        settle().await;
        let first_at = c.cache_entry("id").1.unwrap();

        now.fetch_add(5_000, Ordering::SeqCst); // within the 15s TTL
        c.refresh_context("id", "/scratch/proj-ttl", "s");
        settle().await;
        assert_eq!(c.cache_entry("id").1.unwrap(), first_at, "no refresh yet");

        now.fetch_add(11_000, Ordering::SeqCst); // 16s total: stale
        c.refresh_context("id", "/scratch/proj-ttl", "s");
        settle().await;
        assert_ne!(
            c.cache_entry("id").1.unwrap(),
            first_at,
            "a stale entry must refresh"
        );
    }

    #[tokio::test]
    async fn clear_cache_drops_a_cached_entry() {
        let (d, c) = cache();
        write_transcript(
            d.path(),
            "/scratch/proj-clear",
            "s",
            &assistant_line(100_000, 0, 0, "claude-x"),
        );
        c.refresh_context("id", "/scratch/proj-clear", "s");
        settle().await;
        assert!(c.cached_context("id").is_some());
        c.clear_cache("id");
        assert_eq!(c.cached_context("id"), None);
    }
}
