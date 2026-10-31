//! cli-extras: `claude-garage <folder>`, `claude-garage install` and
//! `claude-garage prune`, as one plugin. Everything goes through the daemon's
//! HTTP API (the CLI starts the daemon first: `needs_daemon`).

use std::path::{Path, PathBuf};

use garage_plugin_sdk::folder::canonical_text;
use garage_plugin_sdk::{Api, Error, Value, json, workspace_has_sessions, workspace_of};

/// What `open` was asked for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpenRequest {
    /// The folder as typed.
    pub word: String,
    /// It arrived as an unknown first word (not as `claude-garage open ...`),
    /// so a non-folder is a mistyped subcommand.
    pub fallback: bool,
    pub no_open: bool,
}

/// Read the words after `claude-garage`: `[open] <folder> [--no-open]`.
pub fn parse_open(argv: &[String]) -> Option<OpenRequest> {
    let fallback = argv.first().map(String::as_str) != Some("open");
    let words = if fallback { argv } else { &argv[1..] };
    let no_open = words.iter().any(|w| w == "--no-open");
    let word = words.iter().find(|w| !w.starts_with("--"))?;
    Some(OpenRequest {
        word: word.clone(),
        fallback,
        no_open,
    })
}

/// The directory's basename as a workspace name: lowercase, every run of
/// characters outside `[a-z0-9]` becomes one `-`, trimmed of `-`.
pub fn workspace_name(dir: &Path) -> Option<String> {
    let base = dir.file_name()?.to_string_lossy().to_lowercase();
    let mut out = String::new();
    for c in base.chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c);
        } else if !out.ends_with('-') {
            out.push('-');
        }
    }
    let out = out.trim_matches('-').to_owned();
    (!out.is_empty()).then_some(out)
}

/// The name to use for `dir`: the folder's own workspace if it is registered
/// already, else the first free of `base`, `base-2`, `base-3` ...
/// `(name, already registered)`.
pub fn choose_workspace(workspaces: &Value, base: &str, dir: &str) -> (String, bool) {
    let registered = |name: &str| {
        workspaces
            .as_array()
            .into_iter()
            .flatten()
            .find(|w| w.get("name").and_then(Value::as_str) == Some(name))
            .map(|w| w.get("dir").and_then(Value::as_str).unwrap_or(""))
    };
    // the same folder under any name wins; both sides are compared by their
    // real paths, so a symlinked or differently spelled registration matches
    let want = canonical_text(dir);
    if let Some(w) = workspaces.as_array().into_iter().flatten().find(|w| {
        w.get("dir")
            .and_then(Value::as_str)
            .is_some_and(|d| canonical_text(d) == want)
    }) && let Some(name) = w.get("name").and_then(Value::as_str)
    {
        return (name.to_owned(), true);
    }
    for n in 1u32.. {
        let name = if n == 1 {
            base.to_owned()
        } else {
            format!("{base}-{n}")
        };
        if registered(&name).is_none() {
            return (name, false);
        }
    }
    unreachable!("an unbounded range finds a free name")
}

/// First free `claude-N` in the workspace (the wall's default label family).
pub fn next_label(sessions: &Value, workspace: &str) -> String {
    let taken: Vec<u32> = sessions
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|s| s.get("id").and_then(Value::as_str))
        .filter(|id| workspace_of(id) == Some(workspace))
        .filter_map(|id| id.rsplit('/').next()?.strip_prefix("claude-")?.parse().ok())
        .collect();
    let n = (1u32..).find(|n| !taken.contains(n)).unwrap_or(1);
    format!("claude-{n}")
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Opened {
    pub workspace: String,
    pub dir: String,
    pub registered_now: bool,
    /// The session this call started; `None` when the project already had one.
    pub started: Option<String>,
}

impl Opened {
    pub fn summary(&self) -> String {
        let how = if self.registered_now {
            "added"
        } else {
            "found"
        };
        match &self.started {
            Some(id) => format!("{how} {} ({}), started {id}", self.workspace, self.dir),
            None => format!(
                "{how} {} ({}), already has sessions",
                self.workspace, self.dir
            ),
        }
    }
}

/// Register `dir` as a project (if it is not one) and start a session in it
/// (if it has none).
pub fn open_folder(api: &Api, dir: &Path) -> Result<Opened, Error> {
    // `resolve_folder` already refused a path that is not UTF-8; this guards
    // a caller that did not use it
    let dir_s = dir
        .to_str()
        .ok_or_else(|| {
            Error::Context(format!(
                "{}: the path is not valid UTF-8 and cannot be stored",
                dir.display()
            ))
        })?
        .to_owned();
    let base = workspace_name(dir).ok_or_else(|| {
        Error::Context(format!(
            "{dir_s}: the folder name has no letters or digits to make a project name from"
        ))
    })?;
    let (name, existed) = choose_workspace(&api.get("/api/workspaces")?, &base, &dir_s);
    if !existed {
        api.put("/api/workspaces", &json!({"name": name, "dir": dir_s}))?;
    }
    let sessions = api.get("/api/sessions")?;
    let live = sessions.as_array().into_iter().flatten().any(|s| {
        s.get("restorable").and_then(Value::as_bool) != Some(true)
            && s.get("workspace").and_then(Value::as_str) == Some(name.as_str())
    });
    let mut started = None;
    if !live {
        let label = next_label(&sessions, &name);
        match api.post("/api/sessions", &json!({"workspace": name, "label": label})) {
            Ok(_) => started = Some(format!("garage/{name}/{label}")),
            // somebody started it between our two calls
            Err(Error::Http { status: 409, .. }) => {}
            Err(e) => return Err(e),
        }
    }
    Ok(Opened {
        workspace: name,
        dir: dir_s,
        registered_now: !existed,
        started,
    })
}

fn reason(e: &Error) -> String {
    match e {
        Error::Http { body, status } => serde_json::from_str::<Value>(body)
            .ok()
            .and_then(|v| v.get("error").and_then(Value::as_str).map(str::to_owned))
            .unwrap_or_else(|| format!("daemon answered {status}")),
        other => other.to_string(),
    }
}

/// Install the hooks and the statusline wrapper; one result line each.
pub fn install(api: &Api) -> Vec<String> {
    [
        ("hooks", "/api/hooks/install"),
        ("statusline", "/api/statusline/install"),
    ]
    .into_iter()
    .map(|(what, path)| match api.post(path, &json!({})) {
        Ok(v) if v.get("alreadyInstalled").and_then(Value::as_bool) == Some(true) => {
            format!("{what}: already installed")
        }
        Ok(_) => format!("{what}: installed"),
        Err(e) => format!("{what}: FAILED: {}", reason(&e)),
    })
    .collect()
}

/// Projects with no session at all (live or restorable), by name.
pub fn empty_workspaces(workspaces: &Value, sessions: &Value) -> Vec<String> {
    let mut names: Vec<String> = workspaces
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|w| w.get("name").and_then(Value::as_str))
        .filter(|n| !workspace_has_sessions(sessions, n))
        .map(str::to_owned)
        .collect();
    names.sort();
    names
}

/// A project registered more recently than this is never removed as empty:
/// it was probably just added and is waiting for its first chat. The daemon
/// enforces it.
pub const MIN_AGE_SECS: u32 = 600;

/// What `prune` would do.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct PrunePlan {
    /// No session and no closed chat: candidates for removal.
    pub remove: Vec<String>,
    /// No session, but closed chats the user may want back: left alone.
    pub keep_for_history: Vec<String>,
}

/// Look for projects `prune` may remove.
pub fn plan_prune(api: &Api) -> Result<PrunePlan, Error> {
    let empty = empty_workspaces(&api.get("/api/workspaces")?, &api.get("/api/sessions")?);
    let mut plan = PrunePlan::default();
    for name in empty {
        let closed = match api.get(&format!("/api/workspaces/{name}/closed")) {
            Ok(v) => v.as_array().map_or(0, Vec::len),
            // removed meanwhile
            Err(Error::Http { status: 404, .. }) => continue,
            Err(e) => return Err(e),
        };
        if closed > 0 {
            plan.keep_for_history.push(name);
        } else {
            plan.remove.push(name);
        }
    }
    Ok(plan)
}

/// What happened to one project.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Pruned {
    Removed,
    /// The daemon refused: it holds something now, or is too new. The text is
    /// the daemon's.
    Kept(String),
}

/// Remove `names`, each only if the daemon still finds it empty and old
/// enough (`DELETE ...?if_empty=1&min_age_secs=...`, decided atomically).
/// A project removed meanwhile (404) counts as done.
pub fn prune(api: &Api, names: &[String]) -> Result<Vec<(String, Pruned)>, Error> {
    let mut out = Vec::new();
    for n in names {
        let res = api.delete(&format!(
            "/api/workspaces/{n}?if_empty=1&min_age_secs={MIN_AGE_SECS}"
        ));
        let r = match res {
            Ok(_) | Err(Error::Http { status: 404, .. }) => Pruned::Removed,
            Err(e @ Error::Http { status: 409, .. }) => Pruned::Kept(reason(&e)),
            Err(e) => return Err(e),
        };
        out.push((n.clone(), r));
    }
    Ok(out)
}

/// What to do about asking before `prune` removes anything.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Confirm {
    /// `--yes`: no question.
    Go,
    /// A person is at the terminal: ask `[y/N]`.
    Ask,
    /// No terminal and no `--yes`: do not guess.
    Refuse,
}

pub fn confirm_mode(yes: bool, interactive: bool) -> Confirm {
    if yes {
        Confirm::Go
    } else if interactive {
        Confirm::Ask
    } else {
        Confirm::Refuse
    }
}

/// Does `answer` (a line typed at the `[y/N]` question) mean yes?
pub fn is_yes(answer: &str) -> bool {
    matches!(answer.trim().to_lowercase().as_str(), "y" | "yes")
}

#[cfg(test)]
mod tests {
    use super::*;
    use garage_plugin_sdk::testing::FakeDaemon;

    fn s(v: &[&str]) -> Vec<String> {
        v.iter().map(|x| (*x).to_owned()).collect()
    }

    #[test]
    fn a_bare_word_is_the_fallback_and_open_is_explicit() {
        let r = parse_open(&s(&["~/dev/app"])).unwrap();
        assert_eq!(
            (r.word.as_str(), r.fallback, r.no_open),
            ("~/dev/app", true, false)
        );
        let r = parse_open(&s(&["open", "app", "--no-open"])).unwrap();
        assert_eq!(
            (r.word.as_str(), r.fallback, r.no_open),
            ("app", false, true)
        );
        let r = parse_open(&s(&["--no-open", "x"])).unwrap();
        assert_eq!((r.word.as_str(), r.no_open), ("x", true));
        assert_eq!(parse_open(&s(&["open"])), None);
        assert_eq!(parse_open(&[]), None);
    }

    #[test]
    fn names_are_lowercase_dashed_and_never_empty() {
        for (dir, want) in [
            ("/dev/MyApp", Some("myapp")),
            ("/dev/my app_v2", Some("my-app-v2")),
            ("/dev/--a..b--", Some("a-b")),
            ("/dev/日本", None),
            ("/", None),
        ] {
            assert_eq!(workspace_name(Path::new(dir)).as_deref(), want, "{dir}");
        }
    }

    #[test]
    fn a_registered_folder_is_reused_a_clash_gets_a_suffix() {
        let ws = json!([
            {"name": "app", "dir": "/dev/app"},
            {"name": "app-2", "dir": "/work/app"},
        ]);
        assert_eq!(
            choose_workspace(&ws, "app", "/dev/app/"),
            ("app".into(), true)
        );
        assert_eq!(
            choose_workspace(&ws, "app", "/work/app"),
            ("app-2".into(), true)
        );
        assert_eq!(
            choose_workspace(&ws, "app", "/third/app"),
            ("app-3".into(), false)
        );
        assert_eq!(choose_workspace(&json!([]), "x", "/x"), ("x".into(), false));
    }

    #[cfg(unix)]
    #[test]
    fn the_same_folder_under_a_symlink_or_trailing_slash_is_the_same_project() {
        let root = tempfile::tempdir().unwrap();
        let real = root.path().join("real");
        std::fs::create_dir(&real).unwrap();
        let link = root.path().join("link");
        std::os::unix::fs::symlink(&real, &link).unwrap();
        let (real_s, link_s) = (
            real.to_string_lossy().into_owned(),
            link.to_string_lossy().into_owned(),
        );
        // registered through the link ...
        let ws = json!([{"name": "app", "dir": link_s}]);
        // ... opened by its real path, with or without a slash
        assert_eq!(choose_workspace(&ws, "real", &real_s), ("app".into(), true));
        assert_eq!(
            choose_workspace(&ws, "real", &format!("{real_s}/")),
            ("app".into(), true)
        );
        // and the other way round
        let ws = json!([{"name": "app", "dir": format!("{real_s}/")}]);
        assert_eq!(choose_workspace(&ws, "link", &link_s), ("app".into(), true));
        // a folder that is somewhere else is not matched
        let other = root.path().join("other");
        std::fs::create_dir(&other).unwrap();
        assert_eq!(
            choose_workspace(&ws, "other", &other.to_string_lossy()),
            ("other".into(), false)
        );
    }

    #[test]
    fn labels_follow_the_walls_claude_n_family() {
        let sessions = json!([
            {"id": "garage/app/claude-1"},
            {"id": "garage/app/claude-3"},
            {"id": "garage/app/notes"},
            {"id": "garage/other/claude-2"},
        ]);
        assert_eq!(next_label(&sessions, "app"), "claude-2");
        assert_eq!(next_label(&sessions, "other"), "claude-1");
        assert_eq!(next_label(&json!([]), "app"), "claude-1");
    }

    fn folder_daemon(workspaces: &'static str, sessions: &'static str, create: u16) -> FakeDaemon {
        FakeDaemon::start(move |req| match (req.method.as_str(), req.path.as_str()) {
            ("GET", "/api/workspaces") => (200, workspaces.into()),
            ("GET", "/api/sessions") => (200, sessions.into()),
            ("PUT", "/api/workspaces") => (200, "{}".into()),
            ("POST", "/api/sessions") => (create, r#"{"error":"x"}"#.into()),
            _ => (500, "{}".into()),
        })
    }

    #[test]
    fn a_new_folder_is_registered_and_gets_a_first_session() {
        let d = folder_daemon("[]", "[]", 200);
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("My App");
        std::fs::create_dir(&dir).unwrap();
        let o = open_folder(&d.api("cli-extras"), &dir).unwrap();
        assert_eq!((o.workspace.as_str(), o.registered_now), ("my-app", true));
        assert_eq!(o.started.as_deref(), Some("garage/my-app/claude-1"));
        let reqs = d.requests();
        let put = reqs.iter().find(|r| r.method == "PUT").unwrap();
        assert_eq!(put.body["name"], "my-app");
        assert_eq!(put.body["dir"], dir.to_string_lossy().as_ref());
        let post = reqs.iter().find(|r| r.method == "POST").unwrap();
        assert_eq!(
            post.body,
            json!({"workspace": "my-app", "label": "claude-1"})
        );
        assert!(o.summary().starts_with("added my-app"), "{}", o.summary());
    }

    #[test]
    fn a_project_that_already_has_a_live_session_gets_no_second_one() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("app");
        std::fs::create_dir(&dir).unwrap();
        let ws: &'static str = Box::leak(
            format!(r#"[{{"name":"app","dir":{:?}}}]"#, dir.to_string_lossy()).into_boxed_str(),
        );
        let d = folder_daemon(
            ws,
            r#"[{"id":"garage/app/claude-1","workspace":"app","status":"idle"}]"#,
            200,
        );
        let o = open_folder(&d.api("cli-extras"), &dir).unwrap();
        assert!(!o.registered_now && o.started.is_none());
        assert!(
            d.requests().iter().all(|r| r.method == "GET"),
            "{:?}",
            d.log()
        );
        assert!(o.summary().contains("already has sessions"));
    }

    #[test]
    fn a_session_that_appears_meanwhile_is_not_an_error_but_a_real_failure_is() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("app");
        std::fs::create_dir(&dir).unwrap();
        let o = open_folder(&folder_daemon("[]", "[]", 409).api("p"), &dir).unwrap();
        assert!(o.started.is_none());
        let err = open_folder(&folder_daemon("[]", "[]", 500).api("p"), &dir).unwrap_err();
        assert!(matches!(err, Error::Http { status: 500, .. }));
    }

    #[test]
    fn install_reports_each_step_and_the_daemons_own_error_text() {
        let d = FakeDaemon::start(|req| match req.path.as_str() {
            "/api/hooks/install" => (
                200,
                r#"{"ok":true,"installed":true,"alreadyInstalled":true}"#.into(),
            ),
            _ => (
                422,
                r#"{"error":"settings.json: `statusLine` is not an object"}"#.into(),
            ),
        });
        assert_eq!(
            install(&d.api("p")),
            [
                "hooks: already installed",
                "statusline: FAILED: settings.json: `statusLine` is not an object"
            ]
        );
        let d = FakeDaemon::start(|_| {
            (
                200,
                r#"{"ok":true,"installed":true,"alreadyInstalled":false}"#.into(),
            )
        });
        assert_eq!(
            install(&d.api("p")),
            ["hooks: installed", "statusline: installed"]
        );
    }

    fn prune_daemon() -> FakeDaemon {
        FakeDaemon::start(|req| {
            match (req.method.as_str(), req.path.as_str()) {
            ("GET", "/api/workspaces") => (
                200,
                r#"[{"name":"live"},{"name":"crashed"},{"name":"has-history"},{"name":"empty-b"},{"name":"empty-a"}]"#.into(),
            ),
            ("GET", "/api/sessions") => (
                200,
                r#"[{"id":"garage/live/claude-1","workspace":"live"},{"id":"garage/crashed/claude-1","workspace":"crashed","restorable":true}]"#.into(),
            ),
            ("GET", "/api/workspaces/has-history/closed") => (200, r#"[{"id":"garage/has-history/x"}]"#.into()),
            ("GET", p) if p.ends_with("/closed") => (200, "[]".into()),
            ("DELETE", "/api/workspaces/empty-b?if_empty=1&min_age_secs=600") => (404, "{}".into()),
            ("DELETE", _) => (204, "".into()),
            _ => (500, "{}".into()),
        }
        })
    }

    #[test]
    fn the_plan_lists_only_projects_with_no_session_and_no_closed_chat() {
        let d = prune_daemon();
        let plan = plan_prune(&d.api("p")).unwrap();
        assert_eq!(plan.remove, ["empty-a", "empty-b"]);
        assert_eq!(plan.keep_for_history, ["has-history"]);
        assert!(
            d.log().iter().all(|l| l.starts_with("GET")),
            "planning removes nothing: {:?}",
            d.log()
        );
    }

    #[test]
    fn pruning_asks_the_daemon_to_remove_each_only_if_empty_and_not_brand_new() {
        let d = prune_daemon();
        let done = prune(&d.api("p"), &["empty-a".into(), "empty-b".into()]).unwrap();
        assert_eq!(
            done,
            [
                ("empty-a".to_owned(), Pruned::Removed),
                ("empty-b".to_owned(), Pruned::Removed)
            ],
            "404 (gone already) is done"
        );
        let deletes: Vec<_> = d
            .log()
            .into_iter()
            .filter(|l| l.starts_with("DELETE"))
            .collect();
        assert_eq!(
            deletes,
            [
                "DELETE /api/workspaces/empty-a?if_empty=1&min_age_secs=600",
                "DELETE /api/workspaces/empty-b?if_empty=1&min_age_secs=600"
            ]
        );
    }

    #[test]
    fn a_project_that_filled_up_or_is_brand_new_is_reported_kept_with_the_daemons_reason() {
        let d = FakeDaemon::start(|req| {
            if req.path.starts_with("/api/workspaces/fresh") {
                (
                    409,
                    r#"{"error":"fresh was registered 30s ago","reason":"too-new"}"#.into(),
                )
            } else {
                (
                    409,
                    r#"{"error":"busy has 1 live session(s)","reason":"live"}"#.into(),
                )
            }
        });
        let done = prune(&d.api("p"), &["fresh".into(), "busy".into()]).unwrap();
        assert_eq!(
            done,
            [
                (
                    "fresh".to_owned(),
                    Pruned::Kept("fresh was registered 30s ago".into())
                ),
                (
                    "busy".to_owned(),
                    Pruned::Kept("busy has 1 live session(s)".into())
                ),
            ]
        );
    }

    #[test]
    fn nothing_is_removed_without_yes_or_a_person_to_ask() {
        assert_eq!(confirm_mode(true, false), Confirm::Go);
        assert_eq!(confirm_mode(true, true), Confirm::Go);
        assert_eq!(confirm_mode(false, true), Confirm::Ask);
        assert_eq!(
            confirm_mode(false, false),
            Confirm::Refuse,
            "a script that forgot --yes removes nothing"
        );
        assert!(is_yes(" Y\n") && is_yes("yes"));
        assert!(!is_yes("") && !is_yes("no") && !is_yes("sure"));
    }
}
