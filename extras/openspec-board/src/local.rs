//! The local collector: the OpenSpec changes in a repo's working tree.
//! Read-only: it lists folders, reads `proposal.md` and `tasks.md`, and runs
//! `git symbolic-ref`, `rev-parse` and `ls-tree` (reads) to find the names
//! already archived on the default branch. It writes nothing in any repo.

use std::collections::{BTreeSet, HashMap};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::SystemTime;

use crate::parser::{State, Tasks, parse_tasks};

/// A `tasks.md` or `proposal.md` larger than this is skipped, and never read
/// past it.
pub const MAX_FILE_BYTES: u64 = 1024 * 1024;
/// Directory entries under `openspec/changes` looked at per repo.
pub const MAX_CHANGES: usize = 200;

/// One change of one repo.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Row {
    pub name: String,
    /// The repo folder's name.
    pub repo: String,
    /// Where it lives: `local` for the working tree, `origin/<branch>` for each
    /// remote branch that holds it.
    pub places: Vec<String>,
    pub state: State,
    pub done: usize,
    pub total: usize,
    pub next: Option<String>,
    /// Every unchecked task, for the detail view.
    pub open: Vec<String>,
    /// The sources of this change disagree on its task counts.
    pub disagree: bool,
}

fn dir_not_link(p: &Path) -> bool {
    std::fs::symlink_metadata(p).is_ok_and(|m| m.file_type().is_dir())
}

/// A regular file of at most [`MAX_FILE_BYTES`] with its mtime and size.
/// Symlinks (which could point out of the repo), directories, devices and
/// oversized files give `None`; the read is capped too, in case the file
/// grows between the check and the read.
fn read_small_file(path: &Path) -> Option<(String, (SystemTime, u64))> {
    let meta = std::fs::symlink_metadata(path).ok()?;
    if !meta.file_type().is_file() || meta.len() > MAX_FILE_BYTES {
        return None;
    }
    let stamp = (meta.modified().ok()?, meta.len());
    let mut bytes = Vec::new();
    std::fs::File::open(path)
        .ok()?
        .take(MAX_FILE_BYTES + 1)
        .read_to_end(&mut bytes)
        .ok()?;
    if bytes.len() as u64 > MAX_FILE_BYTES {
        return None;
    }
    Some((String::from_utf8_lossy(&bytes).into_owned(), stamp))
}

fn stamp_of(path: &Path) -> Option<(SystemTime, u64)> {
    let meta = std::fs::symlink_metadata(path).ok()?;
    meta.file_type()
        .is_file()
        .then(|| Some((meta.modified().ok()?, meta.len())))?
}

pub(crate) fn git(repo: &Path, args: &[&str]) -> Option<String> {
    let out = Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(args)
        .env("GIT_OPTIONAL_LOCKS", "0")
        .env("GIT_TERMINAL_PROMPT", "0")
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .ok()?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).into_owned())
}

/// `refs/remotes/origin/main` -> `origin/main`; else the first of `main`,
/// `master`, `origin/main`, `origin/master` that exists.
pub fn default_branch(repo: &Path) -> Option<String> {
    if let Some(head) = git(repo, &["symbolic-ref", "-q", "refs/remotes/origin/HEAD"])
        && let Some(short) = head.trim().strip_prefix("refs/remotes/")
        && git(repo, &["rev-parse", "--verify", "-q", short]).is_some()
    {
        return Some(short.to_owned());
    }
    ["main", "master", "origin/main", "origin/master"]
        .into_iter()
        .find(|b| git(repo, &["rev-parse", "--verify", "-q", b]).is_some())
        .map(str::to_owned)
}

/// `2026-11-02-old` -> `old`.
fn strip_date(folder: &str) -> &str {
    let b = folder.as_bytes();
    let dated = b.len() > 11
        && b[..10].iter().enumerate().all(|(i, c)| match i {
            4 | 7 => *c == b'-',
            _ => c.is_ascii_digit(),
        })
        && b[10] == b'-';
    if dated { &folder[11..] } else { folder }
}

/// Names of the changes already archived on the default branch (both the
/// folder name and the name without its date prefix).
pub fn archived_names(repo: &Path) -> BTreeSet<String> {
    let mut names = BTreeSet::new();
    let Some(branch) = default_branch(repo) else {
        return names;
    };
    let tree = format!("{branch}:openspec/changes/archive");
    let Some(out) = git(repo, &["ls-tree", "--name-only", &tree]) else {
        return names;
    };
    for folder in out.lines().filter(|l| !l.is_empty()) {
        names.insert(folder.to_owned());
        names.insert(strip_date(folder).to_owned());
    }
    names
}

#[derive(Clone)]
struct Cached {
    tasks_stamp: Option<(SystemTime, u64)>,
    proposal_stamp: Option<(SystemTime, u64)>,
    tasks: Tasks,
}

/// Collects the changes of repos and remembers what it read: a file is read
/// again only when its mtime or size changed.
#[derive(Default)]
pub struct Collector {
    cache: HashMap<PathBuf, Cached>,
}

impl Collector {
    pub fn new() -> Self {
        Self::default()
    }

    /// The active changes in `repo`'s working tree, sorted by name.
    pub fn collect_repo(&mut self, repo: &Path) -> Vec<Row> {
        let base = repo.join("openspec/changes");
        if !dir_not_link(&base) {
            return vec![];
        }
        let Ok(rd) = std::fs::read_dir(&base) else {
            return vec![];
        };
        let archived = archived_names(repo);
        let repo_name = repo
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| repo.display().to_string());
        let mut rows = vec![];
        for e in rd.flatten().take(MAX_CHANGES) {
            let name = e.file_name().to_string_lossy().into_owned();
            if name == "archive" || name.starts_with('.') || archived.contains(&name) {
                continue;
            }
            let dir = e.path();
            if !dir_not_link(&dir) {
                continue;
            }
            let Some(c) = self.read_change(&dir) else {
                continue;
            };
            rows.push(Row {
                name,
                repo: repo_name.clone(),
                places: vec!["local".into()],
                state: State::of(c.tasks.done, c.tasks.total),
                done: c.tasks.done,
                total: c.tasks.total,
                next: c.tasks.next.as_ref().map(|n| n.label()),
                open: c.tasks.open,
                disagree: false,
            });
        }
        rows.sort_by(|a, b| a.name.cmp(&b.name));
        // Forget what is no longer on disk.
        self.cache.retain(|p, _| p.exists());
        rows
    }

    /// `None` when the folder holds neither `proposal.md` nor `tasks.md`.
    fn read_change(&mut self, dir: &Path) -> Option<Cached> {
        let (tasks_path, proposal_path) = (dir.join("tasks.md"), dir.join("proposal.md"));
        let (tasks_stamp, proposal_stamp) = (stamp_of(&tasks_path), stamp_of(&proposal_path));
        if tasks_stamp.is_none() && proposal_stamp.is_none() {
            return None;
        }
        if let Some(hit) = self.cache.get(dir)
            && hit.tasks_stamp == tasks_stamp
            && hit.proposal_stamp == proposal_stamp
        {
            return Some(hit.clone());
        }
        let tasks_text = read_small_file(&tasks_path);
        // proposal.md only has to exist in this group; its text is not shown.
        let fresh = Cached {
            tasks_stamp,
            proposal_stamp,
            tasks: tasks_text.map(|(t, _)| parse_tasks(&t)).unwrap_or_default(),
        };
        self.cache.insert(dir.to_path_buf(), fresh.clone());
        Some(fresh)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn git_in(dir: &Path, args: &[&str]) {
        let st = Command::new("git")
            .arg("-C")
            .arg(dir)
            .args([
                "-c",
                "user.email=t@t",
                "-c",
                "user.name=t",
                "-c",
                "commit.gpgsign=false",
            ])
            .args(args)
            .stdout(Stdio::null())
            .status()
            .unwrap();
        assert!(st.success(), "git {args:?}");
    }

    fn init(dir: &Path) {
        std::fs::create_dir_all(dir).unwrap();
        git_in(dir, &["init", "-q", "-b", "main"]);
    }

    fn change(repo: &Path, name: &str, tasks: Option<&str>, proposal: bool) {
        let d = repo.join("openspec/changes").join(name);
        std::fs::create_dir_all(&d).unwrap();
        if let Some(t) = tasks {
            std::fs::write(d.join("tasks.md"), t).unwrap();
        }
        if proposal {
            std::fs::write(d.join("proposal.md"), "## Why\n").unwrap();
        }
    }

    fn summary(rows: &[Row]) -> Vec<(String, State, usize, usize)> {
        rows.iter()
            .map(|r| (r.name.clone(), r.state, r.done, r.total))
            .collect()
    }

    #[test]
    fn three_changes_one_in_each_state_with_next_open_task() {
        let d = tempfile::tempdir().unwrap();
        init(d.path());
        change(d.path(), "a-proposal", None, true);
        change(
            d.path(),
            "b-building",
            Some("- [x] 1.1 a\n- [ ] 2.1 debounce\n"),
            true,
        );
        change(
            d.path(),
            "c-complete",
            Some("- [x] 1.1 a\n- [x] 1.2 b\n"),
            true,
        );
        let rows = Collector::new().collect_repo(d.path());
        assert_eq!(
            summary(&rows),
            [
                ("a-proposal".into(), State::Proposal, 0, 0),
                ("b-building".into(), State::Building, 1, 2),
                ("c-complete".into(), State::Complete, 2, 2),
            ]
        );
        assert_eq!(rows[1].next.as_deref(), Some("2.1 debounce"));
        assert_eq!(rows[0].next, None);
        assert_eq!(rows[2].next, None);
        assert_eq!(rows[0].places, ["local"]);
        assert_eq!(
            rows[0].repo,
            d.path().file_name().unwrap().to_str().unwrap()
        );
    }

    #[test]
    fn proposal_only_zero_n_and_tasks_only_folders_are_listed_but_empty_ones_are_not() {
        let d = tempfile::tempdir().unwrap();
        init(d.path());
        change(d.path(), "zero-n", Some("- [ ] a\n- [ ] b\n"), false);
        change(d.path(), "empty-dir", None, false);
        change(d.path(), ".hidden", Some("- [ ] x\n"), true);
        std::fs::write(d.path().join("openspec/changes/stray.md"), "- [ ] x").unwrap();
        let rows = Collector::new().collect_repo(d.path());
        assert_eq!(summary(&rows), [("zero-n".into(), State::Proposal, 0, 2)]);
    }

    #[test]
    fn archive_folder_and_archived_names_are_skipped() {
        let d = tempfile::tempdir().unwrap();
        init(d.path());
        change(d.path(), "archive/2026-11-02-old", Some("- [x] a\n"), true);
        change(d.path(), "kept", Some("- [ ] a\n"), true);
        git_in(d.path(), &["add", "-A"]);
        git_in(d.path(), &["commit", "-q", "-m", "archive old"]);
        // A stale local copy of the archived change reappears in the tree.
        change(d.path(), "old", Some("- [ ] stale\n"), true);
        let rows = Collector::new().collect_repo(d.path());
        assert_eq!(summary(&rows), [("kept".into(), State::Proposal, 0, 1)]);
        assert!(archived_names(d.path()).contains("old"));
        assert_eq!(strip_date("2026-11-02-old"), "old");
        assert_eq!(strip_date("not-a-date"), "not-a-date");
    }

    #[test]
    fn a_repo_without_commits_or_openspec_gives_no_rows_and_no_panic() {
        let d = tempfile::tempdir().unwrap();
        init(d.path());
        assert!(Collector::new().collect_repo(d.path()).is_empty());
        assert!(archived_names(d.path()).is_empty());
        assert!(
            Collector::new()
                .collect_repo(&d.path().join("nowhere"))
                .is_empty()
        );
    }

    #[test]
    fn an_edit_shows_on_the_next_pass_and_an_unchanged_file_is_served_from_the_cache() {
        let d = tempfile::tempdir().unwrap();
        init(d.path());
        change(d.path(), "x", Some("- [ ] 1.1 a\n- [ ] 1.2 b\n"), true);
        let mut c = Collector::new();
        assert_eq!(c.collect_repo(d.path())[0].done, 0);
        assert_eq!(c.cache.len(), 1);
        let again = c.collect_repo(d.path());
        assert_eq!(again[0].done, 0);
        // Tick a box (a different length changes the stamp even within one mtime tick).
        std::fs::write(
            d.path().join("openspec/changes/x/tasks.md"),
            "- [x] 1.1 a\n- [ ] 1.2 b\n",
        )
        .unwrap();
        let after = c.collect_repo(d.path());
        assert_eq!((after[0].done, after[0].total), (1, 2));
        assert_eq!(after[0].next.as_deref(), Some("1.2 b"));
    }

    #[cfg(unix)]
    #[test]
    fn a_read_only_repo_directory_is_read_and_nothing_is_written() {
        use std::os::unix::fs::PermissionsExt;
        let d = tempfile::tempdir().unwrap();
        init(d.path());
        change(d.path(), "ro", Some("- [x] a\n- [ ] b\n"), true);
        git_in(d.path(), &["add", "-A"]);
        git_in(d.path(), &["commit", "-q", "-m", "init"]);
        let snapshot = |p: &Path| {
            let out = Command::new("git")
                .arg("-C")
                .arg(p)
                .args(["status", "--porcelain"])
                .output()
                .unwrap();
            String::from_utf8(out.stdout).unwrap()
        };
        let before = snapshot(d.path());
        // Make every directory under openspec/ and the repo root read-only.
        let dirs = [
            d.path().to_path_buf(),
            d.path().join("openspec"),
            d.path().join("openspec/changes"),
            d.path().join("openspec/changes/ro"),
        ];
        for p in &dirs {
            std::fs::set_permissions(p, std::fs::Permissions::from_mode(0o555)).unwrap();
        }
        let rows = Collector::new().collect_repo(d.path());
        let listing = std::fs::read_dir(d.path()).unwrap().count();
        for p in dirs.iter().rev() {
            std::fs::set_permissions(p, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        assert_eq!(summary(&rows), [("ro".into(), State::Building, 1, 2)]);
        assert_eq!(snapshot(d.path()), before);
        assert_eq!(listing, std::fs::read_dir(d.path()).unwrap().count());
    }

    #[cfg(unix)]
    #[test]
    fn symlinked_tasks_and_folders_are_not_followed_out_of_the_repo() {
        let outside = tempfile::tempdir().unwrap();
        std::fs::write(outside.path().join("secret.md"), "- [x] leaked\n").unwrap();
        std::fs::create_dir_all(outside.path().join("elsewhere")).unwrap();
        std::fs::write(outside.path().join("elsewhere/tasks.md"), "- [x] a\n").unwrap();
        let d = tempfile::tempdir().unwrap();
        init(d.path());
        let base = d.path().join("openspec/changes");
        std::fs::create_dir_all(base.join("linked-file")).unwrap();
        std::os::unix::fs::symlink(
            outside.path().join("secret.md"),
            base.join("linked-file/tasks.md"),
        )
        .unwrap();
        std::os::unix::fs::symlink(outside.path().join("elsewhere"), base.join("linked-dir"))
            .unwrap();
        change(d.path(), "real", Some("- [ ] a\n"), true);
        let rows = Collector::new().collect_repo(d.path());
        assert_eq!(summary(&rows), [("real".into(), State::Proposal, 0, 1)]);
    }

    #[test]
    fn a_huge_tasks_file_is_skipped_as_zero_tasks() {
        let d = tempfile::tempdir().unwrap();
        init(d.path());
        let line = "- [x] task\n";
        let big = line.repeat(MAX_FILE_BYTES as usize / line.len() + 1);
        change(d.path(), "big", Some(&big), true);
        let rows = Collector::new().collect_repo(d.path());
        assert_eq!(summary(&rows), [("big".into(), State::Proposal, 0, 0)]);
    }
}
