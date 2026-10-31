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
use serde_json::{json, Value};

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
}

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
    let vars = Vars { plugin_id: &route.plugin_id, plugin_dir: &route.plugin_dir, state_dir: &state_dir, platform: &platform };
    let mut full = argv::substitute(&route.command, &vars);
    full.extend(route.args.iter().cloned());
    let mut env = base_env(parent_env);
    let mut set = |k: &str, v: String| env.push((k.to_owned(), v));
    set("GARAGE_API", format!("http://127.0.0.1:{port}"));
    set("GARAGE_API_VERSION", "1".into());
    set("GARAGE_PLUGIN_ID", route.plugin_id.clone());
    set("GARAGE_PLUGIN_VERSION", route.plugin_version.clone());
    set("GARAGE_PLUGIN_DIR", route.plugin_dir.to_string_lossy().into_owned());
    set("GARAGE_PLUGIN_STATE_DIR", state_dir.to_string_lossy().into_owned());
    if let Ok(t) = std::fs::read_to_string(paths.token(&route.plugin_id)) {
        set("GARAGE_PLUGIN_TOKEN", t.trim().to_owned());
    }
    set("GARAGE_TRIGGER", "cli".into());
    set("GARAGE_CONTEXT", context(route, &state_dir, argv, cwd).to_string());
    (full, env)
}

/// Run the plugin with the terminal's stdio and return its exit code.
pub fn run(paths: &Paths, port: u16, route: &Route, argv: &[String]) -> i32 {
    let cwd = std::env::current_dir().unwrap_or_default();
    let (full, env) = prepare(paths, port, route, argv, &cwd, std::env::vars());
    let _ = std::fs::create_dir_all(paths.data(&route.plugin_id));
    let Some((program, args)) = full.split_first() else {
        eprintln!("plugin {}: empty command", route.plugin_id);
        return 2;
    };
    let program = argv::resolve_program(program, &route.plugin_dir);
    let status = Command::new(&program).args(args).env_clear().envs(env).current_dir(&cwd).status();
    match status {
        Ok(s) => exit_code(s),
        Err(e) => {
            eprintln!("plugin {}: cannot start {}: {e}", route.plugin_id, program.display());
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
        link_plugin(&paths, d.path(), "usage", &cli("usage", "\"{bin}\", \"cli\"", false), true);
        link_plugin(&paths, d.path(), "opener", &cli("open", "\"opener\"", true), true);
        let all = load_all(&paths).unwrap();
        let r = route(&all, &argv(&["usage", "--week", "x"])).unwrap();
        assert_eq!((r.plugin_id.as_str(), r.fallback, r.args.clone()), ("usage", false, argv(&["--week", "x"])));
        let r = route(&all, &argv(&["open", "a"])).unwrap();
        assert_eq!((r.plugin_id.as_str(), r.args.clone()), ("opener", argv(&["a"])), "its own name, not the fallback path");
        let r = route(&all, &argv(&["/home/me/app", "-x"])).unwrap();
        assert!(r.fallback);
        assert_eq!(r.plugin_id, "opener");
        assert_eq!(r.args, argv(&["/home/me/app", "-x"]), "the unknown word is included");
        assert_eq!(route(&all, &[]), None);
    }

    #[test]
    fn without_a_fallback_an_unknown_word_has_no_route_so_clap_reports_it() {
        let d = tempfile::tempdir().unwrap();
        let paths = Paths::new(d.path());
        link_plugin(&paths, d.path(), "usage", &cli("usage", "\"x\"", false), true);
        assert_eq!(route(&load_all(&paths).unwrap(), &argv(&["nope"])), None);
    }

    #[test]
    fn a_second_fallback_is_refused_and_never_receives_words_and_unapproved_or_disabled_ones_do_not_route() {
        let d = tempfile::tempdir().unwrap();
        let paths = Paths::new(d.path());
        link_plugin(&paths, d.path(), "one-fb", &cli("one", "\"a\"", true), true);
        link_plugin(&paths, d.path(), "two-fb", &cli("two", "\"b\"", true), true);
        link_plugin(&paths, d.path(), "raw", &cli("raw", "\"c\"", false), false);
        link_plugin(&paths, d.path(), "off", &cli("off", "\"d\"", false), true);
        garage_daemon::plugins::admin::set_enabled(&paths, "off", false).unwrap();
        let all = load_all(&paths).unwrap();
        assert_eq!(route(&all, &argv(&["/some/dir"])).unwrap().plugin_id, "one-fb");
        let r = route(&all, &argv(&["two"])).unwrap();
        assert!((r.plugin_id.as_str(), r.fallback) == ("one-fb", true), "the refused plugin's own name is dead: the word falls to the fallback holder");
        for (word, why) in [("raw", "not approved"), ("off", "disabled")] {
            let r = route(&all, &argv(&[word])).unwrap();
            assert!((r.plugin_id.as_str(), r.fallback) == ("one-fb", true), "{why}: its name is dead, the word falls to the fallback");
        }
        // the same word cannot be claimed twice either
        let d = tempfile::tempdir().unwrap();
        let paths = Paths::new(d.path());
        link_plugin(&paths, d.path(), "aa", &cli("same", "\"a\"", false), true);
        link_plugin(&paths, d.path(), "bb", &cli("same", "\"b\"", false), true);
        assert_eq!(route(&load_all(&paths).unwrap(), &argv(&["same"])).unwrap().plugin_id, "aa");
    }

    #[test]
    fn the_run_gets_substituted_argv_a_clean_environment_and_a_cli_context() {
        let d = tempfile::tempdir().unwrap();
        let paths = Paths::new(d.path());
        link_plugin(&paths, d.path(), "usage", &cli("usage", "\"{plugin_dir}/bin/u\", \"--state={state_dir}\"", false), true);
        std::fs::write(d.path().join("ui-token"), "SECRET-UI").unwrap();
        let all = load_all(&paths).unwrap();
        let r = route(&all, &argv(&["usage", "show"])).unwrap();
        let cwd = Path::new("/work/here");
        let parent = [("PATH", "/bin"), ("HOME", "/h"), ("AWS_SECRET", "no"), ("GARAGE_TOKEN", "no")].map(|(k, v)| (k.to_owned(), v.to_owned()));
        let (full, env) = prepare(&paths, 4747, &r, &argv(&["usage", "show"]), cwd, parent.into_iter());
        assert_eq!(full[0], format!("{}/bin/u", r.plugin_dir.display()));
        assert_eq!(full[1], format!("--state={}", paths.data("usage").display()));
        assert_eq!(full[2], "show");
        let get = |k: &str| env.iter().find(|(n, _)| n == k).map(|(_, v)| v.clone());
        assert_eq!(get("PATH").as_deref(), Some("/bin"));
        assert_eq!(get("AWS_SECRET"), None);
        assert_eq!(get("GARAGE_TOKEN"), None);
        assert_eq!(get("GARAGE_API").as_deref(), Some("http://127.0.0.1:4747"));
        assert_eq!(get("GARAGE_TRIGGER").as_deref(), Some("cli"));
        assert_eq!(get("GARAGE_PLUGIN_TOKEN").map(|t| t.len()), Some(64));
        assert!(!env.iter().any(|(_, v)| v.contains("SECRET-UI")), "the UI token never reaches a plugin");
        let ctx: Value = serde_json::from_str(&get("GARAGE_CONTEXT").unwrap()).unwrap();
        assert_eq!(ctx["trigger"]["kind"], "cli");
        assert_eq!(ctx["trigger"]["args"]["argv"], json!(["usage", "show"]));
        assert_eq!(ctx["focus"], json!({"workspace": null, "session": null, "dir": "/work/here"}));
        assert_eq!(ctx["plugin"]["id"], "usage");
    }

    #[cfg(unix)]
    #[test]
    fn a_real_run_passes_the_exit_code_through_and_reports_a_missing_program() {
        let d = tempfile::tempdir().unwrap();
        let paths = Paths::new(d.path());
        let out = d.path().join("seen.txt");
        let script = format!("\"sh\", \"-c\", \"echo $GARAGE_PLUGIN_ID:$1 > {}; exit 7\", \"sh\"", out.display());
        link_plugin(&paths, d.path(), "runner", &cli("go", &script, false), true);
        let all = load_all(&paths).unwrap();
        let r = route(&all, &argv(&["go", "word"])).unwrap();
        assert_eq!(run(&paths, 4747, &r, &argv(&["go", "word"])), 7);
        assert_eq!(std::fs::read_to_string(&out).unwrap().trim(), "runner:word");
        link_plugin(&paths, d.path(), "lost", &cli("lost", "\"./nope\"", false), true);
        let r = route(&load_all(&paths).unwrap(), &argv(&["lost"])).unwrap();
        assert_eq!(run(&paths, 4747, &r, &argv(&["lost"])), 127);
    }
}
