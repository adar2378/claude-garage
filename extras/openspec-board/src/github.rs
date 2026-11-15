//! Pull requests from GitHub through `gh api`, read-only.
//!
//! Every call is `gh api -i --method GET <endpoint>`: nothing is ever
//! written to GitHub. The outcome of a call is decided from the HTTP **status
//! line** that `-i` prints, never from the exit code (`gh api` exits with 1
//! on a `304`); the exit code and stderr are looked at only when there is no
//! status line, i.e. when the request never reached GitHub. Bodies and ETags
//! are cached in `cache/` (private: PR titles and comment bodies) and
//! revalidated with `If-None-Match`. The only data sent are the repo slug
//! and the branch names GitHub already has.

use std::collections::HashMap;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::config::{Patterns, write_cache};

/// A `gh` call running longer than this is killed.
pub const TIMEOUT: Duration = Duration::from_secs(15);
/// Time between two GitHub passes while the pane is open.
pub const POLL: Duration = Duration::from_secs(60);
/// Below this many calls left, the poll interval doubles.
pub const LOW_WATER: u64 = 200;
/// Merged pull requests newer than this many days are shown.
pub const MERGED_DAYS: i64 = 14;
/// Comment pages read per PR, from the newest backwards.
const MAX_COMMENT_PAGES: u32 = 5;
/// Pages walked forward to find the last one.
const MAX_PAGES: u32 = 20;
/// The only environment the child sees: `gh` works through its stored login.
const KEEP_ENV: [&str; 7] = ["PATH", "HOME", "USER", "LANG", "TERM", "TMPDIR", "SHELL"];
/// Output of one call is read up to this size.
const MAX_OUTPUT: u64 = 16 * 1024 * 1024;

/// What one `gh api -i` run produced.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Reply {
    /// There was a status line.
    Http {
        status: u16,
        /// Lower-case names.
        headers: Vec<(String, String)>,
        body: String,
    },
    /// No status line: the request did not reach GitHub.
    NoStatus {
        exit: Option<i32>,
        stderr: String,
    },
    /// `gh` could not be started (not installed).
    Missing,
    TimedOut,
}

/// Status, lower-case headers and body of one response.
pub type Parsed = (u16, Vec<(String, String)>, String);

/// Split `gh api -i` output into status, headers and body. `None` when the
/// first line is not an HTTP status line.
pub fn parse_http(out: &str) -> Option<Parsed> {
    let (head, body) = match out.find("\r\n\r\n") {
        Some(i) => (&out[..i], &out[i + 4..]),
        None => match out.find("\n\n") {
            Some(i) => (&out[..i], &out[i + 2..]),
            None => (out, ""),
        },
    };
    let mut lines = head.lines();
    let first = lines.next()?;
    let mut parts = first.split_whitespace();
    if !parts.next()?.starts_with("HTTP/") {
        return None;
    }
    let status: u16 = parts.next()?.parse().ok()?;
    let headers = lines
        .filter_map(|l| l.split_once(':'))
        .map(|(k, v)| (k.trim().to_ascii_lowercase(), v.trim().to_owned()))
        .collect();
    Some((status, headers, body.to_owned()))
}

fn header<'a>(headers: &'a [(String, String)], name: &str) -> Option<&'a str> {
    headers
        .iter()
        .find(|(k, _)| k == name)
        .map(|(_, v)| v.as_str())
}

/// Runs `gh api`, GET only.
#[derive(Debug, Clone)]
pub struct Runner {
    program: PathBuf,
    timeout: Duration,
}

impl Runner {
    pub fn new(program: impl Into<PathBuf>, timeout: Duration) -> Self {
        Runner {
            program: program.into(),
            timeout,
        }
    }

    /// `gh api -i --method GET [-H If-None-Match: <etag>] <endpoint>`, with
    /// the environment cleared down to [`KEEP_ENV`], stdin null and a timeout.
    pub fn get(&self, endpoint: &str, etag: Option<&str>) -> Reply {
        let mut cmd = Command::new(&self.program);
        cmd.args(["api", "-i", "--method", "GET"]);
        if let Some(e) = etag {
            cmd.arg("-H").arg(format!("If-None-Match: {e}"));
        }
        cmd.arg(endpoint).env_clear();
        for k in KEEP_ENV {
            if let Some(v) = std::env::var_os(k) {
                cmd.env(k, v);
            }
        }
        let mut child = match cmd
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
        {
            Ok(c) => c,
            Err(_) => return Reply::Missing,
        };
        let (out, err) = (drain(child.stdout.take()), drain(child.stderr.take()));
        let start = Instant::now();
        let status = loop {
            match child.try_wait() {
                Ok(Some(s)) => break s,
                Ok(None) if start.elapsed() >= self.timeout => {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Reply::TimedOut;
                }
                Ok(None) => std::thread::sleep(Duration::from_millis(10)),
                Err(_) => return Reply::Missing,
            }
        };
        let (out, err) = (
            out.join().unwrap_or_default(),
            err.join().unwrap_or_default(),
        );
        match parse_http(&out) {
            Some((status, headers, body)) => Reply::Http {
                status,
                headers,
                body,
            },
            None => Reply::NoStatus {
                exit: status.code(),
                stderr: err,
            },
        }
    }
}

fn drain<R: Read + Send + 'static>(s: Option<R>) -> std::thread::JoinHandle<String> {
    std::thread::spawn(move || {
        let mut buf = Vec::new();
        if let Some(s) = s {
            let _ = s.take(MAX_OUTPUT).read_to_end(&mut buf);
        }
        String::from_utf8_lossy(&buf).into_owned()
    })
}

/// What is known of the rate limit, from the headers of every response.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Rate {
    pub remaining: Option<u64>,
    /// Unix seconds.
    pub reset: Option<i64>,
}

impl Rate {
    fn record(&mut self, headers: &[(String, String)]) {
        if let Some(r) = header(headers, "x-ratelimit-remaining").and_then(|v| v.parse().ok()) {
            self.remaining = Some(r);
        }
        if let Some(r) = header(headers, "x-ratelimit-reset").and_then(|v| v.parse().ok()) {
            self.reset = Some(r);
        }
    }

    /// Until when no call may be made: the reset time, when the limit is used
    /// up and the reset is still ahead.
    pub fn blocked_until(&self, now_unix: i64) -> Option<i64> {
        match (self.remaining, self.reset) {
            (Some(0), Some(reset)) if reset > now_unix => Some(reset),
            _ => None,
        }
    }

    /// The poll interval: doubled below [`LOW_WATER`] calls left.
    pub fn interval(&self, base: Duration) -> Duration {
        if self.remaining.is_some_and(|r| r < LOW_WATER) {
            base * 2
        } else {
            base
        }
    }
}

/// How the last attempt to reach GitHub went.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    /// Not asked yet, or GitHub is off for the workspace.
    Off,
    Ok,
    /// `gh` is not installed.
    Missing,
    NotLoggedIn,
    Offline {
        since: Instant,
    },
    /// Unix seconds of the reset.
    RateLimited {
        until: i64,
    },
    /// Another HTTP status (404, 5xx, ...).
    Http(u16),
}

fn hhmm(unix: i64) -> String {
    let s = unix.rem_euclid(86_400);
    format!("{:02}:{:02} UTC", s / 3600, s % 3600 / 60)
}

impl Status {
    /// The short chip for the pane's title.
    pub fn chip(&self, now: Instant) -> String {
        match self {
            Status::Off | Status::Missing => "gh off".to_owned(),
            Status::Ok => "gh ok".to_owned(),
            Status::NotLoggedIn => "gh not logged in (gh auth login)".to_owned(),
            Status::Offline { since } => format!(
                "gh offline {}",
                crate::pane::age(now.saturating_duration_since(*since).as_secs())
            ),
            Status::RateLimited { until } => format!("rate limited until {}", hhmm(*until)),
            Status::Http(c) => format!("gh error {c}"),
        }
    }

    /// A warning row for the states the user has to act on.
    pub fn warning(&self) -> Option<String> {
        match self {
            Status::Missing => Some("gh was not found on PATH: pull requests are not shown".into()),
            Status::NotLoggedIn => Some(
                "gh is not logged in: run `gh auth login` (GH_TOKEN in your shell is not used)"
                    .into(),
            ),
            _ => None,
        }
    }
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct Entry {
    etag: Option<String>,
    body: String,
}

/// A body from `gh`, or from the cache when GitHub said `304` or could not
/// be reached.
#[derive(Debug, Clone)]
struct Body {
    text: String,
}

/// One file of a PR.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrFile {
    pub path: String,
    pub status: String,
}

/// A CI result from a comment (or from the native check runs).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CiResult {
    /// `PASS`, `FAIL`, ...
    pub status: String,
    pub sha: String,
    pub mode: Option<String>,
    /// Names the PR's current head commit.
    pub current: bool,
}

/// A review verdict from a comment.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReviewResult {
    pub verdict: String,
    pub sha: String,
    pub current: bool,
}

/// What the comments of one PR say.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Checks {
    pub ci: Option<CiResult>,
    pub review: Option<ReviewResult>,
    /// The newest review comment is current and its whole body matches
    /// `owner_needed`.
    pub owner_needed: bool,
}

/// One pull request as read, before it is linked to a change.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrData {
    pub number: u64,
    pub url: String,
    pub title: String,
    pub draft: bool,
    pub head_ref: String,
    pub head_sha: String,
    pub updated_at: String,
    pub merged: bool,
    pub files: Vec<PrFile>,
    pub checks: Checks,
}

/// `Some(sha)` is current when it equals or is a prefix of `head`.
pub fn is_current(sha: &str, head: &str) -> bool {
    !sha.is_empty()
        && head
            .to_ascii_lowercase()
            .starts_with(&sha.to_ascii_lowercase())
}

fn re(p: &Option<String>) -> Option<Regex> {
    p.as_deref().and_then(|p| Regex::new(p).ok())
}

/// The newest match of each pattern in `newest_first` comment bodies. `head`
/// is the PR's head sha. `owner_needed` is matched against the whole body of
/// the newest comment matching `review`.
pub fn scan_comments(newest_first: &[String], patterns: &Patterns, head: &str) -> Checks {
    let (ci_re, review_re, owner_re) = (
        re(&patterns.ci),
        re(&patterns.review),
        re(&patterns.owner_needed),
    );
    let mut out = Checks::default();
    for body in newest_first {
        if out.ci.is_none()
            && let Some(c) = ci_re.as_ref().and_then(|r| r.captures(body))
            && let Some(status) = c.name("status")
        {
            let sha = c.name("sha").map_or("", |m| m.as_str()).to_owned();
            out.ci = Some(CiResult {
                status: status.as_str().to_owned(),
                current: is_current(&sha, head),
                sha,
                mode: c.name("mode").map(|m| m.as_str().to_owned()),
            });
        }
        if out.review.is_none()
            && let Some(c) = review_re.as_ref().and_then(|r| r.captures(body))
            && let Some(verdict) = c.name("verdict")
        {
            let sha = c.name("sha").map_or("", |m| m.as_str()).to_owned();
            let current = is_current(&sha, head);
            out.owner_needed = current && owner_re.as_ref().is_some_and(|r| r.is_match(body));
            out.review = Some(ReviewResult {
                verdict: verdict.as_str().trim().to_owned(),
                current,
                sha,
            });
        }
        if (ci_re.is_none() || out.ci.is_some()) && (review_re.is_none() || out.review.is_some()) {
            break;
        }
    }
    out
}

/// Days since 1970-01-01 of a civil date.
fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d - 1;
    era * 146_097 + yoe * 365 + yoe / 4 - yoe / 100 + doy - 719_468
}

/// `2026-11-02T10:11:12Z` to unix seconds.
pub fn parse_iso(s: &str) -> Option<i64> {
    let n = |r: std::ops::Range<usize>| s.get(r)?.parse::<i64>().ok();
    let (y, mo, d, h, mi, se) = (
        n(0..4)?,
        n(5..7)?,
        n(8..10)?,
        n(11..13)?,
        n(14..16)?,
        n(17..19)?,
    );
    Some(days_from_civil(y, mo, d) * 86_400 + h * 3600 + mi * 60 + se)
}

/// `owner/repo` from a github.com remote URL (https, ssh, scp form); `None`
/// for any other host.
pub fn slug_of(url: &str) -> Option<String> {
    let url = url.trim();
    let rest = ["https://", "http://", "ssh://", "git://"]
        .iter()
        .find_map(|p| url.strip_prefix(p))
        .map(|r| r.split_once('@').map_or(r, |(_, h)| h))
        .map(|r| r.strip_prefix("github.com/").map(str::to_owned))
        .unwrap_or_else(|| {
            // scp form: git@github.com:owner/repo.git
            let r = url.split_once('@').map_or(url, |(_, h)| h);
            r.strip_prefix("github.com:").map(str::to_owned)
        })?;
    let rest = rest.trim_end_matches('/');
    let rest = rest.strip_suffix(".git").unwrap_or(rest);
    let mut it = rest.split('/');
    let (owner, repo, extra) = (it.next()?, it.next()?, it.next());
    let ok = |s: &str| {
        !s.is_empty()
            && s.chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
    };
    (extra.is_none() && ok(owner) && ok(repo)).then(|| format!("{owner}/{repo}"))
}

/// The GitHub slug of a repo's `origin`.
pub fn repo_slug(repo: &Path) -> Option<String> {
    slug_of(&crate::local::git(repo, &["remote", "get-url", "origin"])?)
}

fn cache_name(endpoint: &str) -> String {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in endpoint.bytes() {
        h = (h ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3);
    }
    format!("gh-{h:016x}.json")
}

/// The GitHub reader: runner, rate limit, status and what was read.
pub struct Github {
    runner: Runner,
    state_dir: PathBuf,
    pub rate: Rate,
    pub status: Status,
    now_unix: i64,
    /// A call already failed in this pass: no more calls until the next one.
    pass_failed: bool,
    last_poll: Option<Instant>,
    files: HashMap<(String, u64, String), Vec<PrFile>>,
    comments: HashMap<(String, u64, String), Checks>,
    /// The PRs of each repo slug, as last read.
    pub data: HashMap<String, Vec<PrData>>,
}

impl Github {
    pub fn new(program: impl Into<PathBuf>, state_dir: PathBuf) -> Self {
        Github {
            runner: Runner::new(program, TIMEOUT),
            state_dir,
            rate: Rate::default(),
            status: Status::Off,
            now_unix: 0,
            pass_failed: false,
            last_poll: None,
            files: HashMap::new(),
            comments: HashMap::new(),
            data: HashMap::new(),
        }
    }

    /// Whether a GitHub pass is due: never polled, the interval (doubled when
    /// the limit is low) passed, or the user forced it.
    pub fn due(&self, force: bool, now: Instant) -> bool {
        force
            || self
                .last_poll
                .is_none_or(|t| now.saturating_duration_since(t) >= self.rate.interval(POLL))
    }

    /// Start a pass: the calls of one pass stop at the first failure.
    pub fn begin_pass(&mut self, now: Instant, now_unix: i64) {
        self.last_poll = Some(now);
        self.now_unix = now_unix;
        self.pass_failed = false;
    }

    fn cached(&self, endpoint: &str) -> Option<Entry> {
        let bytes = std::fs::read(self.state_dir.join("cache").join(cache_name(endpoint))).ok()?;
        serde_json::from_slice(&bytes).ok()
    }

    fn store(&self, endpoint: &str, entry: &Entry) {
        if let Ok(bytes) = serde_json::to_vec(entry) {
            let _ = write_cache(&self.state_dir, &cache_name(endpoint), &bytes);
        }
    }

    /// GET `endpoint`: a fresh body on `200`, the cached one on `304`, and
    /// the cached one (or `None`) whenever GitHub cannot be reached.
    fn request(&mut self, endpoint: &str) -> Option<Body> {
        let cached = self.cached(endpoint);
        let serve = |c: Option<Entry>| c.map(|e| Body { text: e.body });
        if let Some(until) = self.rate.blocked_until(self.now_unix) {
            self.status = Status::RateLimited { until };
            return serve(cached);
        }
        if self.pass_failed {
            return serve(cached);
        }
        let etag = cached.as_ref().and_then(|c| c.etag.clone());
        match self.runner.get(endpoint, etag.as_deref()) {
            Reply::Http {
                status,
                headers,
                body,
            } => {
                self.rate.record(&headers);
                match status {
                    200 => {
                        self.status = Status::Ok;
                        let entry = Entry {
                            etag: header(&headers, "etag").map(str::to_owned),
                            body,
                        };
                        self.store(endpoint, &entry);
                        serve(Some(entry))
                    }
                    304 => {
                        self.status = Status::Ok;
                        serve(cached)
                    }
                    401 => self.fail(Status::NotLoggedIn, cached),
                    403 | 429
                        if self.rate.remaining == Some(0)
                            || status == 429
                            || body.to_ascii_lowercase().contains("rate limit") =>
                    {
                        let until = self.rate.reset.unwrap_or(self.now_unix + 60);
                        self.fail(Status::RateLimited { until }, cached)
                    }
                    other => self.fail(Status::Http(other), cached),
                }
            }
            Reply::Missing => self.fail(Status::Missing, cached),
            Reply::TimedOut => self.fail(self.offline(), cached),
            Reply::NoStatus { stderr, .. } => {
                let s = stderr.to_ascii_lowercase();
                if s.contains("gh auth login") || s.contains("authentication") {
                    self.fail(Status::NotLoggedIn, cached)
                } else {
                    self.fail(self.offline(), cached)
                }
            }
        }
    }

    fn offline(&self) -> Status {
        match self.status {
            Status::Offline { since } => Status::Offline { since },
            _ => Status::Offline {
                since: Instant::now(),
            },
        }
    }

    fn fail(&mut self, status: Status, cached: Option<Entry>) -> Option<Body> {
        self.status = status;
        self.pass_failed = true;
        cached.map(|e| Body { text: e.body })
    }

    fn json(&mut self, endpoint: &str) -> Option<Value> {
        serde_json::from_str(&self.request(endpoint)?.text).ok()
    }

    /// Read the open and recently merged PRs of `slug`, their changed files
    /// (again only when the head moved) and, for the PRs that link to one of
    /// `names`, their comments (again only when `updated_at` moved). The
    /// result is kept in [`Github::data`]; when GitHub gave no list at all
    /// the previous one stays.
    pub fn read_repo(&mut self, slug: &str, names: &[String], patterns: &Patterns, checks: bool) {
        let open = self.json(&format!("repos/{slug}/pulls?state=open&per_page=100"));
        let closed = self.json(&format!(
            "repos/{slug}/pulls?state=closed&sort=updated&direction=desc&per_page=100"
        ));
        let (Some(open), Some(closed)) = (open, closed) else {
            return;
        };
        let mut prs: Vec<PrData> = vec![];
        for v in open.as_array().into_iter().flatten() {
            if let Some(mut pr) = pr_of(v, false) {
                pr.files = self.pr_files(slug, &pr);
                prs.push(pr);
            }
        }
        for v in closed.as_array().into_iter().flatten() {
            let merged = v["merged_at"].as_str().and_then(parse_iso);
            if let Some(at) = merged
                && self.now_unix - at <= MERGED_DAYS * 86_400
                && let Some(pr) = pr_of(v, true)
            {
                prs.push(pr);
            }
        }
        for pr in prs.iter_mut().filter(|p| !p.merged) {
            let linked = crate::linking::link_change(
                names,
                &pr.head_ref,
                &pr.title,
                pr.files.iter().map(|f| f.path.as_str()),
            )
            .is_some();
            let wants_comments = patterns.ci.is_some() || patterns.review.is_some();
            if !linked || (!wants_comments && !checks) {
                continue;
            }
            pr.checks = self.pr_checks(slug, pr, patterns, checks);
        }
        let live: Vec<u64> = prs.iter().map(|p| p.number).collect();
        self.files
            .retain(|(s, n, _), _| s != slug || live.contains(n));
        self.comments
            .retain(|(s, n, _), _| s != slug || live.contains(n));
        self.data.insert(slug.to_owned(), prs);
    }

    fn pr_files(&mut self, slug: &str, pr: &PrData) -> Vec<PrFile> {
        let key = (slug.to_owned(), pr.number, pr.head_sha.clone());
        if let Some(f) = self.files.get(&key) {
            return f.clone();
        }
        let endpoint = format!("repos/{slug}/pulls/{}/files?per_page=100", pr.number);
        let Some(v) = self.json(&endpoint) else {
            return vec![];
        };
        let files: Vec<PrFile> = v
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|f| {
                Some(PrFile {
                    path: f["filename"].as_str()?.to_owned(),
                    status: f["status"].as_str().unwrap_or("").to_owned(),
                })
            })
            .collect();
        // A failed read is not remembered: it is tried again next pass.
        if self.status == Status::Ok {
            self.files.insert(key, files.clone());
        }
        files
    }

    fn pr_checks(&mut self, slug: &str, pr: &PrData, patterns: &Patterns, native: bool) -> Checks {
        let key = (
            slug.to_owned(),
            pr.number,
            format!("{}|{}", pr.updated_at, pr.head_sha),
        );
        if let Some(c) = self.comments.get(&key) {
            return c.clone();
        }
        let mut found = if patterns.ci.is_some() || patterns.review.is_some() {
            self.scan_pr_comments(slug, pr, patterns)
        } else {
            Checks::default()
        };
        if native && patterns.ci.is_none() {
            found.ci = self.native_ci(slug, pr);
        }
        // Check runs finishing does not move `updated_at`: a native result
        // that is not final is read again next pass.
        let settled = !(native && patterns.ci.is_none()) || found.ci.is_some();
        if self.status == Status::Ok && settled {
            self.comments.insert(key, found.clone());
        }
        found
    }

    /// Read the comments from the newest page backwards and stop at the
    /// newest match of each pattern.
    fn scan_pr_comments(&mut self, slug: &str, pr: &PrData, patterns: &Patterns) -> Checks {
        let base = format!("repos/{slug}/issues/{}/comments?per_page=100", pr.number);
        // The page count is found by walking forward while pages are full
        // (a 304 costs nothing), not from a cached `Link` header that goes
        // stale when a page is added.
        let mut pages: Vec<String> = vec![];
        for page in 1..=MAX_PAGES {
            let endpoint = if page == 1 {
                base.clone()
            } else {
                format!("{base}&page={page}")
            };
            let Some(body) = self.request(&endpoint) else {
                break;
            };
            let full = serde_json::from_str::<Vec<Value>>(&body.text).is_ok_and(|l| l.len() >= 100);
            pages.push(body.text);
            if !full {
                break;
            }
        }
        if pages.is_empty() {
            return Checks::default();
        }
        let last = pages.len() as u32;
        let mut newest_first: Vec<String> = vec![];
        let mut page = last;
        let stop_at = last.saturating_sub(MAX_COMMENT_PAGES - 1).max(1);
        loop {
            let list: Vec<Value> =
                serde_json::from_str(&pages[page as usize - 1]).unwrap_or_default();
            newest_first.extend(
                list.iter()
                    .rev()
                    .filter_map(|c| c["body"].as_str().map(str::to_owned)),
            );
            let found = scan_comments(&newest_first, patterns, &pr.head_sha);
            let done = (patterns.ci.is_none() || found.ci.is_some())
                && (patterns.review.is_none() || found.review.is_some());
            if done || page <= stop_at {
                return found;
            }
            page -= 1;
        }
    }

    fn native_ci(&mut self, slug: &str, pr: &PrData) -> Option<CiResult> {
        let v = self.json(&format!(
            "repos/{slug}/commits/{}/check-runs?per_page=100",
            pr.head_sha
        ))?;
        let runs = v["check_runs"].as_array()?;
        if runs.is_empty() {
            return None;
        }
        let failed = runs.iter().any(|r| {
            matches!(
                r["conclusion"].as_str(),
                Some("failure" | "timed_out" | "cancelled" | "action_required")
            )
        });
        let pending = runs.iter().any(|r| r["conclusion"].is_null());
        let status = if failed {
            "FAIL"
        } else if pending {
            return None;
        } else {
            "PASS"
        };
        Some(CiResult {
            status: status.to_owned(),
            sha: pr.head_sha.clone(),
            mode: None,
            current: true,
        })
    }
}

fn pr_of(v: &Value, merged: bool) -> Option<PrData> {
    Some(PrData {
        number: v["number"].as_u64()?,
        url: v["html_url"].as_str().unwrap_or("").to_owned(),
        title: v["title"].as_str().unwrap_or("").to_owned(),
        draft: v["draft"].as_bool().unwrap_or(false),
        head_ref: v["head"]["ref"].as_str().unwrap_or("").to_owned(),
        head_sha: v["head"]["sha"].as_str().unwrap_or("").to_owned(),
        updated_at: v["updated_at"].as_str().unwrap_or("").to_owned(),
        merged,
        files: vec![],
        checks: Checks::default(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fakegh::FakeGh;

    fn github(fake: &FakeGh, state: &Path) -> Github {
        let mut g = Github::new(fake.program(), state.to_path_buf());
        g.begin_pass(Instant::now(), 1_000_000);
        g
    }

    const PULLS: &str = "repos/o/r/pulls?state=open&per_page=100";
    const CLOSED: &str = "repos/o/r/pulls?state=closed&sort=updated&direction=desc&per_page=100";

    fn pr_json(n: u64, title: &str, branch: &str, sha: &str) -> String {
        format!(
            r#"{{"number":{n},"html_url":"https://github.com/o/r/pull/{n}","title":"{title}","draft":false,"updated_at":"2026-11-01T00:00:00Z","head":{{"ref":"{branch}","sha":"{sha}"}}}}"#
        )
    }

    fn names(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn the_status_line_is_parsed_and_the_exit_code_is_not_consulted() {
        let (status, headers, body) = parse_http(
            "HTTP/2.0 304 Not Modified\r\nX-Ratelimit-Remaining: 4999\r\nEtag: \"abc\"\r\n\r\n",
        )
        .unwrap();
        assert_eq!(status, 304);
        assert_eq!(header(&headers, "x-ratelimit-remaining"), Some("4999"));
        assert_eq!(header(&headers, "etag"), Some("\"abc\""));
        assert_eq!(body, "");
        assert_eq!(parse_http("gh: not logged in\n"), None);
    }

    #[test]
    fn a_304_with_exit_status_1_serves_the_cached_body_and_records_the_rate_limit() {
        let state = tempfile::tempdir().unwrap();
        let fake = FakeGh::new();
        fake.ok(PULLS, "\"v1\"", "X-Ratelimit-Remaining: 4000\r\n", "[1,2]");
        let mut g = github(&fake, state.path());
        assert_eq!(g.json(PULLS).unwrap().as_array().unwrap().len(), 2);
        // Now GitHub answers 304 and gh exits 1.
        fake.not_modified(
            PULLS,
            "X-Ratelimit-Remaining: 3999\r\nX-Ratelimit-Reset: 1700000000\r\n",
        );
        let mut g = github(&fake, state.path());
        assert_eq!(g.json(PULLS).unwrap().as_array().unwrap().len(), 2);
        assert_eq!(g.status, Status::Ok);
        assert_eq!(g.rate.remaining, Some(3999));
        assert_eq!(g.rate.reset, Some(1_700_000_000));
        let log = fake.log();
        assert!(log[1].contains("If-None-Match: \"v1\""), "{log:?}");
    }

    #[test]
    fn a_rate_limit_of_zero_stops_calls_until_the_reset_and_shows_the_time() {
        let state = tempfile::tempdir().unwrap();
        let fake = FakeGh::new();
        fake.ok(
            PULLS,
            "\"v1\"",
            "X-Ratelimit-Remaining: 0\r\nX-Ratelimit-Reset: 1000600\r\n",
            "[]",
        );
        let mut g = github(&fake, state.path());
        g.json(PULLS).unwrap();
        assert_eq!(fake.log().len(), 1);
        // Before the reset: no call, the cached body is served.
        assert!(g.json(PULLS).is_some());
        assert!(g.json(CLOSED).is_none());
        assert_eq!(fake.log().len(), 1);
        assert_eq!(g.status, Status::RateLimited { until: 1_000_600 });
        assert_eq!(
            g.status.chip(Instant::now()),
            "rate limited until 13:56 UTC"
        );
        // After the reset calls resume.
        g.begin_pass(Instant::now(), 1_000_601);
        g.json(PULLS).unwrap();
        assert_eq!(fake.log().len(), 2);
    }

    #[test]
    fn a_low_rate_limit_doubles_the_poll_interval() {
        let mut r = Rate::default();
        assert_eq!(r.interval(POLL), POLL);
        r.remaining = Some(199);
        assert_eq!(r.interval(POLL), POLL * 2);
        r.remaining = Some(200);
        assert_eq!(r.interval(POLL), POLL);
    }

    #[test]
    fn a_403_with_a_rate_limit_body_is_rate_limiting_not_an_error() {
        let state = tempfile::tempdir().unwrap();
        let fake = FakeGh::new();
        fake.raw(
            PULLS,
            "HTTP/2.0 403 Forbidden\r\nX-Ratelimit-Remaining: 0\r\nX-Ratelimit-Reset: 1000900\r\n\r\n{\"message\":\"API rate limit exceeded\"}",
            1,
        );
        let mut g = github(&fake, state.path());
        assert!(g.json(PULLS).is_none());
        assert_eq!(g.status, Status::RateLimited { until: 1_000_900 });
    }

    #[test]
    fn not_logged_in_names_gh_auth_login_and_serves_the_cache() {
        let state = tempfile::tempdir().unwrap();
        let fake = FakeGh::new();
        fake.ok(PULLS, "\"v1\"", "", "[7]");
        let mut g = github(&fake, state.path());
        g.json(PULLS).unwrap();
        // The login expires: gh prints its hint to stderr, with no status line.
        std::fs::write(
            fake.dir.path().join("fail"),
            "To get started with GitHub CLI, please run:  gh auth login\n",
        )
        .unwrap();
        let mut g = github(&fake, state.path());
        assert_eq!(g.json(PULLS).unwrap()[0], 7);
        assert_eq!(g.status, Status::NotLoggedIn);
        assert!(g.status.chip(Instant::now()).contains("gh auth login"));
        assert!(g.status.warning().unwrap().contains("gh auth login"));
        // A 401 is the same.
        std::fs::remove_file(fake.dir.path().join("fail")).unwrap();
        fake.raw(PULLS, "HTTP/2.0 401 Unauthorized\r\n\r\n{}", 1);
        std::fs::remove_file(state.path().join("cache").join(cache_name(PULLS))).unwrap();
        let mut g = github(&fake, state.path());
        assert!(g.json(PULLS).is_none());
        assert_eq!(g.status, Status::NotLoggedIn);
    }

    #[test]
    fn offline_serves_the_cache_with_one_status_and_stops_the_pass() {
        let state = tempfile::tempdir().unwrap();
        let fake = FakeGh::new();
        fake.ok(PULLS, "\"v1\"", "", "[1]");
        let mut g = github(&fake, state.path());
        g.json(PULLS).unwrap();
        std::fs::write(
            fake.dir.path().join("fail"),
            "error connecting to api.github.com\n",
        )
        .unwrap();
        let mut g = github(&fake, state.path());
        assert!(g.json(PULLS).is_some());
        assert!(matches!(g.status, Status::Offline { .. }));
        assert!(g.status.chip(Instant::now()).starts_with("gh offline"));
        // The rest of the pass makes no further call.
        let before = fake.log().len();
        g.json(CLOSED);
        assert_eq!(fake.log().len(), before);
    }

    #[test]
    fn a_missing_gh_is_a_warning_and_not_an_error() {
        let state = tempfile::tempdir().unwrap();
        let mut g = Github::new("/nonexistent/gh", state.path().to_path_buf());
        g.begin_pass(Instant::now(), 1_000_000);
        assert!(g.json(PULLS).is_none());
        assert_eq!(g.status, Status::Missing);
        assert_eq!(g.status.chip(Instant::now()), "gh off");
        assert!(g.status.warning().unwrap().contains("not found"));
    }

    #[test]
    fn a_gh_that_hangs_is_killed_after_the_timeout() {
        let fake = FakeGh::new();
        std::fs::write(fake.dir.path().join("sleep"), "").unwrap();
        let runner = Runner::new(fake.program(), Duration::from_millis(200));
        let start = Instant::now();
        assert_eq!(runner.get(PULLS, None), Reply::TimedOut);
        assert!(start.elapsed() < Duration::from_secs(4));
    }

    #[test]
    fn every_call_is_a_get_with_the_cleared_environment_and_no_body_flags() {
        let state = tempfile::tempdir().unwrap();
        let fake = FakeGh::new();
        fake.ok(
            PULLS,
            "\"v1\"",
            "",
            &format!(
                "[{}]",
                pr_json(
                    5,
                    "docs(openspec): propose search-ui",
                    "claude/x",
                    "aaaaaaa1"
                )
            ),
        );
        fake.ok(CLOSED, "\"v2\"", "", "[]");
        fake.ok("repos/o/r/pulls/5/files?per_page=100", "\"v3\"", "", "[]");
        let mut g = github(&fake, state.path());
        g.read_repo("o/r", &names(&["search-ui"]), &Patterns::default(), false);
        let log = fake.log();
        assert_eq!(log.len(), 3, "{log:?}");
        for l in &log {
            assert!(l.starts_with("api -i --method GET "), "{l}");
            for bad in [
                " -X",
                " -f ",
                " -F ",
                "--input",
                "--field",
                "--raw-field",
                "POST",
                "PATCH",
                "DELETE",
                "PUT",
            ] {
                assert!(!l.contains(bad), "{l}");
            }
        }
        // The environment is cleared: a token in the shell is not passed on.
        std::fs::write(
            fake.dir.path().join("gh"),
            "#!/bin/sh\nenv | sort > \"$(dirname \"$0\")/env\"\nprintf 'HTTP/2.0 200 OK\\r\\n\\r\\n[]'\n",
        )
        .unwrap();
        let r = Runner::new(fake.program(), TIMEOUT);
        // SAFETY: no other test reads this variable.
        unsafe { std::env::set_var("GH_TOKEN", "secret") };
        r.get(PULLS, None);
        unsafe { std::env::remove_var("GH_TOKEN") };
        let env = std::fs::read_to_string(fake.dir.path().join("env")).unwrap();
        assert!(!env.contains("GH_TOKEN"), "{env}");
        assert!(env.contains("PATH="));
    }

    #[test]
    fn open_and_recently_merged_prs_are_read_and_old_merges_are_not() {
        let state = tempfile::tempdir().unwrap();
        let fake = FakeGh::new();
        fake.ok(
            PULLS,
            "\"a\"",
            "",
            &format!(
                "[{}]",
                pr_json(5, "wip", "claude/search-ui-x7q", "aaaaaaa1")
            ),
        );
        let merged = |n, at| {
            format!(
                r#"{{"number":{n},"html_url":"u","title":"t","updated_at":"{at}","merged_at":"{at}","head":{{"ref":"b","sha":"s"}}}}"#
            )
        };
        // now = 1_000_000 s = 1970-01-12; both timestamps are relative to it.
        fake.ok(
            CLOSED,
            "\"b\"",
            "",
            &format!(
                "[{},{}]",
                merged(6, "1970-01-11T00:00:00Z"),
                merged(7, "1969-12-01T00:00:00Z")
            ),
        );
        fake.ok(
            "repos/o/r/pulls/5/files?per_page=100",
            "\"c\"",
            "",
            r#"[{"filename":"openspec/changes/search-ui/proposal.md","status":"added"}]"#,
        );
        let mut g = github(&fake, state.path());
        g.read_repo("o/r", &names(&["search-ui"]), &Patterns::default(), false);
        let prs = &g.data["o/r"];
        assert_eq!(
            prs.iter().map(|p| (p.number, p.merged)).collect::<Vec<_>>(),
            [(5, false), (6, true)]
        );
        assert_eq!(prs[0].files[0].status, "added");
        assert_eq!(prs[0].url, "https://github.com/o/r/pull/5");
    }

    #[test]
    fn the_files_of_a_pr_are_read_again_only_when_its_head_moves() {
        let state = tempfile::tempdir().unwrap();
        let fake = FakeGh::new();
        fake.ok(
            PULLS,
            "\"a\"",
            "",
            &format!("[{}]", pr_json(5, "t", "b", "aaaaaaa1")),
        );
        fake.ok(CLOSED, "\"b\"", "", "[]");
        fake.ok("repos/o/r/pulls/5/files?per_page=100", "\"c\"", "", "[]");
        let mut g = github(&fake, state.path());
        g.read_repo("o/r", &[], &Patterns::default(), false);
        g.begin_pass(Instant::now(), 1_000_000);
        g.read_repo("o/r", &[], &Patterns::default(), false);
        let files_calls = |g: &FakeGh| g.log().iter().filter(|l| l.contains("/files")).count();
        assert_eq!(files_calls(&fake), 1);
        fake.ok(
            PULLS,
            "\"a2\"",
            "",
            &format!("[{}]", pr_json(5, "t", "b", "bbbbbbb2")),
        );
        g.begin_pass(Instant::now(), 1_000_000);
        g.read_repo("o/r", &[], &Patterns::default(), false);
        assert_eq!(files_calls(&fake), 2);
    }

    fn patterns() -> Patterns {
        Patterns {
            ci: Some(
                r"Claude CI \((?P<mode>\w+)\): (?P<status>PASS|FAIL) (?P<sha>[0-9a-f]{7,40})"
                    .into(),
            ),
            review: Some(
                r"Claude review: (?P<verdict>[A-Za-z _-]+?) (?P<sha>[0-9a-f]{7,40})".into(),
            ),
            owner_needed: Some(r"(?m)^Owner approval: yes\s*$".into()),
            task: None,
        }
    }

    fn bodies(v: &[&str]) -> Vec<String> {
        v.iter().rev().map(|s| s.to_string()).collect()
    }

    #[test]
    fn the_newest_ci_match_wins() {
        let c = scan_comments(
            &bodies(&[
                "Claude CI (full): FAIL aaa1111",
                "noise",
                "Claude CI (full): PASS bbb2222",
            ]),
            &patterns(),
            "bbb2222ffff",
        );
        let ci = c.ci.unwrap();
        assert_eq!(
            (ci.status.as_str(), ci.sha.as_str(), ci.current),
            ("PASS", "bbb2222", true)
        );
        assert_eq!(ci.mode.as_deref(), Some("full"));
    }

    #[test]
    fn a_review_for_the_head_is_current_and_one_for_an_older_commit_is_stale() {
        let c = scan_comments(
            &bodies(&["Claude review: APPROVED 9c8d7e6"]),
            &patterns(),
            "9c8d7e6a1b",
        );
        let r = c.review.unwrap();
        assert_eq!((r.verdict.as_str(), r.current), ("APPROVED", true));
        let c = scan_comments(
            &bodies(&["Claude review: PASS 5b6c7d8"]),
            &patterns(),
            "77aa11c0000",
        );
        let r = c.review.unwrap();
        assert_eq!((r.verdict.as_str(), r.current), ("PASS", false));
    }

    #[test]
    fn owner_approval_on_its_own_line_is_found_in_the_whole_body_of_a_current_review() {
        let body =
            "Claude review: request changes 9c8d7e6\n\nfindings...\nOwner approval: yes\nmore\n";
        let c = scan_comments(&bodies(&[body]), &patterns(), "9c8d7e6aaa");
        assert!(c.owner_needed);
        assert_eq!(c.review.unwrap().verdict, "request changes");
        // The same body for an older head does not count.
        assert!(!scan_comments(&bodies(&[body]), &patterns(), "1111111aaa").owner_needed);
        // Only the newest review comment is looked at.
        let newer = "Claude review: approve 9c8d7e6\nno owner needed\n";
        assert!(!scan_comments(&bodies(&[body, newer]), &patterns(), "9c8d7e6aaa").owner_needed);
    }

    #[test]
    fn without_patterns_no_comment_result_is_shown() {
        let c = scan_comments(
            &bodies(&["Claude CI (full): PASS bbb2222"]),
            &Patterns::default(),
            "bbb2222ff",
        );
        assert_eq!(c, Checks::default());
    }

    #[test]
    fn comments_are_read_from_the_last_page_backwards() {
        let state = tempfile::tempdir().unwrap();
        let fake = FakeGh::new();
        let base = "repos/o/r/issues/5/comments?per_page=100";
        // Page 1 is full (100 comments, the CI one the oldest), page 2 has
        // the newest comment: the walk finds page 2 without any `Link`.
        let mut page1 = vec![r#"{"body":"Claude CI (full): FAIL aaa1111"}"#.to_owned()];
        page1.extend((1..100).map(|_| r#"{"body":"noise"}"#.to_owned()));
        fake.ok(base, "\"p1\"", "", &format!("[{}]", page1.join(",")));
        fake.ok(
            &format!("{base}&page=2"),
            "\"p2\"",
            "",
            r#"[{"body":"Claude review: APPROVED 9c8d7e6"}]"#,
        );
        let mut g = github(&fake, state.path());
        let pr = PrData {
            number: 5,
            url: String::new(),
            title: String::new(),
            draft: false,
            head_ref: String::new(),
            head_sha: "9c8d7e6aa".into(),
            updated_at: "u".into(),
            merged: false,
            files: vec![],
            checks: Checks::default(),
        };
        let c = g.scan_pr_comments("o/r", &pr, &patterns());
        assert_eq!(c.review.unwrap().verdict, "APPROVED");
        assert_eq!(c.ci.unwrap().status, "FAIL");
    }

    #[test]
    fn native_check_runs_are_used_when_asked_and_no_ci_pattern_is_set() {
        let state = tempfile::tempdir().unwrap();
        let fake = FakeGh::new();
        fake.ok(
            "repos/o/r/commits/9c8d7e6aa/check-runs?per_page=100",
            "\"x\"",
            "",
            r#"{"check_runs":[{"conclusion":"success"},{"conclusion":"failure"}]}"#,
        );
        let mut g = github(&fake, state.path());
        let pr = pr_of(
            &serde_json::from_str(&pr_json(5, "t", "b", "9c8d7e6aa")).unwrap(),
            false,
        )
        .unwrap();
        let ci = g.native_ci("o/r", &pr).unwrap();
        assert_eq!((ci.status.as_str(), ci.current), ("FAIL", true));
    }

    #[test]
    fn pending_native_checks_are_read_again_and_not_remembered() {
        let state = tempfile::tempdir().unwrap();
        let fake = FakeGh::new();
        let ep = "repos/o/r/commits/9c8d7e6aa/check-runs?per_page=100";
        fake.ok(ep, "\"x\"", "", r#"{"check_runs":[{"conclusion":null}]}"#);
        let mut g = github(&fake, state.path());
        let pr = pr_of(
            &serde_json::from_str(&pr_json(5, "t", "b", "9c8d7e6aa")).unwrap(),
            false,
        )
        .unwrap();
        let none = Patterns::default();
        assert_eq!(g.pr_checks("o/r", &pr, &none, true).ci, None);
        // The runs finish; the PR's updated_at did not move.
        fake.ok(
            ep,
            "\"y\"",
            "",
            r#"{"check_runs":[{"conclusion":"success"}]}"#,
        );
        g.begin_pass(Instant::now(), 1_000_000);
        let ci = g.pr_checks("o/r", &pr, &none, true).ci.unwrap();
        assert_eq!(ci.status, "PASS");
    }

    #[test]
    fn slugs_come_from_github_com_remotes_only() {
        for u in [
            "https://github.com/o/r.git",
            "https://github.com/o/r",
            "git@github.com:o/r.git",
            "ssh://git@github.com/o/r.git",
            "https://user:tok@github.com/o/r.git",
        ] {
            assert_eq!(slug_of(u).as_deref(), Some("o/r"), "{u}");
        }
        for u in [
            "https://gitlab.com/o/r.git",
            "git@example.com:o/r.git",
            "/srv/git/r.git",
            "https://github.com/o",
        ] {
            assert_eq!(slug_of(u), None, "{u}");
        }
    }

    #[test]
    fn iso_times_convert_to_unix_seconds() {
        assert_eq!(parse_iso("1970-01-01T00:00:00Z"), Some(0));
        assert_eq!(parse_iso("2026-11-15T12:30:05Z"), Some(1_794_745_805));
        assert_eq!(parse_iso("nope"), None);
    }
}
