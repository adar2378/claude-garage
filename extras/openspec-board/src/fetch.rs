//! `git fetch` of a repo's own `origin`, confined and throttled.
//!
//! The fetch writes the remote-tracking refs under `refs/remotes/origin` and
//! the objects it downloads, nothing else: no `FETCH_HEAD`, no gc, no
//! maintenance, no prompt, no stdin. Only the pane (while open, or on `r`)
//! calls it.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// At most one fetch per repo in this time, unless the user forces it.
pub const THROTTLE: Duration = Duration::from_secs(5 * 60);
/// A fetch running longer than this is killed.
pub const TIMEOUT: Duration = Duration::from_secs(30);

/// The arguments after `git -C <repo>`.
pub const ARGS: [&str; 6] = [
    "fetch",
    "--prune",
    "--no-auto-gc",
    "--no-auto-maintenance",
    "--no-write-fetch-head",
    "origin",
];
pub const SSH_COMMAND: &str = "ssh -o BatchMode=yes";

/// Whether the repo has an `origin` remote (without one there is nothing to
/// fetch and nothing to report as failed).
pub fn has_origin(repo: &Path) -> bool {
    crate::local::git(repo, &["config", "--get", "remote.origin.url"]).is_some()
}

/// Run the fetch with `git` (a program name or path) and a timeout. stdin is
/// null, output is ignored, the child never prompts.
pub fn run_with(git: &Path, repo: &Path, timeout: Duration) -> Result<(), String> {
    let mut child = Command::new(git)
        .arg("-C")
        .arg(repo)
        .args(ARGS)
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_SSH_COMMAND", SSH_COMMAND)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| format!("git could not start: {e}"))?;
    let start = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(status)) if status.success() => return Ok(()),
            Ok(Some(status)) => return Err(format!("git fetch exited with {status}")),
            Ok(None) if start.elapsed() >= timeout => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(format!("git fetch timed out after {}s", timeout.as_secs()));
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(25)),
            Err(e) => return Err(format!("git fetch: {e}")),
        }
    }
}

/// How the fetch of one repo went last.
#[derive(Debug, Clone, Copy)]
struct Last {
    at: Instant,
    ok: bool,
}

/// When each repo was last fetched, for the throttle and the `fetch failed`
/// mark.
#[derive(Debug, Default)]
pub struct FetchState {
    last: HashMap<PathBuf, Last>,
}

impl FetchState {
    /// Whether a fetch should run now: never fetched, or the throttle has
    /// passed, or the user forced it.
    pub fn due(&self, repo: &Path, force: bool, now: Instant) -> bool {
        force
            || self
                .last
                .get(repo)
                .is_none_or(|l| now.saturating_duration_since(l.at) >= THROTTLE)
    }

    /// Fetch `repo` if due. `None` when it was not due (or has no origin);
    /// else the result, which is also recorded.
    pub fn fetch(
        &mut self,
        git: &Path,
        repo: &Path,
        force: bool,
        now: Instant,
        timeout: Duration,
    ) -> Option<Result<(), String>> {
        if !self.due(repo, force, now) || !has_origin(repo) {
            return None;
        }
        let result = run_with(git, repo, timeout);
        self.last.insert(
            repo.to_path_buf(),
            Last {
                at: now,
                ok: result.is_ok(),
            },
        );
        Some(result)
    }

    /// When the last fetch of `repo` failed, if the last one did.
    pub fn failed_at(&self, repo: &Path) -> Option<Instant> {
        self.last.get(repo).filter(|l| !l.ok).map(|l| l.at)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    fn fake_git(dir: &Path, body: &str) -> PathBuf {
        let p = dir.join("fakegit");
        std::fs::write(&p, format!("#!/bin/sh\n{body}\n")).unwrap();
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
        p
    }

    #[test]
    fn the_command_line_and_environment_are_confined() {
        let d = tempfile::tempdir().unwrap();
        let log = d.path().join("log");
        let git = fake_git(
            d.path(),
            &format!(
                "{{ echo \"args=$*\"; echo \"prompt=$GIT_TERMINAL_PROMPT\"; \
                 echo \"ssh=$GIT_SSH_COMMAND\"; echo \"stdin=$(cat | wc -c)\"; }} > {}",
                log.display()
            ),
        );
        run_with(&git, Path::new("/some/repo"), TIMEOUT).unwrap();
        let text = std::fs::read_to_string(&log).unwrap();
        assert_eq!(
            text,
            "args=-C /some/repo fetch --prune --no-auto-gc --no-auto-maintenance \
             --no-write-fetch-head origin\nprompt=0\nssh=ssh -o BatchMode=yes\nstdin=0\n"
        );
    }

    #[test]
    fn a_fetch_that_runs_too_long_is_killed_and_reported() {
        let d = tempfile::tempdir().unwrap();
        let git = fake_git(d.path(), "sleep 30");
        let start = Instant::now();
        let err = run_with(&git, d.path(), Duration::from_millis(200)).unwrap_err();
        assert!(err.contains("timed out"), "{err}");
        assert!(start.elapsed() < Duration::from_secs(10));
    }

    #[test]
    fn a_failing_fetch_is_an_error() {
        let d = tempfile::tempdir().unwrap();
        let git = fake_git(d.path(), "exit 128");
        assert!(run_with(&git, d.path(), TIMEOUT).is_err());
    }

    fn git_ok(dir: &Path, args: &[&str]) {
        assert!(
            Command::new("git")
                .arg("-C")
                .arg(dir)
                .args(args)
                .stdout(Stdio::null())
                .status()
                .unwrap()
                .success()
        );
    }

    fn repo_with_origin(dir: &Path) {
        std::fs::create_dir_all(dir).unwrap();
        git_ok(dir, &["init", "-q", "-b", "main"]);
        git_ok(dir, &["remote", "add", "origin", "/nonexistent/origin"]);
    }

    #[test]
    fn the_throttle_allows_one_fetch_per_five_minutes_unless_forced() {
        let d = tempfile::tempdir().unwrap();
        let repo = d.path().join("r");
        repo_with_origin(&repo);
        let git = fake_git(d.path(), "exit 0");
        let mut st = FetchState::default();
        let t0 = Instant::now();
        assert_eq!(st.fetch(&git, &repo, false, t0, TIMEOUT), Some(Ok(())));
        // Within the throttle: nothing runs.
        let soon = t0 + Duration::from_secs(60);
        assert_eq!(st.fetch(&git, &repo, false, soon, TIMEOUT), None);
        // `r` forces it.
        assert_eq!(st.fetch(&git, &repo, true, soon, TIMEOUT), Some(Ok(())));
        // After the throttle it is due again.
        assert!(st.due(&repo, false, soon + THROTTLE));
    }

    #[test]
    fn a_failure_is_recorded_and_a_repo_without_origin_is_skipped() {
        let d = tempfile::tempdir().unwrap();
        let repo = d.path().join("r");
        repo_with_origin(&repo);
        let git = fake_git(d.path(), "exit 1");
        let mut st = FetchState::default();
        let t0 = Instant::now();
        assert!(st.fetch(&git, &repo, false, t0, TIMEOUT).unwrap().is_err());
        assert_eq!(st.failed_at(&repo), Some(t0));

        let plain = d.path().join("plain");
        std::fs::create_dir_all(&plain).unwrap();
        git_ok(&plain, &["init", "-q"]);
        assert_eq!(st.fetch(&git, &plain, true, t0, TIMEOUT), None);
        assert_eq!(st.failed_at(&plain), None);
    }
}
