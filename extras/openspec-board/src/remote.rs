//! Changes that live on remote branches, read without touching the checkout.
//!
//! Only `git for-each-ref`, `rev-parse`, `ls-tree`, `cat-file -s` and `show`
//! run here, on `refs/remotes/origin/*`: no checkout, no worktree, no write.
//! The fetch that brings the refs up to date is in [`crate::fetch`].

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::{Path, PathBuf};

use crate::fetch::FetchState;
use crate::local::{MAX_CHANGES, MAX_FILE_BYTES, Row, archived_names, default_branch, git};
use crate::parser::{State, Tasks, parse_tasks};

/// Without a `branches` list, branches older than this are not scanned.
pub const RECENT_DAYS: i64 = 30;
/// Branches read per repo, newest first.
pub const MAX_BRANCHES: usize = 200;

/// `*` matches any run of characters (slashes included).
pub fn glob_match(pattern: &str, text: &str) -> bool {
    let (p, t): (Vec<char>, Vec<char>) = (pattern.chars().collect(), text.chars().collect());
    let (mut pi, mut ti, mut star, mut mark) = (0, 0, None, 0);
    while ti < t.len() {
        if pi < p.len() && p[pi] == '*' {
            star = Some(pi);
            mark = ti;
            pi += 1;
        } else if pi < p.len() && p[pi] == t[ti] {
            pi += 1;
            ti += 1;
        } else if let Some(s) = star {
            pi = s + 1;
            mark += 1;
            ti = mark;
        } else {
            return false;
        }
    }
    p[pi..].iter().all(|c| *c == '*')
}

/// The remote-tracking branches worth reading, as `origin/<name>`, newest
/// first: matching `globs` when given, else with a commit in the last
/// [`RECENT_DAYS`]. `origin/HEAD` and the default branch are left out.
pub fn candidates(
    repo: &Path,
    globs: &[String],
    now_unix: i64,
    default: Option<&str>,
) -> Vec<String> {
    let Some(out) = git(
        repo,
        &[
            "for-each-ref",
            "--format=%(refname) %(committerdate:unix)",
            "refs/remotes/origin",
        ],
    ) else {
        return vec![];
    };
    let mut found: Vec<(i64, String)> = vec![];
    for line in out.lines() {
        let Some((full, ts)) = line.rsplit_once(' ') else {
            continue;
        };
        let Some(short) = full.strip_prefix("refs/remotes/") else {
            continue;
        };
        let Some(branch) = short.strip_prefix("origin/") else {
            continue;
        };
        if branch == "HEAD" || Some(short) == default {
            continue;
        }
        let ts: i64 = ts.parse().unwrap_or(0);
        let wanted = if globs.is_empty() {
            now_unix - ts <= RECENT_DAYS * 86_400
        } else {
            globs.iter().any(|g| glob_match(g, branch))
        };
        if wanted {
            found.push((ts, short.to_owned()));
        }
    }
    found.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(&b.1)));
    found.into_iter().take(MAX_BRANCHES).map(|f| f.1).collect()
}

/// The tree id of `openspec/changes` at `rev`, if it has one.
fn changes_tree(repo: &Path, rev: &str) -> Option<String> {
    let id = git(
        repo,
        &[
            "rev-parse",
            "--verify",
            "-q",
            &format!("{rev}:openspec/changes"),
        ],
    )?;
    Some(id.trim().to_owned()).filter(|s| !s.is_empty())
}

/// One change as read from a branch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Found {
    pub name: String,
    pub tasks: Tasks,
}

/// Reads and remembers what the remote branches hold. Results are cached by
/// the tree id of `openspec/changes`, so an unchanged branch costs one
/// `rev-parse`.
#[derive(Default)]
pub struct Remote {
    /// When each repo was last fetched (throttle and `fetch failed`).
    pub fetch: FetchState,
    cache: HashMap<(PathBuf, String), Vec<Found>>,
    /// How many `git show` runs there have been (for tests and diagnosis).
    pub shows: usize,
}

impl Remote {
    pub fn new() -> Self {
        Self::default()
    }

    /// The changes on the remote branches of `repo`, one row per change per
    /// branch (`places` is `["origin/<branch>"]`), minus names archived on the
    /// default branch. Branches whose `openspec/changes` tree equals the
    /// default branch's are skipped without reading a file.
    pub fn collect_repo(&mut self, repo: &Path, globs: &[String], now_unix: i64) -> Vec<Row> {
        let default = default_branch(repo);
        let default_tree = default.as_deref().and_then(|d| changes_tree(repo, d));
        let archived = archived_names(repo);
        let repo_name = repo
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| repo.display().to_string());
        let mut rows = vec![];
        let mut touched: BTreeSet<String> = BTreeSet::new();
        for branch in candidates(repo, globs, now_unix, default.as_deref()) {
            let Some(tree) = changes_tree(repo, &branch) else {
                continue;
            };
            if Some(&tree) == default_tree.as_ref() {
                continue;
            }
            let key = (repo.to_path_buf(), tree.clone());
            if !self.cache.contains_key(&key) {
                let read = self.read_branch(repo, &branch);
                self.cache.insert(key.clone(), read);
            }
            touched.insert(tree);
            for f in self.cache.get(&key).into_iter().flatten() {
                if archived.contains(&f.name) {
                    continue;
                }
                rows.push(Row {
                    name: f.name.clone(),
                    repo: repo_name.clone(),
                    places: vec![branch.clone()],
                    state: State::of(f.tasks.done, f.tasks.total),
                    done: f.tasks.done,
                    total: f.tasks.total,
                    next: f.tasks.next.as_ref().map(|n| n.label()),
                    open: f.tasks.open.clone(),
                    disagree: false,
                });
            }
        }
        self.cache
            .retain(|(r, tree), _| r.as_path() != repo || touched.contains(tree));
        rows
    }

    /// `git ls-tree` then `git show` of each `tasks.md`; a change holding
    /// neither `proposal.md` nor `tasks.md` is not a change.
    fn read_branch(&mut self, repo: &Path, branch: &str) -> Vec<Found> {
        let Some(listing) = git(
            repo,
            &[
                "ls-tree",
                "-r",
                "-z",
                "--name-only",
                branch,
                "openspec/changes/",
            ],
        ) else {
            return vec![];
        };
        // name -> has tasks.md
        let mut names: BTreeMap<&str, bool> = BTreeMap::new();
        for path in listing.split('\0') {
            let Some(rest) = path.strip_prefix("openspec/changes/") else {
                continue;
            };
            let mut parts = rest.split('/');
            let (Some(name), Some(file), None) = (parts.next(), parts.next(), parts.next()) else {
                continue;
            };
            if name == "archive" || name.starts_with('.') {
                continue;
            }
            match file {
                "tasks.md" => {
                    names.insert(name, true);
                }
                "proposal.md" => {
                    names.entry(name).or_insert(false);
                }
                _ => {}
            }
        }
        let mut out = vec![];
        for (name, has_tasks) in names.into_iter().take(MAX_CHANGES) {
            let tasks = if has_tasks {
                self.show_tasks(repo, branch, name)
            } else {
                Tasks::default()
            };
            out.push(Found {
                name: name.to_owned(),
                tasks,
            });
        }
        out
    }

    fn show_tasks(&mut self, repo: &Path, branch: &str, name: &str) -> Tasks {
        let spec = format!("{branch}:openspec/changes/{name}/tasks.md");
        let small = git(repo, &["cat-file", "-s", &spec])
            .and_then(|s| s.trim().parse::<u64>().ok())
            .is_some_and(|n| n <= MAX_FILE_BYTES);
        if !small {
            return Tasks::default();
        }
        self.shows += 1;
        git(repo, &["show", &spec])
            .map(|t| parse_tasks(&t))
            .unwrap_or_default()
    }
}

/// One row per (repo, change) from the local rows and the branch rows. Values
/// come from the most advanced source (highest `done`; the first listed wins
/// a tie, and local is listed first). `places` lists every source; `disagree`
/// is set when a source's counts differ from the shown ones.
pub fn merge(local: Vec<Row>, remote: Vec<Row>) -> Vec<Row> {
    let mut groups: BTreeMap<(String, String), Vec<Row>> = BTreeMap::new();
    for r in local.into_iter().chain(remote) {
        groups
            .entry((r.name.clone(), r.repo.clone()))
            .or_default()
            .push(r);
    }
    groups
        .into_values()
        .filter_map(|group| {
            let mut best = group.first()?.clone();
            for r in &group[1..] {
                if r.done > best.done {
                    best = r.clone();
                }
            }
            let mut places: Vec<String> = vec![];
            for p in group.iter().flat_map(|r| r.places.iter()) {
                if !places.contains(p) {
                    places.push(p.clone());
                }
            }
            best.disagree = group
                .iter()
                .any(|r| (r.done, r.total) != (best.done, best.total));
            best.places = places;
            Some(best)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fetch;
    use std::process::{Command, Stdio};
    use std::time::{Duration, Instant};

    fn git_in(dir: &Path, args: &[&str]) -> String {
        let out = Command::new("git")
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
            .stderr(Stdio::null())
            .output()
            .unwrap();
        assert!(out.status.success(), "git {args:?}");
        String::from_utf8_lossy(&out.stdout).into_owned()
    }

    fn write(repo: &Path, rel: &str, text: &str) {
        let p = repo.join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, text).unwrap();
    }

    /// A bare origin with `main` holding `README`, and a clone of it.
    fn origin_and_clone(root: &Path) -> (PathBuf, PathBuf) {
        let (origin, clone) = (root.join("origin.git"), root.join("clone"));
        assert!(
            Command::new("git")
                .args(["init", "-q", "--bare", "-b", "main"])
                .arg(&origin)
                .status()
                .unwrap()
                .success()
        );
        let seed = root.join("seed");
        std::fs::create_dir_all(&seed).unwrap();
        git_in(&seed, &["init", "-q", "-b", "main"]);
        write(&seed, "README", "hi\n");
        git_in(&seed, &["add", "."]);
        git_in(&seed, &["commit", "-q", "-m", "init"]);
        git_in(&seed, &["push", "-q", origin.to_str().unwrap(), "main"]);
        assert!(
            Command::new("git")
                .args(["clone", "-q"])
                .arg(&origin)
                .arg(&clone)
                .status()
                .unwrap()
                .success()
        );
        (origin, clone)
    }

    /// Push a branch with `files` (relative path, text) from the seed.
    fn push_branch(root: &Path, origin: &Path, branch: &str, files: &[(&str, &str)]) {
        let seed = root.join("seed");
        git_in(&seed, &["checkout", "-q", "-B", branch, "main"]);
        for (p, t) in files {
            write(&seed, p, t);
        }
        git_in(&seed, &["add", "."]);
        git_in(&seed, &["commit", "-q", "-m", branch]);
        git_in(
            &seed,
            &["push", "-q", "-f", origin.to_str().unwrap(), branch],
        );
        git_in(&seed, &["checkout", "-q", "main"]);
    }

    fn fetch_now(clone: &Path) -> Result<(), String> {
        fetch::run_with(Path::new("git"), clone, Duration::from_secs(30))
    }

    fn now() -> i64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64
    }

    fn names(rows: &[Row]) -> Vec<(String, Vec<String>, usize, usize)> {
        rows.iter()
            .map(|r| (r.name.clone(), r.places.clone(), r.done, r.total))
            .collect()
    }

    /// Everything a read-only refresh must leave alone.
    fn fingerprint(clone: &Path) -> (String, String, String, Vec<String>) {
        let outside_remotes = git_in(
            clone,
            &["for-each-ref", "--format=%(refname) %(objectname)"],
        )
        .lines()
        .filter(|l| !l.starts_with("refs/remotes/"))
        .map(str::to_owned)
        .collect();
        (
            git_in(clone, &["status", "--porcelain"]),
            git_in(clone, &["rev-parse", "HEAD"]),
            git_in(clone, &["ls-files", "-s"]),
            outside_remotes,
        )
    }

    const TASKS: &str = "- [x] 1.1 a\n- [x] 1.2 b\n- [ ] 2.1 c\n- [ ] 2.2 d\n";

    #[test]
    fn a_change_only_on_an_integration_branch_is_listed_and_the_clone_is_untouched() {
        let d = tempfile::tempdir().unwrap();
        let (origin, clone) = origin_and_clone(d.path());
        push_branch(
            d.path(),
            &origin,
            "feat/billing-v2",
            &[
                ("openspec/changes/billing-v2/tasks.md", TASKS),
                ("openspec/changes/billing-v2/proposal.md", "## Why\n"),
            ],
        );
        // A file the user is editing: the refresh must not disturb it.
        write(&clone, "README", "edited\n");
        write(&clone, "scratch.txt", "untracked\n");
        let before = fingerprint(&clone);
        let readme = std::fs::read_to_string(clone.join("README")).unwrap();

        fetch_now(&clone).unwrap();
        let rows = Remote::new().collect_repo(&clone, &[], now());

        assert_eq!(
            names(&rows),
            [(
                "billing-v2".to_owned(),
                vec!["origin/feat/billing-v2".to_owned()],
                2,
                4
            )]
        );
        assert_eq!(rows[0].state, State::Building);
        assert_eq!(rows[0].next.as_deref(), Some("2.1 c"));
        assert_eq!(fingerprint(&clone), before);
        assert_eq!(
            std::fs::read_to_string(clone.join("README")).unwrap(),
            readme
        );
        assert!(!clone.join("openspec").exists(), "nothing was checked out");
        assert!(!clone.join(".git/FETCH_HEAD").exists());
    }

    #[test]
    fn a_branch_with_the_default_branchs_changes_tree_is_not_read() {
        let d = tempfile::tempdir().unwrap();
        let (origin, clone) = origin_and_clone(d.path());
        // main holds a change; the branch only adds an unrelated file, so its
        // openspec/changes tree is identical to main's.
        let seed = d.path().join("seed");
        write(&seed, "openspec/changes/same/tasks.md", "- [ ] 1.1 a\n");
        git_in(&seed, &["add", "."]);
        git_in(&seed, &["commit", "-q", "-m", "change"]);
        git_in(&seed, &["push", "-q", origin.to_str().unwrap(), "main"]);
        push_branch(d.path(), &origin, "feat/other", &[("src.txt", "x\n")]);
        push_branch(
            d.path(),
            &origin,
            "feat/real",
            &[("openspec/changes/real/tasks.md", "- [ ] 1.1 a\n")],
        );
        git_in(&clone, &["fetch", "-q", "origin"]);

        let mut remote = Remote::new();
        let rows = remote.collect_repo(&clone, &[], now());
        // `feat/other` has main's tree and was not read: only `feat/real`
        // was, with one `git show` for each of its two changes.
        assert_eq!(remote.shows, 2);
        assert!(
            rows.iter()
                .all(|r| r.places == ["origin/feat/real".to_owned()]),
            "{rows:?}"
        );
        // A second pass is served from the tree-id cache: no more `show`.
        remote.collect_repo(&clone, &[], now());
        assert_eq!(remote.shows, 2);
    }

    #[test]
    fn a_name_archived_on_the_default_branch_is_skipped_on_stale_branches() {
        let d = tempfile::tempdir().unwrap();
        let (origin, clone) = origin_and_clone(d.path());
        let seed = d.path().join("seed");
        write(
            &seed,
            "openspec/changes/archive/2026-11-02-old/tasks.md",
            "- [x] 1.1 a\n",
        );
        git_in(&seed, &["add", "."]);
        git_in(&seed, &["commit", "-q", "-m", "archive old"]);
        git_in(&seed, &["push", "-q", origin.to_str().unwrap(), "main"]);
        // A stale branch from before the archive still holds `old`.
        push_branch(
            d.path(),
            &origin,
            "feat/old",
            &[
                (
                    "openspec/changes/old/tasks.md",
                    "- [x] 1.1 a\n- [ ] 1.2 b\n",
                ),
                ("openspec/changes/live/tasks.md", "- [ ] 1.1 a\n"),
            ],
        );
        fetch_now(&clone).unwrap();
        let rows = Remote::new().collect_repo(&clone, &[], now());
        assert_eq!(
            names(&rows).into_iter().map(|r| r.0).collect::<Vec<_>>(),
            ["live"]
        );
    }

    #[test]
    fn when_the_fetch_fails_the_already_fetched_refs_are_still_listed() {
        let d = tempfile::tempdir().unwrap();
        let (origin, clone) = origin_and_clone(d.path());
        push_branch(
            d.path(),
            &origin,
            "feat/x",
            &[("openspec/changes/x/tasks.md", "- [x] 1.1 a\n- [ ] 1.2 b\n")],
        );
        fetch_now(&clone).unwrap();
        // The origin disappears.
        std::fs::remove_dir_all(&origin).unwrap();
        let mut state = fetch::FetchState::default();
        let t0 = Instant::now();
        let r = state.fetch(Path::new("git"), &clone, true, t0, Duration::from_secs(30));
        assert!(matches!(r, Some(Err(_))), "{r:?}");
        assert_eq!(state.failed_at(&clone), Some(t0));

        let rows = Remote::new().collect_repo(&clone, &[], now());
        assert_eq!(
            names(&rows),
            [("x".to_owned(), vec!["origin/feat/x".to_owned()], 1, 2)]
        );
    }

    fn row(name: &str, place: &str, done: usize, total: usize) -> Row {
        Row {
            name: name.into(),
            repo: "api".into(),
            places: vec![place.into()],
            state: State::of(done, total),
            done,
            total,
            next: None,
            open: vec![],
            disagree: false,
        }
    }

    #[test]
    fn sources_that_disagree_make_one_row_from_the_most_advanced_and_mark_it() {
        let merged = merge(
            vec![row("x", "local", 3, 10)],
            vec![
                row("x", "origin/feat/x", 6, 10),
                row("y", "origin/feat/y", 1, 2),
            ],
        );
        assert_eq!(merged.len(), 2);
        let x = &merged[0];
        assert_eq!((x.done, x.total), (6, 10));
        assert_eq!(x.places, ["local", "origin/feat/x"]);
        assert!(x.disagree);
        // A change seen in one place only is not marked.
        assert!(!merged[1].disagree);
        // Equal sources are one row, not marked, with every place.
        let same = merge(
            vec![row("z", "local", 2, 4)],
            vec![row("z", "origin/feat/z", 2, 4)],
        );
        assert_eq!(same.len(), 1);
        assert!(!same[0].disagree);
        assert_eq!(same[0].places, ["local", "origin/feat/z"]);
    }

    #[test]
    fn branches_are_filtered_by_glob_or_by_age_and_origin_head_is_never_one() {
        let d = tempfile::tempdir().unwrap();
        let (origin, clone) = origin_and_clone(d.path());
        push_branch(d.path(), &origin, "feat/a", &[("a", "1")]);
        push_branch(d.path(), &origin, "fix/b", &[("b", "1")]);
        fetch_now(&clone).unwrap();
        let all = candidates(&clone, &[], now(), Some("origin/main"));
        assert_eq!(all.len(), 2);
        assert!(all.iter().all(|b| b != "origin/HEAD" && b != "origin/main"));
        assert_eq!(
            candidates(&clone, &["feat/*".into()], now(), Some("origin/main")),
            ["origin/feat/a"]
        );
        // Everything is older than 30 days a year from now; a glob list is
        // not subject to the age filter.
        let later = now() + 365 * 86_400;
        assert!(candidates(&clone, &[], later, Some("origin/main")).is_empty());
        assert_eq!(
            candidates(&clone, &["fix/*".into()], later, Some("origin/main")),
            ["origin/fix/b"]
        );
        assert!(glob_match("feat/*", "feat/a/b") && !glob_match("feat/*", "fix/a"));
    }
}
