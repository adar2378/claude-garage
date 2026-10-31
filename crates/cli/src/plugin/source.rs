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

/// Run git with a deadline; stderr is kept for the error message.
fn run_git(args: &[&str], cwd: Option<&Path>, timeout: Duration) -> Result<String, String> {
    let mut cmd = Command::new("git");
    cmd.args(args).env("GIT_TERMINAL_PROMPT", "0").stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped());
    if let Some(c) = cwd {
        cmd.current_dir(c);
    }
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

/// A symlink in a checkout may point only inside it. Absolute targets and
/// anything that walks above the root are refused, whether or not the target
/// exists. Returns the offending path, relative to the root.
pub fn find_symlink_escape(root: &Path) -> Result<Option<PathBuf>, String> {
    fn walk(root: &Path, dir: &Path) -> Result<Option<PathBuf>, String> {
        for entry in std::fs::read_dir(dir).map_err(|e| format!("cannot read {}: {e}", dir.display()))? {
            let entry = entry.map_err(|e| e.to_string())?;
            let path = entry.path();
            let ty = entry.file_type().map_err(|e| e.to_string())?;
            if ty.is_symlink() {
                let target = std::fs::read_link(&path).map_err(|e| e.to_string())?;
                let base = path.parent().unwrap_or(root).strip_prefix(root).unwrap_or(Path::new("")).to_path_buf();
                if target.is_absolute() || !stays_inside(&base, &target) {
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
    fn symlinks_may_point_inside_the_checkout_only() {
        use std::os::unix::fs::symlink;
        let d = tempfile::tempdir().unwrap();
        let root = d.path();
        std::fs::create_dir_all(root.join("a/b")).unwrap();
        std::fs::write(root.join("a/file"), "x").unwrap();
        symlink("../file", root.join("a/b/ok")).unwrap();
        symlink("b", root.join("a/dirlink")).unwrap();
        assert_eq!(find_symlink_escape(root).unwrap(), None);
        symlink("../../../outside", root.join("a/b/up")).unwrap();
        assert_eq!(find_symlink_escape(root).unwrap(), Some(PathBuf::from("a/b/up")));
        std::fs::remove_file(root.join("a/b/up")).unwrap();
        symlink("/etc/passwd", root.join("abs")).unwrap();
        assert_eq!(find_symlink_escape(root).unwrap(), Some(PathBuf::from("abs")));
    }
}
