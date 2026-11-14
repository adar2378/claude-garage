//! The plugin's own config (`config.toml` in `GARAGE_PLUGIN_STATE_DIR`) and its
//! disposable `cache/`. The config is written only by the user: this module
//! reads it on every run and never creates or rewrites it. `cache/` is the
//! only place the plugin writes (mode 0700, files 0600, atomic, under a lock).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

use garage_plugin_sdk::{Error, StateLock};
use regex::Regex;
use serde::Deserialize;

/// Default for `pr.task`: extracts only the task number from a PR title.
pub const DEFAULT_TASK_PATTERN: &str = r"\[(?P<change>[a-z0-9-]+)(?: (?P<task>[\d.]+))?\]";

#[derive(Debug, Default, Deserialize)]
struct RawConfig {
    #[serde(default)]
    workspace: BTreeMap<String, RawWorkspace>,
}

#[derive(Debug, Deserialize)]
struct RawWorkspace {
    repos: Option<Vec<String>>,
    #[serde(default)]
    github: bool,
    #[serde(default = "yes")]
    fetch: bool,
    #[serde(default)]
    branches: Vec<String>,
    #[serde(default)]
    pr: RawPr,
    #[serde(default)]
    comments: RawComments,
}

fn yes() -> bool {
    true
}

#[derive(Debug, Default, Deserialize)]
struct RawPr {
    task: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
struct RawComments {
    ci: Option<String>,
    review: Option<String>,
    owner_needed: Option<String>,
}

/// The validated patterns of one workspace: an invalid one is `None` and has
/// a warning.
#[derive(Debug, Clone, Default)]
pub struct Patterns {
    pub task: Option<String>,
    pub ci: Option<String>,
    pub review: Option<String>,
    pub owner_needed: Option<String>,
}

/// One `[workspace.<name>]` table, validated.
#[derive(Debug, Clone)]
pub struct WorkspaceConfig {
    /// As written; `~` is expanded when the repos are resolved.
    pub repos: Option<Vec<String>>,
    pub github: bool,
    pub fetch: bool,
    pub branches: Vec<String>,
    pub patterns: Patterns,
}

impl Default for WorkspaceConfig {
    fn default() -> Self {
        WorkspaceConfig {
            repos: None,
            github: false,
            fetch: true,
            branches: vec![],
            patterns: Patterns {
                task: Some(DEFAULT_TASK_PATTERN.to_owned()),
                ..Patterns::default()
            },
        }
    }
}

/// The whole file, plus the warnings found while reading it.
#[derive(Debug, Default)]
pub struct Config {
    pub workspaces: BTreeMap<String, WorkspaceConfig>,
    /// File-level warnings (bad TOML).
    pub warnings: Vec<String>,
    /// Warnings that belong to one workspace (bad regexes).
    pub workspace_warnings: BTreeMap<String, Vec<String>>,
}

impl Config {
    /// The config of `name`; the defaults when the file has no table for it.
    pub fn workspace(&self, name: &str) -> WorkspaceConfig {
        self.workspaces.get(name).cloned().unwrap_or_default()
    }
}

/// `Some(pattern)` when it compiles, else a warning naming `key`.
fn check_pattern(key: &str, pattern: Option<String>, warnings: &mut Vec<String>) -> Option<String> {
    let p = pattern?;
    match Regex::new(&p) {
        Ok(_) => Some(p),
        Err(e) => {
            let reason = e.to_string().lines().last().unwrap_or("").trim().to_owned();
            warnings.push(format!(
                "{key} is not a valid regex ({reason}); pattern off"
            ));
            None
        }
    }
}

/// Parse `text` as the config file. Never fails: a file that does not parse
/// behaves as an empty one, with a warning.
pub fn parse(text: &str) -> Config {
    let raw: RawConfig = match toml::from_str(text) {
        Ok(r) => r,
        Err(e) => {
            let first = e.to_string().lines().next().unwrap_or("").to_owned();
            return Config {
                warnings: vec![format!("config.toml could not be read: {first}")],
                ..Config::default()
            };
        }
    };
    let mut cfg = Config::default();
    for (name, w) in raw.workspace {
        let mut warnings = vec![];
        let t = format!("workspace.{name}");
        let patterns = Patterns {
            task: check_pattern(
                &format!("{t}.pr.task"),
                Some(w.pr.task.unwrap_or_else(|| DEFAULT_TASK_PATTERN.to_owned())),
                &mut warnings,
            ),
            ci: check_pattern(&format!("{t}.comments.ci"), w.comments.ci, &mut warnings),
            review: check_pattern(
                &format!("{t}.comments.review"),
                w.comments.review,
                &mut warnings,
            ),
            owner_needed: check_pattern(
                &format!("{t}.comments.owner_needed"),
                w.comments.owner_needed,
                &mut warnings,
            ),
        };
        if !warnings.is_empty() {
            cfg.workspace_warnings.insert(name.clone(), warnings);
        }
        cfg.workspaces.insert(
            name,
            WorkspaceConfig {
                repos: w.repos,
                github: w.github,
                fetch: w.fetch,
                branches: w.branches,
                patterns,
            },
        );
    }
    cfg
}

/// Read `<state_dir>/config.toml`. A missing file is an empty one, without a
/// warning. Never writes.
pub fn load(state_dir: &Path) -> Config {
    match std::fs::read_to_string(state_dir.join("config.toml")) {
        Ok(text) => parse(&text),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Config::default(),
        Err(e) => Config {
            warnings: vec![format!("config.toml could not be read: {e}")],
            ..Config::default()
        },
    }
}

/// `~` and `~/x` expanded against `home`.
pub fn expand_home(path: &str, home: Option<&Path>) -> PathBuf {
    match (path, home) {
        ("~", Some(h)) => h.to_path_buf(),
        (p, Some(h)) if p.starts_with("~/") => h.join(&p[2..]),
        (p, _) => PathBuf::from(p),
    }
}

fn is_git_repo(dir: &Path) -> bool {
    dir.join(".git").exists()
}

fn has_openspec(dir: &Path) -> bool {
    dir.join("openspec").is_dir()
}

/// The repos of a workspace and the warnings found on the way.
///
/// With `repos` set: exactly those paths (`~` expanded); one that is missing
/// or not a git repo is a warning. Without it: the workspace dir when it is a
/// git repo or has `openspec/`, plus its direct child directories that are git
/// repos with `openspec/`.
pub fn resolve_repos(
    dir: &Path,
    cfg: &WorkspaceConfig,
    home: Option<&Path>,
) -> (Vec<PathBuf>, Vec<String>) {
    let (mut repos, mut warnings) = (vec![], vec![]);
    if let Some(listed) = &cfg.repos {
        for p in listed {
            let path = expand_home(p, home);
            if is_git_repo(&path) {
                repos.push(path);
            } else {
                warnings.push(format!("{p} is not a git repo"));
            }
        }
        return (repos, warnings);
    }
    if is_git_repo(dir) || has_openspec(dir) {
        repos.push(dir.to_path_buf());
    }
    let mut children: Vec<PathBuf> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
        .map(|e| e.path())
        .filter(|p| is_git_repo(p) && has_openspec(p))
        .collect();
    children.sort();
    repos.extend(children);
    (repos, warnings)
}

/// The folder the plugin may write to: `<state_dir>/cache`, mode 0700.
pub fn cache_dir(state_dir: &Path) -> Result<PathBuf, Error> {
    let dir = state_dir.join("cache");
    let io = |e: std::io::Error| Error::Context(format!("cache: {e}"));
    std::fs::create_dir_all(&dir).map_err(io)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700)).map_err(io)?;
    }
    Ok(dir)
}

/// Write `name` in `cache/` atomically (temp file, then rename), file mode
/// 0600, holding the `StateLock` `<name>` for the step.
pub fn write_cache(state_dir: &Path, name: &str, bytes: &[u8]) -> Result<(), Error> {
    use std::io::Write;
    let dir = cache_dir(state_dir)?;
    let _lock = StateLock::acquire(&dir, name, Duration::from_secs(30), Duration::from_secs(2))?;
    let io = |e: std::io::Error| Error::Context(format!("cache/{name}: {e}"));
    let tmp = dir.join(format!("{name}.{}.tmp", std::process::id()));
    let mut opts = std::fs::OpenOptions::new();
    opts.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(0o600);
    }
    opts.open(&tmp)
        .and_then(|mut f| f.write_all(bytes))
        .map_err(io)?;
    std::fs::rename(&tmp, dir.join(name)).map_err(io)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn repo(dir: &Path, openspec: bool) {
        std::fs::create_dir_all(dir.join(".git")).unwrap();
        if openspec {
            std::fs::create_dir_all(dir.join("openspec/changes")).unwrap();
        }
    }

    #[test]
    fn no_config_file_is_an_empty_config_without_a_warning() {
        let d = tempfile::tempdir().unwrap();
        let cfg = load(d.path());
        assert!(cfg.workspaces.is_empty() && cfg.warnings.is_empty());
        let ws = cfg.workspace("kowboy");
        assert!(!ws.github && ws.fetch && ws.repos.is_none());
        assert_eq!(ws.patterns.task.as_deref(), Some(DEFAULT_TASK_PATTERN));
        // Reading never creates anything.
        assert_eq!(std::fs::read_dir(d.path()).unwrap().count(), 0);
    }

    #[test]
    fn a_full_table_is_read() {
        let cfg = parse(
            r#"
[workspace.kowboy]
github = true
fetch = false
repos = ["~/code/kowboy/api"]
branches = ["feat/*"]
[workspace.kowboy.comments]
ci = 'CI: (?P<status>PASS|FAIL) (?P<sha>[0-9a-f]+)'
"#,
        );
        let w = cfg.workspace("kowboy");
        assert!(w.github && !w.fetch);
        assert_eq!(w.branches, ["feat/*"]);
        assert_eq!(
            w.repos.as_deref(),
            Some(&["~/code/kowboy/api".to_owned()][..])
        );
        assert!(w.patterns.ci.is_some() && w.patterns.review.is_none());
        assert!(cfg.workspace_warnings.is_empty());
    }

    #[test]
    fn bad_toml_behaves_as_empty_with_a_warning() {
        let cfg = parse("[workspace.kowboy\ngithub = ");
        assert!(cfg.workspaces.is_empty());
        assert_eq!(cfg.warnings.len(), 1);
        assert!(cfg.warnings[0].starts_with("config.toml could not be read"));
        // A wrong type is also unparsable, not a panic.
        assert_eq!(parse("[workspace.a]\ngithub = \"yes\"").warnings.len(), 1);
    }

    #[test]
    fn a_bad_regex_disables_only_that_pattern_and_names_it() {
        let cfg = parse(
            r#"
[workspace.kowboy.comments]
ci = 'CI (?P<status>PASS|FAIL)'
review = 'Claude review: ('
"#,
        );
        let w = cfg.workspace("kowboy");
        assert!(w.patterns.ci.is_some());
        assert!(w.patterns.review.is_none());
        let warns = &cfg.workspace_warnings["kowboy"];
        assert_eq!(warns.len(), 1);
        assert!(warns[0].contains("comments.review"), "{warns:?}");
    }

    #[test]
    fn a_bad_task_pattern_falls_back_to_none_with_a_warning() {
        let cfg = parse("[workspace.a.pr]\ntask = '['");
        assert!(cfg.workspace("a").patterns.task.is_none());
        assert!(cfg.workspace_warnings["a"][0].contains("pr.task"));
    }

    #[test]
    fn with_no_repos_the_workspace_dir_and_child_repos_with_openspec_are_scanned() {
        let d = tempfile::tempdir().unwrap();
        repo(&d.path().join("api"), true);
        repo(&d.path().join("web"), true);
        repo(&d.path().join("no-openspec"), false);
        std::fs::create_dir_all(d.path().join("notes/openspec")).unwrap();
        let (repos, warns) = resolve_repos(d.path(), &WorkspaceConfig::default(), None);
        let names: Vec<_> = repos
            .iter()
            .map(|p| p.file_name().unwrap().to_str().unwrap().to_owned())
            .collect();
        assert_eq!(names, ["api", "web"]);
        assert!(warns.is_empty());
    }

    #[test]
    fn the_workspace_dir_itself_counts_when_it_has_openspec() {
        let d = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(d.path().join("openspec")).unwrap();
        let (repos, _) = resolve_repos(d.path(), &WorkspaceConfig::default(), None);
        assert_eq!(repos, [d.path().to_path_buf()]);
    }

    #[test]
    fn a_bad_listed_path_is_a_warning_and_the_others_are_kept() {
        let home = tempfile::tempdir().unwrap();
        repo(&home.path().join("code/kowboy/api"), true);
        let cfg = WorkspaceConfig {
            repos: Some(vec![
                "~/code/kowboy/api".into(),
                "~/code/kowboy/gone".into(),
            ]),
            ..WorkspaceConfig::default()
        };
        let (repos, warns) = resolve_repos(Path::new("/ignored"), &cfg, Some(home.path()));
        assert_eq!(repos, [home.path().join("code/kowboy/api")]);
        assert_eq!(warns, ["~/code/kowboy/gone is not a git repo"]);
    }

    #[test]
    fn a_listed_folder_that_is_not_a_git_repo_is_a_warning() {
        let d = tempfile::tempdir().unwrap();
        repo(&d.path().join("plain"), false);
        std::fs::remove_dir_all(d.path().join("plain/.git")).unwrap();
        let cfg = WorkspaceConfig {
            repos: Some(vec![d.path().join("plain").to_string_lossy().into_owned()]),
            ..WorkspaceConfig::default()
        };
        let (repos, warns) = resolve_repos(d.path(), &cfg, None);
        assert!(repos.is_empty() && warns.len() == 1);
    }

    #[test]
    fn expand_home_handles_tilde_forms() {
        let h = Path::new("/home/x");
        assert_eq!(expand_home("~", Some(h)), h);
        assert_eq!(expand_home("~/a/b", Some(h)), Path::new("/home/x/a/b"));
        assert_eq!(expand_home("/abs", Some(h)), Path::new("/abs"));
        assert_eq!(expand_home("~/a", None), Path::new("~/a"));
    }

    #[cfg(unix)]
    #[test]
    fn the_cache_is_private_and_is_the_only_thing_written() {
        use std::os::unix::fs::PermissionsExt;
        let d = tempfile::tempdir().unwrap();
        std::fs::write(d.path().join("config.toml"), "# mine\n").unwrap();
        write_cache(d.path(), "model.json", b"{}").unwrap();
        write_cache(d.path(), "model.json", b"{\"v\":2}").unwrap();
        let mode = |p: &Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode(&d.path().join("cache")), 0o700);
        assert_eq!(mode(&d.path().join("cache/model.json")), 0o600);
        assert_eq!(
            std::fs::read(d.path().join("cache/model.json")).unwrap(),
            b"{\"v\":2}"
        );
        // No temp or lock file is left, and the config is untouched.
        let left: Vec<_> = std::fs::read_dir(d.path().join("cache"))
            .unwrap()
            .map(|e| e.unwrap().file_name().into_string().unwrap())
            .collect();
        assert_eq!(left, ["model.json"]);
        assert_eq!(
            std::fs::read_to_string(d.path().join("config.toml")).unwrap(),
            "# mine\n"
        );
    }
}
