//! The `claude-garage plugin ...` subcommands and the glue from clap to the
//! flows in [`super::install`]: real terminal, real git, real `curl`, and the
//! daemon-or-direct registry writer.

use std::io::{BufRead, IsTerminal, Write};
use std::path::PathBuf;
use std::time::Duration;

use clap::Subcommand;
use garage_daemon::plugins::layout::{Paths, PluginsFile};
use serde_json::{Value, json};

use super::admin::{Admin, choose};
use super::dev::{LogTail, Watcher};
use super::external;
use super::fetch::{CurlFetch, ProcessBuilder};
use super::install::{self, Cx, Opts, Outcome, Ui};
use super::registry::{Loaded, load_all};
use super::scaffold::{self, Lang, NewPlugin, SdkSource};
use super::search::{self, Sort};
use super::source::{GITHUB, RealGit};

#[derive(Subcommand, Debug, PartialEq, Eq)]
pub enum PluginCmd {
    /// List installed plugins, their source, state and permissions.
    List {
        #[arg(long)]
        json: bool,
    },
    /// Register a local folder as a plugin (no build; also approves added permissions).
    Link {
        path: PathBuf,
        #[arg(long)]
        yes: bool,
        #[arg(long)]
        allow_dangerous: bool,
    },
    /// Install from GitHub: owner/repo[/subdir].
    Install {
        spec: String,
        /// Tag, branch or full commit sha.
        #[arg(long = "ref")]
        git_ref: Option<String>,
        #[arg(long)]
        yes: bool,
        #[arg(long)]
        allow_dangerous: bool,
    },
    /// Update GitHub plugins; asks again only when commands, permissions or URLs changed.
    Update {
        id: Option<String>,
        #[arg(long)]
        all: bool,
        #[arg(long)]
        yes: bool,
        #[arg(long)]
        allow_dangerous: bool,
    },
    /// Remove a plugin; its data and logs are kept unless --purge.
    Remove {
        id: String,
        #[arg(long)]
        purge: bool,
    },
    Enable {
        id: String,
    },
    Disable {
        id: String,
    },
    /// Bind a key to a plugin action (`plugin:action`), or --clear it.
    Bind {
        action: String,
        key: Option<String>,
        #[arg(long)]
        clear: bool,
    },
    /// Create a working plugin in ./<name> (manifest, code, README, CLAUDE.md, a test).
    New {
        /// The plugin id, also the folder name: lowercase letters, digits and '-'.
        name: String,
        #[arg(long, value_enum, default_value_t = Lang::Rust)]
        lang: Lang,
        /// Use a local checkout of the Rust SDK instead of the tagged release.
        #[arg(long, hide = true)]
        sdk_path: Option<PathBuf>,
    },
    /// Link a plugin folder, rebuild and reload it when files change, and tail its log.
    Dev {
        /// The plugin folder (default: the current folder).
        #[arg(default_value = ".")]
        path: PathBuf,
        #[arg(long)]
        yes: bool,
        #[arg(long)]
        allow_dangerous: bool,
    },
    /// Find plugins: public GitHub repositories with the topic claude-garage-plugin (unreviewed).
    Search {
        /// Words to look for in names, descriptions and READMEs.
        query: Vec<String>,
        #[arg(long)]
        json: bool,
        #[arg(long, value_enum, default_value_t = Sort::Stars)]
        sort: Sort,
        #[arg(long, default_value_t = 20)]
        limit: u32,
    },
    /// Print the JSON schema of garage-plugin.toml.
    Schema,
    /// Show a plugin's log.
    Logs {
        id: String,
        #[arg(short = 'f', long)]
        follow: bool,
        #[arg(short = 'n', long, default_value_t = 100)]
        lines: usize,
    },
}

/// The real terminal.
pub struct RealUi {
    interactive: bool,
}

impl RealUi {
    pub fn new() -> Self {
        Self {
            interactive: std::io::stdin().is_terminal() && std::io::stdout().is_terminal(),
        }
    }
}

impl Ui for RealUi {
    fn say(&mut self, text: &str) {
        println!("{text}");
    }
    fn interactive(&self) -> bool {
        self.interactive
    }
    fn confirm(&mut self, question: &str) -> bool {
        print!("{question} [y/N] ");
        let _ = std::io::stdout().flush();
        let mut line = String::new();
        std::io::stdin().lock().read_line(&mut line).is_ok()
            && matches!(line.trim().to_ascii_lowercase().as_str(), "y" | "yes")
    }
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// One `plugin list` row.
#[derive(Debug, Clone, PartialEq)]
pub struct Row {
    pub id: String,
    pub version: String,
    pub source: String,
    pub enabled: bool,
    pub state: String,
    pub permissions: Vec<String>,
    pub dangerous: Vec<String>,
    pub problems: Vec<String>,
}

pub fn rows(all: &[Loaded]) -> Vec<Row> {
    all.iter()
        .map(|l| {
            let permissions = l
                .manifest
                .as_ref()
                .map(|m| m.permission_names())
                .or_else(|| l.entry.granted_permissions.clone())
                .unwrap_or_default();
            Row {
                id: l.entry.id.clone(),
                version: l
                    .manifest
                    .as_ref()
                    .map(|m| m.version.clone())
                    .unwrap_or_else(|| "?".into()),
                source: l.source_label(),
                enabled: l.entry.enabled,
                state: l.state().to_owned(),
                dangerous: permissions
                    .iter()
                    .filter(|p| garage_plugin::features::is_dangerous(p))
                    .cloned()
                    .collect(),
                permissions,
                problems: l
                    .problems
                    .iter()
                    .cloned()
                    .chain(l.refused.clone())
                    .collect(),
            }
        })
        .collect()
}

pub fn list_json(rows: &[Row]) -> Value {
    Value::Array(
        rows.iter()
            .map(|r| {
                json!({"id": r.id, "version": r.version, "source": r.source, "enabled": r.enabled, "state": r.state,
                       "permissions": r.permissions, "dangerous": r.dangerous, "problems": r.problems})
            })
            .collect(),
    )
}

pub fn list_text(rows: &[Row]) -> String {
    if rows.is_empty() {
        return "No plugins installed. Try `claude-garage plugin install owner/repo` or `plugin link <folder>`.".into();
    }
    let w = |f: fn(&Row) -> usize, min: usize| rows.iter().map(f).max().unwrap_or(0).max(min);
    let (wi, wv, ws, wt) = (
        w(|r| r.id.len(), 2),
        w(|r| r.version.len(), 7),
        w(|r| r.source.len(), 6),
        w(|r| r.state.len(), 5),
    );
    let mut o = format!(
        "{:<wi$}  {:<wv$}  {:<ws$}  {:<wt$}  PERMISSIONS\n",
        "ID", "VERSION", "SOURCE", "STATE"
    );
    for r in rows {
        let perms: Vec<String> = r
            .permissions
            .iter()
            .map(|p| {
                if r.dangerous.contains(p) {
                    format!("{p}!")
                } else {
                    p.clone()
                }
            })
            .collect();
        let perms = if perms.is_empty() {
            "-".into()
        } else {
            perms.join(" ")
        };
        o.push_str(&format!(
            "{:<wi$}  {:<wv$}  {:<ws$}  {:<wt$}  {perms}\n",
            r.id, r.version, r.source, r.state
        ));
    }
    if rows.iter().any(|r| !r.dangerous.is_empty()) {
        o.push_str("\n! = dangerous permission\n");
    }
    for r in rows.iter().filter(|r| !r.problems.is_empty()) {
        for p in &r.problems {
            o.push_str(&format!("{}: {p}\n", r.id));
        }
    }
    o.trim_end().to_owned()
}

pub fn tail_lines(text: &str, n: usize) -> String {
    let lines: Vec<&str> = text.lines().collect();
    lines[lines.len().saturating_sub(n)..].join("\n")
}

/// `plugin bind`: the id before the first `:` names the plugin.
pub fn split_action(action: &str) -> Result<(&str, &str), String> {
    match action.split_once(':') {
        Some((p, a)) if !p.is_empty() && !a.is_empty() => Ok((p, a)),
        _ => Err(format!(
            "{action:?} is not a full action id; use <plugin>:<action> (see `claude-garage plugin list`)"
        )),
    }
}

fn bind(
    paths: &Paths,
    admin: &dyn Admin,
    action: &str,
    key: Option<&str>,
    clear: bool,
) -> Result<String, String> {
    let (plugin, local) = split_action(action)?;
    let all = load_all(paths)?;
    let Some(l) = all.iter().find(|l| l.entry.id == plugin) else {
        return Err(format!("no plugin {plugin:?}"));
    };
    if let Some(m) = &l.manifest
        && m.action(local).is_none()
    {
        let known: Vec<String> = m
            .actions
            .iter()
            .map(|a| format!("{plugin}:{}", a.id))
            .collect();
        return Err(format!(
            "{plugin} has no action {local:?} (it has: {})",
            if known.is_empty() {
                "none".into()
            } else {
                known.join(", ")
            }
        ));
    }
    if clear {
        admin.bind(plugin, action, None)?;
        return Ok(format!(
            "Cleared the key for {action}; its default key (if any) applies again."
        ));
    }
    let Some(key) = key.filter(|k| !k.trim().is_empty() && !k.contains(char::is_whitespace)) else {
        return Err("give a key, for example `plugin bind usage-bar:refresh u`, or --clear".into());
    };
    admin.bind(plugin, action, Some(key))?;
    Ok(format!(
        "Bound {key} to {action}. A key that a core action already uses wins; the TUI reports conflicts."
    ))
}

fn follow(path: &std::path::Path, lines: usize) -> Result<(), String> {
    let (mut tail, recent) = LogTail::start(path, lines);
    if !recent.is_empty() {
        println!("{recent}");
    }
    loop {
        std::thread::sleep(Duration::from_millis(400));
        let more = tail.poll();
        if !more.is_empty() {
            print!("{more}");
            let _ = std::io::stdout().flush();
        }
    }
}

#[cfg(unix)]
static INTERRUPTED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

#[cfg(unix)]
extern "C" fn on_sigint(_: libc::c_int) {
    INTERRUPTED.store(true, std::sync::atomic::Ordering::SeqCst);
}

fn interrupted() -> bool {
    #[cfg(unix)]
    {
        INTERRUPTED.load(std::sync::atomic::Ordering::SeqCst)
    }
    #[cfg(not(unix))]
    {
        false
    }
}

/// `plugin dev`: run until Ctrl+C. On Windows the default Ctrl+C action ends the process.
fn dev_loop(cx: &mut Cx, path: &std::path::Path, opts: Opts) -> Result<(), String> {
    #[cfg(unix)]
    unsafe {
        libc::signal(libc::SIGINT, on_sigint as *const () as libc::sighandler_t);
    }
    let mut watcher = Watcher::start(cx, path, opts)?;
    cx.ui
        .say("Watching for changes. Press Ctrl+C to stop; the plugin stays linked.");
    while !interrupted() {
        std::thread::sleep(Duration::from_millis(400));
        watcher.step(cx);
    }
    cx.ui.say(
        "Stopped. The plugin is still linked (`claude-garage plugin remove <id>` unlinks it).",
    );
    Ok(())
}

/// Run one `plugin` subcommand; the value is the process exit code.
pub fn run(cmd: PluginCmd) -> Result<i32, String> {
    let config = garage_daemon::Config::from_env().map_err(|e| e.to_string())?;
    let paths = Paths::new(&config.garage_dir);
    match cmd {
        PluginCmd::Schema => {
            println!("{}", garage_plugin::schema::schema_json());
            Ok(0)
        }
        PluginCmd::List { json } => {
            let r = rows(&load_all(&paths)?);
            if json {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&list_json(&r)).expect("json")
                );
            } else {
                println!("{}", list_text(&r));
            }
            Ok(0)
        }
        PluginCmd::Logs {
            id,
            follow: f,
            lines,
        } => {
            let path = paths.log(&id);
            if f {
                follow(&path, lines)?;
                return Ok(0);
            }
            match std::fs::read_to_string(&path) {
                Ok(t) => println!(
                    "{}",
                    garage_protocol::strip_hostile_keep_lines(&tail_lines(&t, lines))
                ),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                    let known = PluginsFile::read(&paths)?
                        .plugins
                        .iter()
                        .any(|p| p.id == id);
                    return Err(if known {
                        format!(
                            "{id} has not logged anything yet ({} does not exist)",
                            path.display()
                        )
                    } else {
                        format!("no plugin {id:?}")
                    });
                }
                Err(e) => return Err(format!("cannot read {}: {e}", path.display())),
            }
            Ok(0)
        }
        PluginCmd::New {
            name,
            lang,
            sdk_path,
        } => {
            let sdk = match sdk_path {
                Some(p) => SdkSource::Path(std::path::absolute(&p).map_err(|e| e.to_string())?),
                None => SdkSource::Git,
            };
            let plugin = NewPlugin {
                name: &name,
                lang,
                sdk,
            };
            let cwd = std::env::current_dir().map_err(|e| e.to_string())?;
            let dir = scaffold::create(&cwd, &plugin)?;
            println!("{}", scaffold::next_steps(&dir, &plugin));
            Ok(0)
        }
        PluginCmd::Dev {
            path,
            yes,
            allow_dangerous,
        } => with_cx(&config, |cx| {
            dev_loop(
                cx,
                &path,
                Opts {
                    yes,
                    allow_dangerous,
                },
            )
            .map(|_| 0)
        }),
        PluginCmd::Search {
            query,
            json,
            sort,
            limit,
        } => {
            let found = search::search(&CurlFetch, search::API, &query, sort, limit)?;
            if json {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&search::render_json(&found)).expect("json")
                );
            } else {
                println!("{}", search::render_text(&found, &query, now_ms() / 1000));
            }
            Ok(0)
        }
        PluginCmd::Enable { id } => {
            let admin = choose(&config.garage_dir, config.port)?;
            admin.set_enabled(&id, true)?;
            println!("Enabled {id} ({}).", admin.via());
            Ok(0)
        }
        PluginCmd::Disable { id } => {
            let admin = choose(&config.garage_dir, config.port)?;
            admin.set_enabled(&id, false)?;
            println!("Disabled {id} ({}).", admin.via());
            Ok(0)
        }
        PluginCmd::Bind { action, key, clear } => {
            let admin = choose(&config.garage_dir, config.port)?;
            println!(
                "{}",
                bind(&paths, admin.as_ref(), &action, key.as_deref(), clear)?
            );
            Ok(0)
        }
        PluginCmd::Link {
            path,
            yes,
            allow_dangerous,
        } => with_cx(&config, |cx| {
            install::link(
                cx,
                &path,
                &Opts {
                    yes,
                    allow_dangerous,
                },
            )
            .map(|_| 0)
        }),
        PluginCmd::Install {
            spec,
            git_ref,
            yes,
            allow_dangerous,
        } => with_cx(&config, |cx| {
            install::install(
                cx,
                &spec,
                git_ref.as_deref(),
                &Opts {
                    yes,
                    allow_dangerous,
                },
            )
            .map(|_| 0)
        }),
        PluginCmd::Update {
            id,
            all,
            yes,
            allow_dangerous,
        } => {
            if id.is_none() && !all {
                return Err("say which plugin to update, or pass --all".into());
            }
            with_cx(&config, |cx| {
                let out = install::update(
                    cx,
                    id.as_deref(),
                    &Opts {
                        yes,
                        allow_dangerous,
                    },
                )?;
                Ok(if out.iter().any(|o| matches!(o, Outcome::Failed { .. })) {
                    1
                } else {
                    0
                })
            })
        }
        PluginCmd::Remove { id, purge } => {
            with_cx(&config, |cx| install::remove(cx, &id, purge).map(|_| 0))
        }
    }
}

fn with_cx(
    config: &garage_daemon::Config,
    f: impl FnOnce(&mut Cx) -> Result<i32, String>,
) -> Result<i32, String> {
    let admin = choose(&config.garage_dir, config.port)?;
    let mut ui = RealUi::new();
    let mut cx = Cx {
        paths: Paths::new(&config.garage_dir),
        git: &RealGit,
        fetch: &CurlFetch,
        builder: &ProcessBuilder,
        admin: admin.as_ref(),
        ui: &mut ui,
        github_base: GITHUB.into(),
        now_ms: now_ms(),
    };
    f(&mut cx)
}

/// An unknown first word: a plugin's `[[cli]]`, the fallback, or `None` for
/// the usual clap error. The exit code is the plugin's.
pub fn dispatch_external(argv: &[String]) -> Result<Option<i32>, String> {
    let config = garage_daemon::Config::from_env().map_err(|e| e.to_string())?;
    let paths = Paths::new(&config.garage_dir);
    let all = match load_all(&paths) {
        Ok(a) => a,
        // An unreadable registry must not hide the plain "unknown subcommand" error.
        Err(e) => {
            eprintln!("warning: {e}");
            return Ok(None);
        }
    };
    let Some(route) = external::route(&all, argv) else {
        return Ok(None);
    };
    let host = crate::host::RealHost { port: config.port };
    let cwd = std::env::current_dir().unwrap_or_default();
    let home = std::env::var_os("HOME").map(std::path::PathBuf::from);
    dispatch_route(&host, &paths, &route, argv, &cwd, home.as_deref())
}

/// Run a routed CLI plugin.
///
/// A fallback route that needs the daemon only ever receives a word that is an
/// existing folder (that is what a fallback is for): the check happens here,
/// BEFORE any daemon is started or replaced, so a mistyped subcommand
/// (`claude-garage instal`) cannot start anything.
///
/// If the plugin finished and asked for the wall, the wall is opened here, by
/// the launcher, in this process's own environment: the plugin's token and run
/// context never reach it.
pub(crate) fn dispatch_route(
    host: &dyn crate::launcher::Host,
    paths: &Paths,
    route: &external::Route,
    argv: &[String],
    cwd: &std::path::Path,
    home: Option<&std::path::Path>,
) -> Result<Option<i32>, String> {
    if route.fallback && route.needs_daemon {
        let word = argv.first().map(String::as_str).unwrap_or_default();
        match garage_plugin_sdk::folder::resolve_folder(word, cwd, home) {
            Ok(_) => {}
            // not a folder: the usual unrecognized-subcommand error
            Err(garage_plugin_sdk::folder::FolderError::NotADirectory) => return Ok(None),
            // a word written like a path (`~bob`), but unusable: say why
            Err(e) if word.starts_with('~') || word.contains('/') => {
                return Err(format!("{word}: {e}"));
            }
            Err(_) => return Ok(None),
        }
    }
    if route.needs_daemon {
        crate::launcher::ensure_daemon(host).map_err(|f| f.0)?;
    }
    let ran = external::run(paths, host.port(), route, argv);
    if ran.open_wall {
        return crate::launcher::tui_main(host).map(Some).map_err(|f| f.0);
    }
    Ok(Some(ran.code))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plugin::admin::DirectAdmin;
    use crate::plugin::testkit::link_plugin;
    use clap::Parser;

    #[derive(Parser)]
    struct W {
        #[command(subcommand)]
        c: PluginCmd,
    }

    fn parse(args: &[&str]) -> PluginCmd {
        W::try_parse_from(std::iter::once("plugin").chain(args.iter().copied()))
            .unwrap()
            .c
    }

    #[test]
    fn every_subcommand_and_flag_parses() {
        assert_eq!(parse(&["list", "--json"]), PluginCmd::List { json: true });
        assert_eq!(
            parse(&[
                "install",
                "o/r/sub",
                "--ref",
                "v1",
                "--yes",
                "--allow-dangerous"
            ]),
            PluginCmd::Install {
                spec: "o/r/sub".into(),
                git_ref: Some("v1".into()),
                yes: true,
                allow_dangerous: true
            }
        );
        assert_eq!(
            parse(&["link", "./x"]),
            PluginCmd::Link {
                path: "./x".into(),
                yes: false,
                allow_dangerous: false
            }
        );
        assert_eq!(
            parse(&["update", "--all"]),
            PluginCmd::Update {
                id: None,
                all: true,
                yes: false,
                allow_dangerous: false
            }
        );
        assert_eq!(
            parse(&["remove", "p-one", "--purge"]),
            PluginCmd::Remove {
                id: "p-one".into(),
                purge: true
            }
        );
        assert_eq!(
            parse(&["bind", "p:a", "g"]),
            PluginCmd::Bind {
                action: "p:a".into(),
                key: Some("g".into()),
                clear: false
            }
        );
        assert_eq!(
            parse(&["logs", "p", "-f", "-n", "5"]),
            PluginCmd::Logs {
                id: "p".into(),
                follow: true,
                lines: 5
            }
        );
        assert_eq!(parse(&["schema"]), PluginCmd::Schema);
        assert_eq!(
            parse(&["new", "my-plug", "--lang", "shell"]),
            PluginCmd::New {
                name: "my-plug".into(),
                lang: Lang::Shell,
                sdk_path: None
            }
        );
        assert_eq!(
            parse(&["new", "x1"]),
            PluginCmd::New {
                name: "x1".into(),
                lang: Lang::Rust,
                sdk_path: None
            }
        );
        assert_eq!(
            parse(&["dev"]),
            PluginCmd::Dev {
                path: ".".into(),
                yes: false,
                allow_dangerous: false
            }
        );
        assert_eq!(
            parse(&["search", "usage", "bar", "--json", "--sort", "updated"]),
            PluginCmd::Search {
                query: vec!["usage".into(), "bar".into()],
                json: true,
                sort: Sort::Updated,
                limit: 20
            }
        );
        assert!(W::try_parse_from(["plugin", "new", "x", "--lang", "cobol"]).is_err());
        assert!(W::try_parse_from(["plugin", "install"]).is_err());
    }

    #[test]
    fn list_shows_state_source_and_flags_dangerous_permissions_with_problems_below() {
        let d = tempfile::tempdir().unwrap();
        let paths = Paths::new(d.path());
        link_plugin(&paths, d.path(), "quiet", "", true);
        link_plugin(
            &paths,
            d.path(),
            "typist",
            "permissions = [\"sessions.input\", \"network\"]",
            true,
        );
        link_plugin(&paths, d.path(), "fresh", "", false);
        let r = rows(&load_all(&paths).unwrap());
        let text = list_text(&r);
        let lines: Vec<&str> = text.lines().collect();
        assert!(
            lines[0].starts_with("ID") && lines[0].contains("PERMISSIONS"),
            "{text}"
        );
        assert!(
            lines[1].starts_with("quiet")
                && lines[1].contains(" active ")
                && lines[1].trim_end().ends_with('-'),
            "{text}"
        );
        assert!(lines[2].contains("sessions.input! network"), "{text}");
        assert!(text.contains("! = dangerous permission"));
        assert!(
            lines
                .iter()
                .any(|l| l.starts_with("fresh") && l.contains("not approved"))
        );
        let j = list_json(&r);
        assert_eq!(j[1]["dangerous"], json!(["sessions.input"]));
        assert_eq!(j[0]["enabled"], true);
        assert_eq!(
            list_text(&[]),
            "No plugins installed. Try `claude-garage plugin install owner/repo` or `plugin link <folder>`."
        );
    }

    #[test]
    fn bind_needs_a_full_known_action_and_a_key_and_clear_removes_it() {
        let d = tempfile::tempdir().unwrap();
        let paths = Paths::new(d.path());
        link_plugin(
            &paths,
            d.path(),
            "keyed",
            "[[actions]]\nid = \"go\"\ntitle = \"Go\"\ncommand = [\"x\"]\n",
            true,
        );
        let admin = DirectAdmin {
            paths: paths.clone(),
        };
        assert!(
            bind(&paths, &admin, "go", Some("g"), false)
                .unwrap_err()
                .contains("full action id")
        );
        assert!(
            bind(&paths, &admin, "nope:go", Some("g"), false)
                .unwrap_err()
                .contains("no plugin")
        );
        let e = bind(&paths, &admin, "keyed:stop", Some("g"), false).unwrap_err();
        assert!(e.contains("keyed:go"), "lists what exists: {e}");
        assert!(
            bind(&paths, &admin, "keyed:go", None, false)
                .unwrap_err()
                .contains("give a key")
        );
        assert!(bind(&paths, &admin, "keyed:go", Some("a b"), false).is_err());
        bind(&paths, &admin, "keyed:go", Some("g"), false).unwrap();
        assert_eq!(
            PluginsFile::read(&paths).unwrap().plugins[0].bindings["keyed:go"],
            "g"
        );
        bind(&paths, &admin, "keyed:go", None, true).unwrap();
        assert!(
            PluginsFile::read(&paths).unwrap().plugins[0]
                .bindings
                .is_empty()
        );
    }

    #[test]
    fn the_log_tail_keeps_the_last_lines_and_the_schema_is_the_committed_one() {
        assert_eq!(tail_lines("a\nb\nc\nd", 2), "c\nd");
        assert_eq!(tail_lines("a", 5), "a");
        assert_eq!(tail_lines("", 5), "");
        let committed = std::fs::read_to_string(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../schemas/garage-plugin.schema.json"
        ))
        .unwrap();
        assert_eq!(
            committed.trim(),
            garage_plugin::schema::schema_json().trim(),
            "`plugin schema` prints the committed file"
        );
    }
}
