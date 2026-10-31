//! `[[cli]]` dispatch: `claude-garage <word> ...` where `<word>` is not a core
//! subcommand. Core names always win (clap resolves them first); here the
//! word is looked up among the usable plugins' `[[cli]]` names, and failing
//! that the single `fallback = true` plugin receives the whole argv (so
//! `claude-garage ~/dev/app` can be handled). With neither, the caller shows
//! the usual unknown-subcommand error.

use std::path::{Path, PathBuf};
use std::process::Command;

use garage_daemon::plugins::layout::Paths;
use garage_daemon::plugins::runner::base_env;
use garage_plugin::argv::{self, Vars};
use serde_json::{Value, json};

use super::registry::Loaded;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Route {
    pub plugin_id: String,
    pub plugin_version: String,
    pub plugin_dir: PathBuf,
    /// The `[[cli]]` entry's argv, before substitution.
    pub command: Vec<String>,
    /// Words appended after the command: the rest after a matched name, or the
    /// whole argv for the fallback.
    pub args: Vec<String>,
    pub fallback: bool,
    /// The `[[cli]]` entry asked for a running daemon.
    pub needs_daemon: bool,
}

/// The `claude-garage` settings a CLI plugin's own `claude-garage` child needs.
const CONFIG_ENV: &[&str] = &[
    "GARAGE_PORT",
    "GARAGE_DIR",
    "GARAGE_TMUX_SOCKET",
    "GARAGE_TMUX_CONTROL",
    "GARAGE_BACKEND",
];

/// Who handles `argv` (the words after `claude-garage`)?
pub fn route(all: &[Loaded], argv: &[String]) -> Option<Route> {
    let first = argv.first()?;
    let usable = || all.iter().filter(|l| l.usable());
    for l in usable() {
        let m = l.manifest.as_ref()?;
        if let Some(c) = m.cli.iter().find(|c| &c.name == first) {
            return Some(Route {
                plugin_id: m.id.clone(),
                plugin_version: m.version.clone(),
                plugin_dir: l.dir.clone(),
                command: c.command.clone(),
                args: argv[1..].to_vec(),
                fallback: false,
                needs_daemon: c.needs_daemon,
            });
        }
    }
    for l in usable() {
        let m = l.manifest.as_ref()?;
        if let Some(c) = m.cli.iter().find(|c| c.fallback) {
            return Some(Route {
                plugin_id: m.id.clone(),
                plugin_version: m.version.clone(),
                plugin_dir: l.dir.clone(),
                command: c.command.clone(),
                args: argv.to_vec(),
                fallback: true,
                needs_daemon: c.needs_daemon,
            });
        }
    }
    None
}

/// The context JSON of a CLI run (same shape as every other run; `focus.dir`
/// is the caller's working directory and there is no session).
pub fn context(route: &Route, state_dir: &Path, argv: &[String], cwd: &Path) -> Value {
    json!({
        "api_version": 1,
        "plugin": {"id": route.plugin_id, "version": route.plugin_version, "state_dir": state_dir},
        "trigger": {"kind": "cli", "key": null, "slot": null, "action": null, "args": {"argv": argv}},
        "focus": {"workspace": null, "session": null, "dir": cwd},
        "selection": {"session": null, "index": null, "view": null},
        "worktree": null,
        "event": null,
        "ui": {"slots": {}},
    })
}

/// The argv and environment of a CLI run. The environment starts from the same
/// allowlist as daemon-run commands; the UI token is never included.
pub fn prepare(
    paths: &Paths,
    port: u16,
    route: &Route,
    argv: &[String],
    cwd: &Path,
    parent_env: impl Iterator<Item = (String, String)>,
) -> (Vec<String>, Vec<(String, String)>) {
    let state_dir = paths.data(&route.plugin_id);
    let platform = argv::platform_id();
    let vars = Vars {
        plugin_id: &route.plugin_id,
        plugin_dir: &route.plugin_dir,
        state_dir: &state_dir,
        platform: &platform,
    };
    let mut full = argv::substitute(&route.command, &vars);
    full.extend(route.args.iter().cloned());
    let parent: Vec<(String, String)> = parent_env.collect();
    let mut env = base_env(parent.iter().cloned());
    // A CLI plugin may start `claude-garage` itself (`GARAGE_BIN`): it must
    // find the same daemon and state this command found. These are
    // settings, not secrets.
    env.extend(
        parent
            .into_iter()
            .filter(|(k, _)| CONFIG_ENV.contains(&k.as_str())),
    );
    let mut set = |k: &str, v: String| env.push((k.to_owned(), v));
    set("GARAGE_API", format!("http://127.0.0.1:{port}"));
    set("GARAGE_API_VERSION", "1".into());
    set("GARAGE_PLUGIN_ID", route.plugin_id.clone());
    set("GARAGE_PLUGIN_VERSION", route.plugin_version.clone());
    set(
        "GARAGE_PLUGIN_DIR",
        route.plugin_dir.to_string_lossy().into_owned(),
    );
    set(
        "GARAGE_PLUGIN_STATE_DIR",
        state_dir.to_string_lossy().into_owned(),
    );
    if let Ok(t) = std::fs::read_to_string(paths.token(&route.plugin_id)) {
        set("GARAGE_PLUGIN_TOKEN", t.trim().to_owned());
    }
    // How a CLI plugin asks for the wall: it writes `{"open_wall": true}`
    // here and exits 0; the launcher then opens the wall in its own
    // environment. The plugin never starts the wall and nothing of this run's
    // environment (token, context) reaches it.
    set(
        "GARAGE_HANDOVER",
        handover_path(&state_dir).to_string_lossy().into_owned(),
    );
    set("GARAGE_TRIGGER", "cli".into());
    set(
        "GARAGE_CONTEXT",
        context(route, &state_dir, argv, cwd).to_string(),
    );
    (full, env)
}

/// Where a run of this process leaves its hand-over request.
fn handover_path(state_dir: &Path) -> PathBuf {
    state_dir.join(format!("handover-{}.json", std::process::id()))
}

/// Did the plugin ask for the wall? Reads and removes the hand-over file; only
/// `{"open_wall": true}` counts, anything else (or no file) is no.
fn take_handover(path: &Path) -> bool {
    let asked = std::fs::read_to_string(path)
        .ok()
        .and_then(|t| serde_json::from_str::<Value>(&t).ok())
        .is_some_and(|v| v.get("open_wall") == Some(&Value::Bool(true)));
    let _ = std::fs::remove_file(path);
    asked
}

/// How a CLI plugin run ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Ran {
    pub code: i32,
    /// The plugin finished (exit 0) and asked for the wall.
    pub open_wall: bool,
}

/// Run the plugin with the terminal's stdio.
pub fn run(paths: &Paths, port: u16, route: &Route, argv: &[String]) -> Ran {
    let cwd = std::env::current_dir().unwrap_or_default();
    let (full, env) = prepare(paths, port, route, argv, &cwd, std::env::vars());
    let state_dir = paths.data(&route.plugin_id);
    let _ = std::fs::create_dir_all(&state_dir);
    let handover = handover_path(&state_dir);
    let _ = std::fs::remove_file(&handover);
    let code = run_command(route, &full, env, &cwd);
    let open_wall = take_handover(&handover) && code == 0;
    Ran { code, open_wall }
}

fn run_command(route: &Route, full: &[String], env: Vec<(String, String)>, cwd: &Path) -> i32 {
    let Some((program, args)) = full.split_first() else {
        eprintln!("plugin {}: empty command", route.plugin_id);
        return 2;
    };
    let program = argv::resolve_program(program, &route.plugin_dir);
    let status = Command::new(&program)
        .args(args)
        .env_clear()
        .envs(env)
        .current_dir(cwd)
        .status();
    match status {
        Ok(s) => exit_code(s),
        Err(e) => {
            eprintln!(
                "plugin {}: cannot start {}: {e}",
                route.plugin_id,
                program.display()
            );
            127
        }
    }
}

fn exit_code(s: std::process::ExitStatus) -> i32 {
    if let Some(c) = s.code() {
        return c;
    }
    #[cfg(unix)]
    {
        use std::os::unix::process::ExitStatusExt;
        if let Some(sig) = s.signal() {
            return 128 + sig;
        }
    }
    1
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plugin::registry::load_all;
    use crate::plugin::testkit::link_plugin;

    fn argv(words: &[&str]) -> Vec<String> {
        words.iter().map(|s| s.to_string()).collect()
    }

    fn cli(name: &str, cmd: &str, fb: bool) -> String {
        format!("[[cli]]\nname = \"{name}\"\ncommand = [{cmd}]\nfallback = {fb}\n")
    }

    #[test]
    fn a_named_word_goes_to_its_plugin_with_the_rest_and_the_fallback_gets_everything() {
        let d = tempfile::tempdir().unwrap();
        let paths = Paths::new(d.path());
        link_plugin(
            &paths,
            d.path(),
            "usage",
            &cli("usage", "\"{bin}\", \"cli\"", false),
            true,
        );
        link_plugin(
            &paths,
            d.path(),
            "opener",
            &cli("open", "\"opener\"", true),
            true,
        );
        let all = load_all(&paths).unwrap();
        let r = route(&all, &argv(&["usage", "--week", "x"])).unwrap();
        assert_eq!(
            (r.plugin_id.as_str(), r.fallback, r.args.clone()),
            ("usage", false, argv(&["--week", "x"]))
        );
        let r = route(&all, &argv(&["open", "a"])).unwrap();
        assert_eq!(
            (r.plugin_id.as_str(), r.args.clone()),
            ("opener", argv(&["a"])),
            "its own name, not the fallback path"
        );
        let r = route(&all, &argv(&["/home/me/app", "-x"])).unwrap();
        assert!(r.fallback);
        assert_eq!(r.plugin_id, "opener");
        assert_eq!(
            r.args,
            argv(&["/home/me/app", "-x"]),
            "the unknown word is included"
        );
        assert_eq!(route(&all, &[]), None);
    }

    #[test]
    fn the_shipped_cli_extras_manifest_routes_folders_install_and_prune() {
        let real = std::fs::read_to_string(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../plugins/cli-extras/garage-plugin.toml"),
        )
        .unwrap();
        // the test kit writes the header; the shipped body starts at its permissions
        let body = &real[real.find("permissions").unwrap()..];
        let d = tempfile::tempdir().unwrap();
        let paths = Paths::new(d.path());
        link_plugin(&paths, d.path(), "cli-extras", body, true);
        let all = load_all(&paths).unwrap();

        let r = route(&all, &argv(&["~/dev/app", "--no-open"])).unwrap();
        assert!((r.plugin_id.as_str(), r.fallback) == ("cli-extras", true));
        assert_eq!(
            r.args,
            argv(&["~/dev/app", "--no-open"]),
            "the word is included"
        );
        assert!(r.command.ends_with(&argv(&["open"])));
        assert!(r.needs_daemon, "the folder command needs the daemon up");

        let r = route(&all, &argv(&["open", "~/dev/app"])).unwrap();
        assert!(!r.fallback && r.args == argv(&["~/dev/app"]));

        let r = route(&all, &argv(&["install"])).unwrap();
        assert!(!r.fallback && r.command.ends_with(&argv(&["install"])) && r.needs_daemon);
        let r = route(&all, &argv(&["prune", "--dry-run"])).unwrap();
        assert_eq!(r.args, argv(&["--dry-run"]));
        assert!(r.command.ends_with(&argv(&["prune"])));
    }

    #[test]
    fn without_a_fallback_an_unknown_word_has_no_route_so_clap_reports_it() {
        let d = tempfile::tempdir().unwrap();
        let paths = Paths::new(d.path());
        link_plugin(
            &paths,
            d.path(),
            "usage",
            &cli("usage", "\"x\"", false),
            true,
        );
        assert_eq!(route(&load_all(&paths).unwrap(), &argv(&["nope"])), None);
    }

    #[test]
    fn a_second_fallback_is_refused_and_never_receives_words_and_unapproved_or_disabled_ones_do_not_route()
     {
        let d = tempfile::tempdir().unwrap();
        let paths = Paths::new(d.path());
        link_plugin(&paths, d.path(), "one-fb", &cli("one", "\"a\"", true), true);
        link_plugin(&paths, d.path(), "two-fb", &cli("two", "\"b\"", true), true);
        link_plugin(&paths, d.path(), "raw", &cli("raw", "\"c\"", false), false);
        link_plugin(&paths, d.path(), "off", &cli("off", "\"d\"", false), true);
        garage_daemon::plugins::admin::set_enabled(&paths, "off", false).unwrap();
        let all = load_all(&paths).unwrap();
        assert_eq!(
            route(&all, &argv(&["/some/dir"])).unwrap().plugin_id,
            "one-fb"
        );
        let r = route(&all, &argv(&["two"])).unwrap();
        assert!(
            (r.plugin_id.as_str(), r.fallback) == ("one-fb", true),
            "the refused plugin's own name is dead: the word falls to the fallback holder"
        );
        for (word, why) in [("raw", "not approved"), ("off", "disabled")] {
            let r = route(&all, &argv(&[word])).unwrap();
            assert!(
                (r.plugin_id.as_str(), r.fallback) == ("one-fb", true),
                "{why}: its name is dead, the word falls to the fallback"
            );
        }
        // the same word cannot be claimed twice either
        let d = tempfile::tempdir().unwrap();
        let paths = Paths::new(d.path());
        link_plugin(&paths, d.path(), "aa", &cli("same", "\"a\"", false), true);
        link_plugin(&paths, d.path(), "bb", &cli("same", "\"b\"", false), true);
        assert_eq!(
            route(&load_all(&paths).unwrap(), &argv(&["same"]))
                .unwrap()
                .plugin_id,
            "aa"
        );
    }

    #[test]
    fn the_run_gets_substituted_argv_a_clean_environment_and_a_cli_context() {
        let d = tempfile::tempdir().unwrap();
        let paths = Paths::new(d.path());
        link_plugin(
            &paths,
            d.path(),
            "usage",
            &cli(
                "usage",
                "\"{plugin_dir}/bin/u\", \"--state={state_dir}\"",
                false,
            ),
            true,
        );
        std::fs::write(d.path().join("ui-token"), "SECRET-UI").unwrap();
        let all = load_all(&paths).unwrap();
        let r = route(&all, &argv(&["usage", "show"])).unwrap();
        let cwd = Path::new("/work/here");
        let parent = [
            ("PATH", "/bin"),
            ("HOME", "/h"),
            ("AWS_SECRET", "no"),
            ("GARAGE_PORT", "5151"),
            ("GARAGE_DIR", "/custom/garage"),
            ("GARAGE_TOKEN", "no"),
        ]
        .map(|(k, v)| (k.to_owned(), v.to_owned()));
        let (full, env) = prepare(
            &paths,
            4747,
            &r,
            &argv(&["usage", "show"]),
            cwd,
            parent.into_iter(),
        );
        assert_eq!(full[0], format!("{}/bin/u", r.plugin_dir.display()));
        assert_eq!(
            full[1],
            format!("--state={}", paths.data("usage").display())
        );
        assert_eq!(full[2], "show");
        let get = |k: &str| env.iter().find(|(n, _)| n == k).map(|(_, v)| v.clone());
        assert_eq!(get("PATH").as_deref(), Some("/bin"));
        assert_eq!(get("AWS_SECRET"), None);
        assert_eq!(
            (get("GARAGE_PORT").as_deref(), get("GARAGE_DIR").as_deref()),
            (Some("5151"), Some("/custom/garage")),
            "a plugin's own claude-garage child finds the same daemon"
        );
        assert_eq!(get("GARAGE_TOKEN"), None);
        assert_eq!(get("GARAGE_API").as_deref(), Some("http://127.0.0.1:4747"));
        assert_eq!(get("GARAGE_TRIGGER").as_deref(), Some("cli"));
        assert_eq!(
            get("GARAGE_BIN"),
            None,
            "a plugin never execs claude-garage"
        );
        assert!(
            get("GARAGE_HANDOVER")
                .is_some_and(|p| p.starts_with(&*paths.data("usage").to_string_lossy())),
            "the hand-over file sits in the plugin's own state dir"
        );
        assert_eq!(get("GARAGE_PLUGIN_TOKEN").map(|t| t.len()), Some(64));
        assert!(
            !env.iter().any(|(_, v)| v.contains("SECRET-UI")),
            "the UI token never reaches a plugin"
        );
        let ctx: Value = serde_json::from_str(&get("GARAGE_CONTEXT").unwrap()).unwrap();
        assert_eq!(ctx["trigger"]["kind"], "cli");
        assert_eq!(ctx["trigger"]["args"]["argv"], json!(["usage", "show"]));
        assert_eq!(
            ctx["focus"],
            json!({"workspace": null, "session": null, "dir": "/work/here"})
        );
        assert_eq!(ctx["plugin"]["id"], "usage");
    }

    #[cfg(unix)]
    #[test]
    fn a_plugin_asks_for_the_wall_through_the_hand_over_file_and_only_when_it_succeeded() {
        let d = tempfile::tempdir().unwrap();
        let paths = Paths::new(d.path());
        // writes whatever $1 says into $GARAGE_HANDOVER, then exits with $2
        let script = "\"sh\", \"-c\", \"printf '%s' \\\"$1\\\" > \\\"$GARAGE_HANDOVER\\\"; exit $2\", \"sh\"";
        link_plugin(&paths, d.path(), "runner", &cli("go", script, false), true);
        let all = load_all(&paths).unwrap();
        let ran = |body: &str, code: &str| {
            let a = argv(&["go", body, code]);
            let r = route(&all, &a).unwrap();
            run(&paths, 4747, &r, &a)
        };
        assert_eq!(
            ran(r#"{"open_wall": true}"#, "0"),
            Ran {
                code: 0,
                open_wall: true
            }
        );
        assert_eq!(
            ran(r#"{"open_wall": true}"#, "3"),
            Ran {
                code: 3,
                open_wall: false
            },
            "a failed run never opens the wall"
        );
        for junk in [
            r#"{"open_wall": "yes"}"#,
            r#"{"open_wall": false}"#,
            "true",
            "not json",
        ] {
            assert!(!ran(junk, "0").open_wall, "{junk}");
        }
        let a = argv(&["go", "", "0"]);
        let r = route(&all, &a).unwrap();
        assert!(
            !run(&paths, 4747, &r, &a).open_wall,
            "an empty file asks for nothing"
        );
        // a request left behind by an earlier run does not count for this one
        link_plugin(
            &paths,
            d.path(),
            "quiet",
            &cli("quiet", "\"true\"", false),
            true,
        );
        let all = load_all(&paths).unwrap();
        let a = argv(&["quiet"]);
        let r = route(&all, &a).unwrap();
        std::fs::create_dir_all(paths.data("quiet")).unwrap();
        std::fs::write(
            handover_path(&paths.data("quiet")),
            r#"{"open_wall": true}"#,
        )
        .unwrap();
        assert!(!run(&paths, 4747, &r, &a).open_wall);
    }

    #[cfg(unix)]
    #[test]
    fn a_real_run_passes_the_exit_code_through_and_reports_a_missing_program() {
        let d = tempfile::tempdir().unwrap();
        let paths = Paths::new(d.path());
        let out = d.path().join("seen.txt");
        let script = format!(
            "\"sh\", \"-c\", \"echo $GARAGE_PLUGIN_ID:$1 > {}; exit 7\", \"sh\"",
            out.display()
        );
        link_plugin(&paths, d.path(), "runner", &cli("go", &script, false), true);
        let all = load_all(&paths).unwrap();
        let r = route(&all, &argv(&["go", "word"])).unwrap();
        let ran = run(&paths, 4747, &r, &argv(&["go", "word"]));
        assert_eq!((ran.code, ran.open_wall), (7, false));
        assert_eq!(std::fs::read_to_string(&out).unwrap().trim(), "runner:word");
        link_plugin(
            &paths,
            d.path(),
            "lost",
            &cli("lost", "\"./nope\"", false),
            true,
        );
        let r = route(&load_all(&paths).unwrap(), &argv(&["lost"])).unwrap();
        assert_eq!(run(&paths, 4747, &r, &argv(&["lost"])).code, 127);
    }
}
