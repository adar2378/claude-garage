//! Port of the git helpers in `daemon/src/tmux.js` (`resolveBranch`) and
//! `daemon/src/worktrees.js`. Git stays a subprocess: `git -C <dir> ...`.

use std::path::{Path, PathBuf};
use std::process::Stdio;

use garage_protocol::{FinishAction, FinishWorktreeResponse, WorktreeRef};
use tokio::process::Command;

async fn git(dir: &str, args: &[&str]) -> Option<String> {
    let mut cmd = Command::new("git");
    cmd.arg("-C")
        .arg(dir)
        .args(args)
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .kill_on_drop(true);
    // Timeout drops the future, which kills the child (kill_on_drop).
    let out = tokio::time::timeout(crate::backend::COMMAND_TIMEOUT, cmd.output())
        .await
        .ok()?
        .ok()?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).trim().to_string())
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

/// A failed git operation, shaped for the routes: an HTTP `status`, git's
/// trimmed stderr (or the command line) as `message`, and the raw `stderr`
/// when the process ran (the JS wrapper's `.stderr`; surfaced in 409/500 bodies).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GitFailure {
    pub status: u16,
    pub message: String,
    pub stderr: Option<String>,
}

impl GitFailure {
    fn new(status: u16, message: impl Into<String>) -> Self {
        Self {
            status,
            message: message.into(),
            stderr: None,
        }
    }
}

/// `git -C <dir> <args>` with stderr kept. No timeout, like the JS `execFile`
/// (a merge may legitimately take a while); stdin is null so git can never
/// open an editor or prompt.
async fn git_checked(dir: &str, args: &[&str]) -> Result<String, GitFailure> {
    let mut cmd = Command::new("git");
    cmd.arg("-C")
        .arg(dir)
        .args(args)
        .stdin(Stdio::null())
        .kill_on_drop(true);
    let out = cmd.output().await.map_err(|e| {
        GitFailure::new(
            500,
            format!("Command failed: git -C {dir} {}: {e}", args.join(" ")),
        )
    })?;
    if out.status.success() {
        return Ok(String::from_utf8_lossy(&out.stdout).into_owned());
    }
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    let message = match stderr.trim() {
        "" => format!("Command failed: git -C {dir} {}", args.join(" ")),
        t => t.to_owned(),
    };
    Err(GitFailure {
        status: 500,
        message,
        stderr: Some(stderr),
    })
}

/// Namespace for every branch garage creates; discard/merge refuse the rest.
pub const GARAGE_BRANCH_PREFIX: &str = "garage/";

async fn branch_exists(repo_dir: &str, branch: &str) -> bool {
    git_checked(
        repo_dir,
        &[
            "rev-parse",
            "--verify",
            "--quiet",
            &format!("refs/heads/{branch}"),
        ],
    )
    .await
    .is_ok()
}

/// Directory and branch share one candidate label, suffixed together (`-2`,
/// `-3`, ...) until both are free (JS `pickCandidate`).
async fn pick_candidate(
    garage_dir: &Path,
    repo_dir: &str,
    workspace: &str,
    label: &str,
) -> (PathBuf, String) {
    let base = garage_dir.join("worktrees").join(workspace);
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
/// `repo_dir`'s HEAD. 400 (git's stderr as message) when `repo_dir` is not a
/// repository or `git worktree add` fails; nothing else is created before
/// that succeeds. The JS derived the root from `GARAGE_DIR` at import time;
/// here the caller passes `config.garage_dir`.
pub async fn create_worktree(
    garage_dir: &Path,
    repo_dir: &str,
    workspace: &str,
    label: &str,
) -> Result<WorktreeRef, GitFailure> {
    if git_checked(repo_dir, &["rev-parse", "--is-inside-work-tree"])
        .await
        .is_err()
    {
        return Err(GitFailure::new(
            400,
            format!("not a git repository: {repo_dir}"),
        ));
    }
    let (dir_path, branch) = pick_candidate(garage_dir, repo_dir, workspace, label).await;
    let path = dir_path.to_string_lossy().into_owned();
    if let Some(parent) = dir_path.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(|e| GitFailure::new(500, e.to_string()))?;
    }
    git_checked(repo_dir, &["worktree", "add", "-b", &branch, &path])
        .await
        .map_err(|e| GitFailure { status: 400, ..e })?;
    Ok(WorktreeRef { path, branch })
}

/// The branch checked out in `repo_dir` (what a merge would land on), or
/// `None` when unreadable. Informational only.
pub async fn current_branch(repo_dir: &str) -> Option<String> {
    let out = git_checked(repo_dir, &["rev-parse", "--abbrev-ref", "HEAD"])
        .await
        .ok()?;
    let t = out.trim();
    (!t.is_empty()).then(|| t.to_owned())
}

/// Undo `create_worktree` when the session spawn failed afterwards (a stray
/// worktree and branch must not be left behind): `worktree remove --force`,
/// then `branch -D`. Best effort; each step runs even if the other failed.
pub async fn remove_worktree(repo_dir: &str, path: &str, branch: &str) {
    let _ = git_checked(repo_dir, &["worktree", "remove", "--force", path]).await;
    let _ = git_checked(repo_dir, &["branch", "-D", branch]).await;
}

/// Refuses to delete or merge anything that is not a garage-made worktree:
/// `path` must resolve to somewhere under `<garage_dir>/worktrees/` and, when
/// `require_head`, have `branch` checked out (merge only: discard must still
/// work after the agent switched branches, detached, or is mid-rebase; the
/// folder is ours either way). The record comes from the client, so a forged path
/// must never reach `worktree remove --force`.
async fn assert_garage_worktree(
    garage_dir: &Path,
    path: &str,
    branch: &str,
    require_head: bool,
) -> Result<(), GitFailure> {
    let refuse = || {
        GitFailure::new(
            400,
            format!("refusing to touch a path that is not a garage worktree: {path}"),
        )
    };
    let root = tokio::fs::canonicalize(garage_dir.join("worktrees"))
        .await
        .map_err(|_| refuse())?;
    let real = match tokio::fs::canonicalize(path).await {
        Ok(real) => real,
        // The directory is already gone (deleted by hand, or a crash): there
        // is nothing on disk to damage, and the branch still needs cleaning
        // up, as the JS daemon did. Allowed only when the path, read
        // literally, is a clean child of the worktrees root.
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            let p = Path::new(path);
            let clean = p.is_absolute()
                && p.components().all(|c| {
                    !matches!(
                        c,
                        std::path::Component::ParentDir | std::path::Component::CurDir
                    )
                });
            let under = |root: &Path| p != root && p.starts_with(root);
            let raw_root = garage_dir.join("worktrees");
            return if clean && (under(&root) || under(&raw_root)) {
                Ok(())
            } else {
                Err(refuse())
            };
        }
        Err(_) => return Err(refuse()),
    };
    if real == root || !real.starts_with(&root) {
        return Err(refuse());
    }
    if !require_head {
        return Ok(());
    }
    match git_checked(path, &["rev-parse", "--abbrev-ref", "HEAD"]).await {
        Ok(head) if head.trim() == branch => Ok(()),
        _ => Err(refuse()),
    }
}

fn assert_garage_branch(branch: &str) -> Result<(), GitFailure> {
    if branch.starts_with(GARAGE_BRANCH_PREFIX) {
        Ok(())
    } else {
        Err(GitFailure::new(
            400,
            format!("refusing to touch non-garage branch: {branch}"),
        ))
    }
}

/// Runs after the session is dead. `repo_dir`/`path`/`branch` come from the
/// worktree record the caller holds, never re-derived. `merge` and `discard`
/// only ever touch `garage/*` branches in worktrees under
/// `<garage_dir>/worktrees/` (checked before anything is removed); `keep` is a deliberate no-op (and,
/// as in JS, is not branch-checked).
pub async fn finish_worktree(
    garage_dir: &Path,
    repo_dir: &str,
    path: &str,
    branch: &str,
    action: FinishAction,
) -> Result<FinishWorktreeResponse, GitFailure> {
    let response = |merged, discarded| FinishWorktreeResponse {
        action,
        path: path.to_owned(),
        branch: branch.to_owned(),
        merged,
        discarded,
    };
    match action {
        FinishAction::Keep => Ok(response(None, None)),
        FinishAction::Merge => {
            assert_garage_branch(branch)?;
            assert_garage_worktree(garage_dir, path, branch, true).await?;
            // The usual failure is an agent that edited but never committed:
            // nothing to merge, and `worktree remove` would refuse anyway. A
            // failing status call is swallowed (JS `.catch(() => null)`).
            if let Ok(dirty) = git_checked(path, &["status", "--porcelain"]).await
                && !dirty.trim().is_empty()
            {
                return Err(GitFailure::new(
                    409,
                    "worktree has uncommitted changes \u{2014} commit them in the session first, or discard",
                ));
            }
            if let Err(e) = git_checked(repo_dir, &["merge", "--no-ff", branch]).await {
                // Abort so repo_dir is not left mid-merge (no conflict UI to finish it).
                let _ = git_checked(repo_dir, &["merge", "--abort"]).await;
                return Err(GitFailure {
                    status: 409,
                    message: "merge conflict \u{2014} merge aborted, nothing changed; resolve it in the session (merge the target into the branch), then try again".into(),
                    stderr: e.stderr,
                });
            }
            git_checked(repo_dir, &["worktree", "remove", path]).await?;
            git_checked(repo_dir, &["branch", "-d", branch]).await?;
            Ok(response(Some(true), None))
        }
        FinishAction::Discard => {
            assert_garage_branch(branch)?;
            assert_garage_worktree(garage_dir, path, branch, false).await?;
            if Path::new(path).exists() {
                git_checked(repo_dir, &["worktree", "remove", "--force", path]).await?;
            } else {
                // Folder already gone: drop the stale registration so the
                // branch is free to delete.
                git_checked(repo_dir, &["worktree", "prune"]).await?;
            }
            if branch_exists(repo_dir, branch).await {
                git_checked(repo_dir, &["branch", "-D", branch]).await?;
            }
            Ok(response(None, Some(true)))
        }
    }
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
        assert_eq!(
            resolve_branch(t.0.to_str().unwrap()).await.as_deref(),
            Some("feat-x")
        );
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

    // ---- Lane F: worktrees.js (finish / create / currentBranch) ----

    fn g(dir: &Path, args: &[&str]) -> String {
        let out = std::process::Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(args)
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_CONFIG_SYSTEM", "/dev/null")
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "git {args:?}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8_lossy(&out.stdout).into_owned()
    }

    fn commit_file(dir: &Path, name: &str, content: &str, msg: &str) {
        std::fs::write(dir.join(name), content).unwrap();
        g(dir, &["add", name]);
        g(dir, &["commit", "-q", "-m", msg]);
    }

    /// Scratch repo on `main` plus a `garage/x` worktree (the JS `before`).
    /// Identity and signing are set repo-locally: the daemon's merge commit
    /// runs as a plain subprocess and must not depend on the developer's
    /// global git config.
    struct Fixture {
        _root: tempfile::TempDir,
        scratch: PathBuf,
        repo: PathBuf,
        garage: PathBuf,
        wt: PathBuf,
    }

    fn fixture() -> Fixture {
        let root = tempfile::tempdir().unwrap();
        let scratch = root.path().canonicalize().unwrap();
        let repo = scratch.join("repo");
        let garage = scratch.join("garage");
        let wt = garage.join("worktrees/ws/x");
        std::fs::create_dir(&repo).unwrap();
        std::fs::create_dir_all(wt.parent().unwrap()).unwrap();
        g(&repo, &["init", "-q", "-b", "main"]);
        // No user/system config (GIT_CONFIG_*=/dev/null in `g`) and no hooks.
        for (k, v) in [
            ("user.email", "t@example.com"),
            ("user.name", "t"),
            ("commit.gpgsign", "false"),
            ("core.hooksPath", "/dev/null"),
        ] {
            g(&repo, &["config", k, v]);
        }
        commit_file(&repo, "f.txt", "base\n", "base");
        g(
            &repo,
            &[
                "worktree",
                "add",
                "-q",
                "-b",
                "garage/x",
                wt.to_str().unwrap(),
            ],
        );
        Fixture {
            _root: root,
            scratch,
            repo,
            garage,
            wt,
        }
    }

    fn s(p: &Path) -> &str {
        p.to_str().unwrap()
    }

    #[tokio::test]
    async fn current_branch_returns_the_checked_out_branch_null_for_a_non_repo() {
        let f = fixture();
        assert_eq!(current_branch(s(&f.repo)).await.as_deref(), Some("main"));
        assert_eq!(current_branch(s(&f.scratch)).await, None);
    }

    #[tokio::test]
    async fn dirty_worktree_refuses_merge_with_409() {
        let f = fixture();
        std::fs::write(f.wt.join("untracked.txt"), "x\n").unwrap();
        let e = finish_worktree(
            &f.garage,
            s(&f.repo),
            s(&f.wt),
            "garage/x",
            FinishAction::Merge,
        )
        .await
        .unwrap_err();
        assert_eq!(e.status, 409);
        assert!(e.message.contains("uncommitted"), "{}", e.message);
        assert!(!f.repo.join(".git/MERGE_HEAD").exists());
    }

    #[tokio::test]
    async fn merge_conflict_is_409_with_stderr_merge_aborted_worktree_and_branch_kept() {
        let f = fixture();
        commit_file(&f.wt, "f.txt", "from branch\n", "branch change");
        commit_file(&f.repo, "f.txt", "from main\n", "main change");
        let e = finish_worktree(
            &f.garage,
            s(&f.repo),
            s(&f.wt),
            "garage/x",
            FinishAction::Merge,
        )
        .await
        .unwrap_err();
        assert_eq!(e.status, 409);
        assert!(e.stderr.is_some(), "stderr is surfaced as a string");
        assert!(
            !f.repo.join(".git/MERGE_HEAD").exists(),
            "no merge in progress"
        );
        assert_eq!(g(&f.repo, &["status", "--porcelain"]).trim(), "");
        assert!(f.wt.exists());
        assert!(g(&f.repo, &["branch", "--list", "garage/x"]).contains("garage/x"));
    }

    #[tokio::test]
    async fn clean_merge_lands_a_merge_commit_and_removes_worktree_and_branch() {
        let f = fixture();
        commit_file(&f.wt, "new.txt", "hello\n", "add new");
        let r = finish_worktree(
            &f.garage,
            s(&f.repo),
            s(&f.wt),
            "garage/x",
            FinishAction::Merge,
        )
        .await
        .unwrap();
        assert_eq!((r.merged, r.discarded), (Some(true), None));
        assert!(f.repo.join("new.txt").exists(), "merged into main");
        assert!(
            g(&f.repo, &["log", "-1", "--format=%p"])
                .trim()
                .contains(' '),
            "--no-ff: a two-parent commit"
        );
        assert!(!f.wt.exists());
        assert_eq!(g(&f.repo, &["branch", "--list", "garage/x"]).trim(), "");
    }

    #[tokio::test]
    async fn discard_force_removes_a_dirty_worktree_and_an_unmerged_branch() {
        let f = fixture();
        commit_file(&f.wt, "unmerged.txt", "x\n", "unmerged work");
        std::fs::write(f.wt.join("dirty.txt"), "x\n").unwrap();
        let r = finish_worktree(
            &f.garage,
            s(&f.repo),
            s(&f.wt),
            "garage/x",
            FinishAction::Discard,
        )
        .await
        .unwrap();
        assert_eq!((r.merged, r.discarded), (None, Some(true)));
        assert!(!f.wt.exists());
        assert_eq!(g(&f.repo, &["branch", "--list", "garage/x"]).trim(), "");
    }

    #[tokio::test]
    async fn keep_changes_nothing_and_is_not_branch_checked() {
        let f = fixture();
        let r = finish_worktree(&f.garage, s(&f.repo), s(&f.wt), "main", FinishAction::Keep)
            .await
            .unwrap();
        assert_eq!(
            (r.action, r.merged, r.discarded),
            (FinishAction::Keep, None, None)
        );
        assert!(f.wt.exists());
    }

    #[tokio::test]
    async fn merge_and_discard_refuse_a_non_garage_branch_with_400_and_touch_nothing() {
        let f = fixture();
        for action in [FinishAction::Merge, FinishAction::Discard] {
            let e = finish_worktree(&f.garage, s(&f.repo), s(&f.wt), "main", action)
                .await
                .unwrap_err();
            assert_eq!(e.status, 400);
            assert_eq!(e.message, "refusing to touch non-garage branch: main");
        }
        assert!(f.wt.exists());
        assert_eq!(current_branch(s(&f.repo)).await.as_deref(), Some("main"));
    }

    #[tokio::test]
    async fn a_forged_path_or_a_mismatched_branch_is_400_and_deletes_nothing() {
        let f = fixture();
        // A real, dirty checkout that is not under <garage>/worktrees: the main repo itself.
        std::fs::write(f.repo.join("precious.txt"), "keep me\n").unwrap();
        // A garage worktree whose HEAD is not the claimed branch.
        g(&f.wt, &["checkout", "-q", "-b", "garage/other"]);
        for action in [FinishAction::Discard, FinishAction::Merge] {
            for (path, branch) in [
                (s(&f.repo), "garage/x"),
                ("/nonexistent/garage/wt", "garage/x"),
            ] {
                let e = finish_worktree(&f.garage, s(&f.repo), path, branch, action)
                    .await
                    .unwrap_err();
                assert_eq!(e.status, 400, "{action:?} {path}");
            }
        }
        // Merge also refuses the worktree whose HEAD is not the claimed branch.
        let e = finish_worktree(
            &f.garage,
            s(&f.repo),
            s(&f.wt),
            "garage/x",
            FinishAction::Merge,
        )
        .await
        .unwrap_err();
        assert_eq!(e.status, 400);
        assert!(f.repo.join("precious.txt").exists());
        assert!(f.wt.exists());
        assert!(g(&f.repo, &["branch", "--list", "garage/x"]).contains("garage/x"));
    }

    #[tokio::test]
    async fn discard_succeeds_after_the_agent_checked_out_another_branch() {
        let f = fixture();
        g(&f.wt, &["checkout", "-q", "-b", "garage/elsewhere"]);
        let r = finish_worktree(
            &f.garage,
            s(&f.repo),
            s(&f.wt),
            "garage/x",
            FinishAction::Discard,
        )
        .await
        .unwrap();
        assert_eq!(r.discarded, Some(true));
        assert!(!f.wt.exists());
        assert_eq!(g(&f.repo, &["branch", "--list", "garage/x"]).trim(), "");
        // The agent's other branch is not ours to delete.
        assert!(g(&f.repo, &["branch", "--list", "garage/elsewhere"]).contains("garage/elsewhere"));
    }

    #[tokio::test]
    async fn discard_succeeds_on_a_detached_head() {
        let f = fixture();
        g(&f.wt, &["checkout", "-q", "--detach"]);
        finish_worktree(
            &f.garage,
            s(&f.repo),
            s(&f.wt),
            "garage/x",
            FinishAction::Discard,
        )
        .await
        .unwrap();
        assert!(!f.wt.exists());
        assert_eq!(g(&f.repo, &["branch", "--list", "garage/x"]).trim(), "");
    }

    #[tokio::test]
    async fn discard_with_the_folder_gone_and_the_branch_already_deleted_is_ok() {
        let f = fixture();
        std::fs::remove_dir_all(&f.wt).unwrap();
        g(&f.repo, &["worktree", "prune"]);
        g(&f.repo, &["branch", "-D", "garage/x"]);
        finish_worktree(
            &f.garage,
            s(&f.repo),
            s(&f.wt),
            "garage/x",
            FinishAction::Discard,
        )
        .await
        .unwrap();
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn a_symlink_or_dotdot_escape_out_of_the_worktrees_folder_is_400_and_deletes_nothing() {
        let f = fixture();
        // A real worktree outside the garage folder, reached through a symlink
        // that sits inside it, and through a `..` path.
        let outside = f.scratch.join("outside");
        g(
            &f.repo,
            &["worktree", "add", "-q", "-b", "garage/out", s(&outside)],
        );
        let link = f.garage.join("worktrees/ws/link");
        std::os::unix::fs::symlink(&outside, &link).unwrap();
        let dotdot = format!("{}/../../../../outside", s(&f.wt));
        for action in [FinishAction::Discard, FinishAction::Merge] {
            for path in [s(&link), dotdot.as_str()] {
                let e = finish_worktree(&f.garage, s(&f.repo), path, "garage/out", action)
                    .await
                    .unwrap_err();
                assert_eq!(e.status, 400, "{action:?} {path}");
            }
        }
        assert!(outside.exists());
        assert!(g(&f.repo, &["branch", "--list", "garage/out"]).contains("garage/out"));
    }

    #[tokio::test]
    async fn discarding_a_worktree_whose_directory_is_gone_still_removes_the_branch() {
        let f = fixture();
        std::fs::remove_dir_all(&f.wt).unwrap();
        let wt = s(&f.wt).to_owned();
        let wt_branch = "garage/x".to_owned();
        // a `..` escape out of the worktrees root is still refused
        let escape = format!("{}/../../etc/nothing", s(&f.garage.join("worktrees")));
        let e = finish_worktree(
            &f.garage,
            s(&f.repo),
            &escape,
            &wt_branch,
            FinishAction::Discard,
        )
        .await
        .unwrap_err();
        assert_eq!(e.status, 400);
        finish_worktree(
            &f.garage,
            s(&f.repo),
            &wt,
            &wt_branch,
            FinishAction::Discard,
        )
        .await
        .unwrap();
        assert_eq!(g(&f.repo, &["branch", "--list", &wt_branch]).trim(), "");
    }

    #[tokio::test]
    async fn remove_worktree_undoes_a_created_worktree_and_branch() {
        let f = fixture();
        let garage = f.scratch.join("g2");
        let w = create_worktree(&garage, s(&f.repo), "ws", "undo")
            .await
            .unwrap();
        remove_worktree(s(&f.repo), &w.path, &w.branch).await;
        assert!(!Path::new(&w.path).exists());
        assert_eq!(g(&f.repo, &["branch", "--list", "garage/undo"]).trim(), "");
    }

    #[tokio::test]
    async fn create_worktree_suffixes_dir_and_branch_together_on_collision() {
        let f = fixture();
        let garage = f.scratch.join("garage");
        let a = create_worktree(&garage, s(&f.repo), "ws", "feat")
            .await
            .unwrap();
        assert_eq!(a.branch, "garage/feat");
        assert_eq!(a.path, garage.join("worktrees/ws/feat").to_string_lossy());
        assert_eq!(
            current_branch(&a.path).await.as_deref(),
            Some("garage/feat")
        );
        let b = create_worktree(&garage, s(&f.repo), "ws", "feat")
            .await
            .unwrap();
        assert_eq!(b.branch, "garage/feat-2");
        assert!(b.path.ends_with("worktrees/ws/feat-2"));
        // A leftover branch alone also forces the next suffix.
        g(&f.repo, &["branch", "garage/other"]);
        let c = create_worktree(&garage, s(&f.repo), "ws", "other")
            .await
            .unwrap();
        assert_eq!(c.branch, "garage/other-2");
    }

    #[tokio::test]
    async fn create_worktree_in_a_non_repo_is_400_and_creates_nothing() {
        let f = fixture();
        let garage = f.scratch.join("fresh-garage");
        let e = create_worktree(&garage, s(&f.scratch), "ws", "feat")
            .await
            .unwrap_err();
        assert_eq!(e.status, 400);
        assert_eq!(
            e.message,
            format!("not a git repository: {}", s(&f.scratch))
        );
        assert!(!garage.exists());
    }
}
