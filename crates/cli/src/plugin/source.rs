//! Where a plugin comes from: GitHub shorthand and URLs, the shallow clone
//! at a pinned commit, and the check that a checkout cannot reach outside
//! itself through symlinks.

use std::io::Read;
use std::path::{Component, Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

pub const GITHUB: &str = "https://github.com";
pub const CLONE_TIMEOUT: Duration = Duration::from_secs(60);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GithubSource {
    /// `owner/repo`
    pub repo: String,
    /// A folder inside the repo that holds `garage-plugin.toml`.
    pub subdir: Option<String>,
}

impl GithubSource {
    pub fn clone_url(&self, base: &str) -> String {
        format!("{}/{}", base.trim_end_matches('/'), self.repo)
    }

    pub fn display(&self) -> String {
        match &self.subdir {
            Some(s) => format!("{}/{s}", self.repo),
            None => self.repo.clone(),
        }
    }
}

fn name_ok(s: &str) -> bool {
    !s.is_empty() && s != "." && s != ".." && s.len() <= 100 && s.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))
}

/// `owner/repo`, `owner/repo/sub/dir`, `https://github.com/owner/repo[.git]`
/// and the same with a trailing `/sub/dir`.
pub fn parse_github(spec: &str) -> Result<GithubSource, String> {
    let spec = spec.trim();
    if spec.starts_with('.') || spec.starts_with('/') || spec.starts_with('~') || spec.contains('\\') || spec.chars().nth(1) == Some(':') {
        return Err(format!("{spec:?} looks like a local path; use `claude-garage plugin link {spec}` for a folder on this machine"));
    }
    let rest = if let Some(r) = spec.strip_prefix("https://github.com/") {
        r
    } else if spec.contains("://") || spec.starts_with("git@") {
        return Err(format!("{spec:?}: only GitHub is supported (owner/repo or https://github.com/owner/repo)"));
    } else {
        spec
    };
    let rest = rest.trim_end_matches('/');
    let mut parts = rest.split('/');
    let (Some(owner), Some(repo)) = (parts.next(), parts.next()) else {
        return Err(format!("{spec:?} is not owner/repo[/subdir]"));
    };
    let repo = repo.strip_suffix(".git").unwrap_or(repo);
    if !name_ok(owner) || !name_ok(repo) {
        return Err(format!("{spec:?} is not owner/repo[/subdir]"));
    }
    let sub: Vec<&str> = parts.collect();
    if sub.first() == Some(&"tree") || sub.first() == Some(&"blob") {
        return Err("a /tree/<ref> URL is ambiguous: use owner/repo/subdir with --ref <ref>".into());
    }
    let subdir = if sub.is_empty() {
        None
    } else {
        if !sub.iter().all(|c| name_ok(c)) {
            return Err(format!("subdir in {spec:?} must be plain folder names (no .., no empty parts)"));
        }
        Some(sub.join("/"))
    };
    Ok(GithubSource { repo: format!("{owner}/{repo}"), subdir })
}

/// Clone seam: a shallow checkout of `git_ref` (default branch when `None`)
/// into `dest`, with `.git` removed afterwards. Returns the commit sha.
pub trait Git {
    fn clone_shallow(&self, url: &str, git_ref: Option<&str>, dest: &Path) -> Result<String, String>;
}

pub struct RealGit;

/// Config that keeps a fetched repository from running anything on this
/// machine: no hooks, no fsmonitor, no submodules, no LFS or other
/// clean/smudge filters, and https as the only transport.
const HARDENING: &[&str] = &[
    "core.hooksPath=/dev/null",
    "core.fsmonitor=false",
    "submodule.recurse=false",
    "filter.lfs.process=",
    "filter.lfs.smudge=",
    "filter.lfs.required=false",
    "protocol.allow=never",
    "protocol.https.allow=always",
];

/// `git` with the hardening above, no system or user config, and no prompts.
fn git_command(args: &[&str], cwd: Option<&Path>) -> Command {
    let mut cmd = Command::new("git");
    for c in HARDENING {
        cmd.args(["-c", c]);
    }
    // Tests clone from local fixture repositories; the shipped binary never does.
    if cfg!(test) {
        cmd.args(["-c", "protocol.file.allow=always"]);
    }
    cmd.args(args)
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_LFS_SKIP_SMUDGE", "1")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", if cfg!(windows) { "NUL" } else { "/dev/null" })
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if let Some(c) = cwd {
        cmd.current_dir(c);
    }
    cmd
}

/// Run git with a deadline; stderr is kept for the error message.
fn run_git(args: &[&str], cwd: Option<&Path>, timeout: Duration) -> Result<String, String> {
    let mut cmd = git_command(args, cwd);
    let mut child = cmd.spawn().map_err(|e| format!("cannot run git: {e} (is git installed?)"))?;
    let (mut out, mut err) = (child.stdout.take().unwrap(), child.stderr.take().unwrap());
    let t_out = std::thread::spawn(move || {
        let mut s = String::new();
        let _ = out.read_to_string(&mut s);
        s
    });
    let t_err = std::thread::spawn(move || {
        let mut s = String::new();
        let _ = err.read_to_string(&mut s);
        s
    });
    let start = Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(s)) => break s,
            Ok(None) if start.elapsed() > timeout => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(format!("git timed out after {}s", timeout.as_secs()));
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(20)),
            Err(e) => return Err(format!("git: {e}")),
        }
    };
    let (stdout, stderr) = (t_out.join().unwrap_or_default(), t_err.join().unwrap_or_default());
    if status.success() {
        Ok(stdout)
    } else {
        Err(format!("git {} failed: {}", args.first().unwrap_or(&""), stderr.trim()))
    }
}

fn is_full_sha(s: &str) -> bool {
    s.len() == 40 && s.bytes().all(|b| b.is_ascii_hexdigit())
}

impl Git for RealGit {
    fn clone_shallow(&self, url: &str, git_ref: Option<&str>, dest: &Path) -> Result<String, String> {
        let d = dest.to_string_lossy();
        match git_ref {
            Some(r) if is_full_sha(r) => {
                run_git(&["init", "--quiet", &d], None, CLONE_TIMEOUT)?;
                run_git(&["fetch", "--quiet", "--depth", "1", url, r], Some(dest), CLONE_TIMEOUT)?;
                run_git(&["checkout", "--quiet", "--detach", "FETCH_HEAD"], Some(dest), CLONE_TIMEOUT)?;
            }
            Some(r) => {
                if r.starts_with('-') {
                    return Err(format!("{r:?} is not a valid ref"));
                }
                run_git(&["clone", "--quiet", "--depth", "1", "--branch", r, "--", url, &d], None, CLONE_TIMEOUT)?;
            }
            None => {
                run_git(&["clone", "--quiet", "--depth", "1", "--", url, &d], None, CLONE_TIMEOUT)?;
            }
        }
        let sha = run_git(&["rev-parse", "HEAD"], Some(dest), CLONE_TIMEOUT)?.trim().to_owned();
        // The history is not needed, and no hook or config in it may run later.
        std::fs::remove_dir_all(dest.join(".git")).map_err(|e| format!("cannot clean the checkout: {e}"))?;
        Ok(sha)
    }
}

/// What a checkout may contain in the way of symlinks: file links that stay
/// inside it. A link to a folder is refused outright (it could be walked
/// through later to somewhere the lexical check below cannot see), and so is
/// any link whose target is absolute or climbs above the root, whether or
/// not the target exists. With no folder links, resolving `..` by name is
/// exact, so a chain of file links cannot escape either. Returns the first
/// offending path, relative to the root.
pub fn find_symlink_escape(root: &Path) -> Result<Option<PathBuf>, String> {
    fn walk(root: &Path, dir: &Path) -> Result<Option<PathBuf>, String> {
        for entry in std::fs::read_dir(dir).map_err(|e| format!("cannot read {}: {e}", dir.display()))? {
            let entry = entry.map_err(|e| e.to_string())?;
            let path = entry.path();
            let ty = entry.file_type().map_err(|e| e.to_string())?;
            if ty.is_symlink() {
                let target = std::fs::read_link(&path).map_err(|e| e.to_string())?;
                let base = path.parent().unwrap_or(root).strip_prefix(root).unwrap_or(Path::new("")).to_path_buf();
                // `metadata` follows the whole chain: a link to a link to a folder is a folder link.
                let to_dir = std::fs::metadata(&path).is_ok_and(|m| m.is_dir());
                if to_dir || target.is_absolute() || !stays_inside(&base, &target) {
                    return Ok(Some(path.strip_prefix(root).unwrap_or(&path).to_path_buf()));
                }
            } else if ty.is_dir() {
                if let Some(bad) = walk(root, &path)? {
                    return Ok(Some(bad));
                }
            }
        }
        Ok(None)
    }
    walk(root, root)
}

/// No component of `root/subdir` may be a symlink: the plugin folder is
/// whatever the path really names.
pub fn find_symlink_in_path(root: &Path, subdir: &str) -> Option<PathBuf> {
    let mut cur = root.to_path_buf();
    for part in subdir.split('/') {
        cur.push(part);
        if std::fs::symlink_metadata(&cur).is_ok_and(|m| m.file_type().is_symlink()) {
            return Some(cur.strip_prefix(root).unwrap_or(&cur).to_path_buf());
        }
    }
    None
}

/// Does `base/target` (lexically) stay at or below the root?
fn stays_inside(base: &Path, target: &Path) -> bool {
    let mut depth: i32 = base.components().count() as i32;
    for c in target.components() {
        match c {
            Component::Normal(_) => depth += 1,
            Component::ParentDir => {
                depth -= 1;
                if depth < 0 {
                    return false;
                }
            }
            Component::CurDir => {}
            _ => return false,
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shorthand_and_urls_parse_to_repo_and_subdir() {
        let p = |s: &str| parse_github(s).unwrap();
        assert_eq!(p("someone/usage-bar"), GithubSource { repo: "someone/usage-bar".into(), subdir: None });
        assert_eq!(p("o/r/plugins/hello"), GithubSource { repo: "o/r".into(), subdir: Some("plugins/hello".into()) });
        assert_eq!(p("https://github.com/o/r"), p("o/r"));
        assert_eq!(p("https://github.com/o/r.git/"), p("o/r"));
        assert_eq!(p("https://github.com/o/r/tools/x").subdir.as_deref(), Some("tools/x"));
        assert_eq!(p("o/r").clone_url(GITHUB), "https://github.com/o/r");
        assert_eq!(p("o/r/a").display(), "o/r/a");
    }

    #[test]
    fn local_paths_other_hosts_and_traversal_are_refused_with_a_useful_message() {
        for local in ["./dev", "/abs/dir", "~/plugins/x", "C:\\x"] {
            assert!(parse_github(local).unwrap_err().contains("plugin link"), "{local}");
        }
        for bad in ["onlyone", "o/", "/r", "o/r/../x", "o/r//x", "o/r/a b", "https://gitlab.com/o/r", "git@github.com:o/r.git", "o/r/tree/main/x", "o/..", "a b/c"] {
            assert!(parse_github(bad).is_err(), "{bad}");
        }
        assert!(parse_github("o/r/tree/main").unwrap_err().contains("--ref"));
    }

    #[cfg(unix)]
    #[test]
    fn symlinks_may_point_at_files_inside_the_checkout_only() {
        use std::os::unix::fs::symlink;
        let d = tempfile::tempdir().unwrap();
        let root = d.path();
        std::fs::create_dir_all(root.join("a/b")).unwrap();
        std::fs::write(root.join("a/file"), "x").unwrap();
        symlink("../file", root.join("a/b/ok")).unwrap();
        symlink("../b/ok", root.join("a/chain")).unwrap();
        assert_eq!(find_symlink_escape(root).unwrap(), None, "file links, chained, inside the root");
        symlink("../../../outside", root.join("a/b/up")).unwrap();
        assert_eq!(find_symlink_escape(root).unwrap(), Some(PathBuf::from("a/b/up")));
        std::fs::remove_file(root.join("a/b/up")).unwrap();
        symlink("/etc/passwd", root.join("abs")).unwrap();
        assert_eq!(find_symlink_escape(root).unwrap(), Some(PathBuf::from("abs")));
    }

    #[cfg(unix)]
    #[test]
    fn folder_links_and_chains_that_end_in_a_folder_or_outside_are_refused() {
        use std::os::unix::fs::symlink;
        let outside = tempfile::tempdir().unwrap();
        std::fs::write(outside.path().join("secret"), "s").unwrap();
        // a folder link, even one that stays inside
        let d = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(d.path().join("a/b")).unwrap();
        symlink("b", d.path().join("a/dirlink")).unwrap();
        assert_eq!(find_symlink_escape(d.path()).unwrap(), Some(PathBuf::from("a/dirlink")));
        // a chain: two innocent-looking hops ending at a folder
        let d = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(d.path().join("real")).unwrap();
        symlink("real", d.path().join("hop2")).unwrap();
        symlink("hop2", d.path().join("hop1")).unwrap();
        assert!(find_symlink_escape(d.path()).unwrap().is_some());
        // a chain whose last hop leaves the root: the walk reaches that hop
        let d = tempfile::tempdir().unwrap();
        symlink(outside.path().join("secret"), d.path().join("last")).unwrap();
        symlink("last", d.path().join("first")).unwrap();
        assert_eq!(find_symlink_escape(d.path()).unwrap().map(|p| p.to_string_lossy().into_owned()).as_deref().map(|p| p == "last" || p == "first"), Some(true));
        // `..` through a folder link cannot be used to climb out
        let d = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(d.path().join("x/y")).unwrap();
        symlink("x/y", d.path().join("deep")).unwrap();
        symlink("deep/../../../outside", d.path().join("climb")).unwrap();
        assert!(find_symlink_escape(d.path()).unwrap().is_some());
    }

    #[cfg(unix)]
    #[test]
    fn a_plugin_subdir_may_not_pass_through_a_symlink() {
        use std::os::unix::fs::symlink;
        let d = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(d.path().join("real/inner")).unwrap();
        assert_eq!(find_symlink_in_path(d.path(), "real/inner"), None);
        symlink("real", d.path().join("alias")).unwrap();
        assert_eq!(find_symlink_in_path(d.path(), "alias/inner"), Some(PathBuf::from("alias")));
        symlink("inner", d.path().join("real/last")).unwrap();
        assert_eq!(find_symlink_in_path(d.path(), "real/last"), Some(PathBuf::from("real/last")));
    }

    fn plain_git(dir: &Path, args: &[&str], home: Option<&Path>) -> std::process::Output {
        let mut c = Command::new("git");
        c.args(args).current_dir(dir).env("GIT_CONFIG_NOSYSTEM", "1").env("GIT_CONFIG_GLOBAL", "/dev/null");
        if let Some(h) = home {
            c.env("HOME", h).env("GIT_CONFIG_GLOBAL", h.join(".gitconfig"));
        }
        let o = c.output().unwrap();
        assert!(o.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&o.stderr));
        o
    }

    /// The user's own git config names a filter and a hooks folder that would
    /// run commands; a repository with `.gitattributes` asking for that filter
    /// must not make them run when garage checks it out.
    #[cfg(unix)]
    #[test]
    fn a_hostile_attributes_file_and_user_git_config_run_nothing() {
        use std::os::unix::fs::PermissionsExt;
        let t = tempfile::tempdir().unwrap();
        let (repo, home, marker, hooks) = (t.path().join("repo"), t.path().join("home"), t.path().join("ran"), t.path().join("hooks"));
        std::fs::create_dir_all(&repo).unwrap();
        std::fs::create_dir_all(&home).unwrap();
        std::fs::create_dir_all(&hooks).unwrap();
        let hook = hooks.join("post-checkout");
        std::fs::write(&hook, format!("#!/bin/sh\necho hook >> {}\n", marker.display())).unwrap();
        std::fs::set_permissions(&hook, std::fs::Permissions::from_mode(0o755)).unwrap();
        let smudge = t.path().join("smudge.sh");
        std::fs::write(&smudge, format!("#!/bin/sh\necho smudge >> {}\ncat\n", marker.display())).unwrap();
        std::fs::set_permissions(&smudge, std::fs::Permissions::from_mode(0o755)).unwrap();
        std::fs::write(
            home.join(".gitconfig"),
            format!("[filter \"evil\"]\n\tsmudge = {s}\n\tclean = cat\n\trequired = true\n[core]\n\thooksPath = {h}\n", s = smudge.display(), h = hooks.display()),
        )
        .unwrap();
        std::fs::write(repo.join(".gitattributes"), "* filter=evil\n").unwrap();
        std::fs::write(repo.join("garage-plugin.toml"), "x").unwrap();
        plain_git(&repo, &["init", "--quiet"], None);
        plain_git(&repo, &["-c", "user.name=t", "-c", "user.email=t@t", "add", "-A"], None);
        plain_git(&repo, &["-c", "user.name=t", "-c", "user.email=t@t", "commit", "--quiet", "-m", "x"], None);

        // Control: with the user's config honoured, checking the file out runs both.
        std::fs::remove_file(repo.join("garage-plugin.toml")).unwrap();
        plain_git(&repo, &["checkout", "--quiet", "--", "."], Some(&home));
        let ran = std::fs::read_to_string(&marker).unwrap_or_default();
        assert!(ran.contains("smudge"), "the fixture must be able to run a filter: {ran:?}");

        // Through garage's git: the same checkout, the same config in HOME, nothing runs.
        std::fs::remove_file(&marker).unwrap();
        std::fs::remove_file(repo.join("garage-plugin.toml")).unwrap();
        let out = git_command(&["checkout", "--quiet", "--", "."], Some(&repo)).env("HOME", &home).env("XDG_CONFIG_HOME", &home).output().unwrap();
        assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
        assert_eq!(std::fs::read_to_string(repo.join("garage-plugin.toml")).unwrap(), "x");
        assert!(!marker.exists(), "garage's git ran a filter or hook: {:?}", std::fs::read_to_string(&marker));
    }
}
