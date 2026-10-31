//! Port of the git helpers in `daemon/src/tmux.js` (`resolveBranch`) and
//! `daemon/src/worktrees.js`. Git stays a subprocess: `git -C <dir> ...`.

use std::process::Stdio;

use tokio::process::Command;

async fn git(dir: &str, args: &[&str]) -> Option<String> {
    let mut cmd = Command::new("git");
    cmd.arg("-C").arg(dir).args(args).stdin(Stdio::null()).stderr(Stdio::null()).kill_on_drop(true);
    // Timeout drops the future, which kills the child (kill_on_drop).
    let out = tokio::time::timeout(crate::backend::COMMAND_TIMEOUT, cmd.output()).await.ok()?.ok()?;
    out.status.success().then(|| String::from_utf8_lossy(&out.stdout).trim().to_string())
}

/// What HEAD is at in `dir`: the branch name, `@<short-sha>` for a detached
/// HEAD, or `None` for anything that is not a usable git working directory
/// (empty dir, not a repo, vanished). Never errors: a git failure must not
/// break the sessions/workspaces list.
pub async fn resolve_branch(dir: &str) -> Option<String> {
    if dir.is_empty() {
        return None;
    }
    let branch = git(dir, &["rev-parse", "--abbrev-ref", "HEAD"]).await?;
    if !branch.is_empty() && branch != "HEAD" {
        return Some(branch);
    }
    let sha = git(dir, &["rev-parse", "--short", "HEAD"]).await?;
    (!sha.is_empty()).then(|| format!("@{sha}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::{Path, PathBuf};

    fn sh(dir: &Path, args: &[&str]) {
        let ok = std::process::Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(args)
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_CONFIG_SYSTEM", "/dev/null")
            .env("GIT_AUTHOR_NAME", "t")
            .env("GIT_AUTHOR_EMAIL", "t@t")
            .env("GIT_COMMITTER_NAME", "t")
            .env("GIT_COMMITTER_EMAIL", "t@t")
            .output()
            .unwrap()
            .status
            .success();
        assert!(ok, "git {args:?}");
    }

    struct Tmp(PathBuf);
    impl Tmp {
        fn new(tag: &str) -> Self {
            let p = std::env::temp_dir().join(format!("garage-git-{tag}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&p);
            std::fs::create_dir_all(&p).unwrap();
            Tmp(p)
        }
    }
    impl Drop for Tmp {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn repo(tag: &str) -> Tmp {
        let t = Tmp::new(tag);
        sh(&t.0, &["init", "-q", "-b", "feat-x"]);
        sh(&t.0, &["commit", "-q", "--allow-empty", "-m", "init"]);
        t
    }

    #[tokio::test]
    async fn branch_name_is_returned() {
        let t = repo("branch");
        assert_eq!(resolve_branch(t.0.to_str().unwrap()).await.as_deref(), Some("feat-x"));
    }

    #[tokio::test]
    async fn detached_head_is_a_short_sha_prefixed_with_at() {
        let t = repo("detached");
        sh(&t.0, &["checkout", "-q", "--detach"]);
        let b = resolve_branch(t.0.to_str().unwrap()).await.unwrap();
        assert!(b.starts_with('@') && b.len() > 4, "{b}");
    }

    #[tokio::test]
    async fn non_repo_empty_and_missing_dirs_are_none() {
        let t = Tmp::new("plain");
        // The temp dir is not inside a repo (std::env::temp_dir is outside the checkout).
        assert_eq!(resolve_branch(t.0.to_str().unwrap()).await, None);
        assert_eq!(resolve_branch("").await, None);
        assert_eq!(resolve_branch("/nonexistent/garage-dir").await, None);
    }

    #[tokio::test]
    async fn repo_without_commits_is_none() {
        let t = Tmp::new("empty-repo");
        sh(&t.0, &["init", "-q", "-b", "main"]);
        // JS: rev-parse --abbrev-ref HEAD fails on an unborn branch -> null.
        assert_eq!(resolve_branch(t.0.to_str().unwrap()).await, None);
    }
}
