//! Port of the create half of `daemon/src/worktrees.js` (`createWorktree`,
//! `currentBranch`). The finish half belongs to the worktrees route.

use std::path::{Path, PathBuf};
use std::process::Stdio;

use tokio::process::Command;

/// Branches garage creates live under this prefix, so a repo's real branches
/// never collide with ours.
pub const GARAGE_BRANCH_PREFIX: &str = "garage/";

struct GitOut {
    ok: bool,
    stdout: String,
    stderr: String,
}

async fn git(dir: &str, args: &[&str]) -> Option<GitOut> {
    let out = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .stdin(Stdio::null())
        .kill_on_drop(true)
        .output()
        .await
        .ok()?;
    Some(GitOut {
        ok: out.status.success(),
        stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
    })
}

async fn branch_exists(repo_dir: &str, branch: &str) -> bool {
    let r = format!("refs/heads/{branch}");
    git(repo_dir, &["rev-parse", "--verify", "--quiet", &r]).await.is_some_and(|o| o.ok)
}

/// Directory and branch are suffixed together (`-2`, `-3`, ...) until a
/// candidate is free on both axes, so `garage/<label>` and its directory stay
/// visually paired after a collision.
async fn pick_candidate(root: &Path, repo_dir: &str, workspace: &str, label: &str) -> (PathBuf, String) {
    let base = root.join(workspace);
    let mut suffix = 1;
    let mut candidate = label.to_owned();
    loop {
        let dir_path = base.join(&candidate);
        let branch = format!("{GARAGE_BRANCH_PREFIX}{candidate}");
        let taken = dir_path.exists() || branch_exists(repo_dir, &branch).await;
        if !taken {
            return (dir_path, branch);
        }
        suffix += 1;
        candidate = format!("{label}-{suffix}");
    }
}

/// Creates `<garage_dir>/worktrees/<workspace>/<label>` (suffixed on
/// collision) as a new worktree on a fresh `garage/<label>` branch cut from
/// `repo_dir`'s HEAD. `Err` is the message for a 400: not a git repository, or
/// git's own stderr when `git worktree add` fails. Nothing is created before
/// this succeeds.
pub async fn create_worktree(
    garage_dir: &Path,
    repo_dir: &str,
    workspace: &str,
    label: &str,
) -> Result<(String, String), String> {
    if !git(repo_dir, &["rev-parse", "--is-inside-work-tree"]).await.is_some_and(|o| o.ok) {
        return Err(format!("not a git repository: {repo_dir}"));
    }
    let root = garage_dir.join("worktrees");
    let (dir_path, branch) = pick_candidate(&root, repo_dir, workspace, label).await;
    if let Some(parent) = dir_path.parent() {
        tokio::fs::create_dir_all(parent).await.map_err(|e| e.to_string())?;
    }
    let dir_str = dir_path.to_string_lossy().into_owned();
    match git(repo_dir, &["worktree", "add", "-b", &branch, &dir_str]).await {
        Some(o) if o.ok => Ok((dir_str, branch)),
        Some(o) if !o.stderr.trim().is_empty() => Err(o.stderr.trim().to_owned()),
        Some(_) => Err("git worktree add failed".to_owned()),
        None => Err("cannot run git".to_owned()),
    }
}

/// Undo `create_worktree` (a failed spawn must not leave a stray worktree and
/// branch behind): `git worktree remove --force`, then `branch -D`. Best
/// effort; each step runs even if the other failed.
pub async fn remove_worktree(repo_dir: &str, path: &str, branch: &str) {
    let _ = git(repo_dir, &["worktree", "remove", "--force", path]).await;
    let _ = git(repo_dir, &["branch", "-D", branch]).await;
}

/// The branch checked out in `repo_dir` (where a merge would land), or `None`
/// when it cannot be read (not a repo, unborn HEAD). A detached HEAD reads as
/// `"HEAD"`, as in JS. Informational only.
pub async fn current_branch(repo_dir: &str) -> Option<String> {
    let o = git(repo_dir, &["rev-parse", "--abbrev-ref", "HEAD"]).await.filter(|o| o.ok)?;
    let b = o.stdout.trim();
    (!b.is_empty()).then(|| b.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

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

    fn repo(root: &Path) -> String {
        let r = root.join("repo");
        std::fs::create_dir_all(&r).unwrap();
        sh(&r, &["init", "-q", "-b", "main"]);
        sh(&r, &["commit", "-q", "--allow-empty", "-m", "init"]);
        r.to_string_lossy().into_owned()
    }

    #[tokio::test]
    async fn creates_under_garage_dir_on_a_garage_branch_and_suffixes_collisions_together() {
        let tmp = tempfile::tempdir().unwrap();
        let repo = repo(tmp.path());
        let garage = tmp.path().join("g");

        let (p1, b1) = create_worktree(&garage, &repo, "ws", "fix").await.unwrap();
        assert_eq!(Path::new(&p1), garage.join("worktrees/ws/fix"));
        assert_eq!(b1, "garage/fix");
        assert!(Path::new(&p1).join(".git").exists());

        // Same label again: dir and branch both move to -2 together.
        let (p2, b2) = create_worktree(&garage, &repo, "ws", "fix").await.unwrap();
        assert_eq!(Path::new(&p2), garage.join("worktrees/ws/fix-2"));
        assert_eq!(b2, "garage/fix-2");

        // A free dir but an existing branch also forces the suffix.
        sh(Path::new(&repo), &["branch", "garage/other"]);
        let (p3, b3) = create_worktree(&garage, &repo, "ws", "other").await.unwrap();
        assert_eq!((Path::new(&p3), b3.as_str()), (garage.join("worktrees/ws/other-2").as_path(), "garage/other-2"));
    }

    #[tokio::test]
    async fn non_repo_is_a_named_error_and_creates_nothing() {
        let tmp = tempfile::tempdir().unwrap();
        let plain = tmp.path().join("plain");
        std::fs::create_dir_all(&plain).unwrap();
        let garage = tmp.path().join("g");
        let e = create_worktree(&garage, plain.to_str().unwrap(), "ws", "fix").await.unwrap_err();
        assert_eq!(e, format!("not a git repository: {}", plain.display()));
        assert!(!garage.join("worktrees").exists());
    }

    #[tokio::test]
    async fn current_branch_reads_head_and_is_none_outside_a_repo() {
        let tmp = tempfile::tempdir().unwrap();
        let repo = repo(tmp.path());
        assert_eq!(current_branch(&repo).await.as_deref(), Some("main"));
        assert_eq!(current_branch(tmp.path().to_str().unwrap()).await, None);
    }
}
