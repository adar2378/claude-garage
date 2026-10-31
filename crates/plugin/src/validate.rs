//! Manifest parsing and validation. Everything here runs before a plugin
//! command does; every error names its field path (`actions[1].command`).

use std::collections::{BTreeSet, HashMap};
use std::fmt;

use crate::core_actions;
use crate::features::{self, API_VERSIONS};
use crate::manifest::*;

/// One validation failure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Problem {
    pub path: String,
    pub message: String,
}

impl Problem {
    fn new(path: impl Into<String>, message: impl Into<String>) -> Self {
        Self { path: path.into(), message: message.into() }
    }
}

impl fmt::Display for Problem {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.path, self.message)
    }
}

/// Ids the host owns; a plugin may not take them.
pub const RESERVED_IDS: &[&str] = &[
    "session", "workspace", "focus", "jump", "overlay", "view", "hooks", "app", "pet", "daemon", "chord", "plugin", "pane",
    "core",
];

/// `claude-garage` subcommands a `[[cli]]` entry may not shadow. `install`
/// is NOT here: core has no such subcommand, so a plugin may own it.
pub const RESERVED_CLI: &[&str] = &["start", "stop", "status", "restart", "plugin", "tui", "daemon", "help", "version"];

/// `^[a-z][a-z0-9-]{1,30}$`
pub fn valid_plugin_id(id: &str) -> bool {
    let b = id.as_bytes();
    (2..=31).contains(&b.len())
        && b[0].is_ascii_lowercase()
        && b.iter().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || *c == b'-')
}

/// Local action / pane / widget ids: no dots or colons, so a bare id is
/// never mistaken for a core (`a.b`) or plugin (`p:a`) id.
pub fn valid_local_id(id: &str) -> bool {
    let b = id.as_bytes();
    (1..=32).contains(&b.len())
        && b[0].is_ascii_lowercase()
        && b.iter().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || *c == b'-' || *c == b'_')
}

fn valid_cli_name(n: &str) -> bool {
    let b = n.as_bytes();
    !b.is_empty() && b.len() <= 32 && b[0].is_ascii_lowercase() && b.iter().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || *c == b'-')
}

/// Parse `garage-plugin.toml` text and validate it. On failure every problem
/// found is returned (syntax and type errors are a single problem, since the
/// parser stops at the first).
pub fn parse(src: &str) -> Result<Manifest, Vec<Problem>> {
    let de = toml::Deserializer::new(src);
    let manifest: Manifest = match serde_path_to_error::deserialize(de) {
        Ok(m) => m,
        Err(e) => return Err(vec![deserialize_problem(src, &e)]),
    };
    let problems = validate(&manifest);
    if problems.is_empty() {
        Ok(manifest)
    } else {
        Err(problems)
    }
}

fn deserialize_problem(src: &str, e: &serde_path_to_error::Error<toml::de::Error>) -> Problem {
    let inner = e.inner();
    let mut path = e.path().to_string();
    let mut message = inner.message().to_owned();
    // `missing field `x`` belongs to the table it was found in: name the field.
    if let Some(field) = message.strip_prefix("missing field `").and_then(|r| r.strip_suffix('`')) {
        path = if path == "." { field.to_owned() } else { format!("{path}.{field}") };
        message = "missing".into();
    }
    if path == "." {
        path = "manifest".into();
    }
    if let Some(span) = inner.span() {
        let line = src[..span.start.min(src.len())].bytes().filter(|b| *b == b'\n').count() + 1;
        message = format!("{message} (line {line})");
    }
    Problem::new(path, message)
}

fn command_problems(path: &str, cmd: &[String], out: &mut Vec<Problem>) {
    if cmd.is_empty() || cmd[0].is_empty() {
        out.push(Problem::new(path, "must be a non-empty argv array with a program first"));
    }
}

fn range_problem(path: &str, v: Option<u32>, lo: u32, hi: u32, out: &mut Vec<Problem>) {
    if let Some(v) = v {
        if !(lo..=hi).contains(&v) {
            out.push(Problem::new(path, format!("{v} is out of range {lo}..={hi}")));
        }
    }
}

/// Structural rules (no host knowledge). See [`check_host`] for the rest.
pub fn validate(m: &Manifest) -> Vec<Problem> {
    let mut out = Vec::new();
    if !valid_plugin_id(&m.id) {
        out.push(Problem::new("id", "must match ^[a-z][a-z0-9-]{1,30}$"));
    } else if RESERVED_IDS.contains(&m.id.as_str()) {
        out.push(Problem::new("id", format!("{:?} is reserved", m.id)));
    }
    if m.name.trim().is_empty() {
        out.push(Problem::new("name", "must not be empty"));
    }
    if semver::Version::parse(&m.version).is_err() {
        out.push(Problem::new("version", format!("{:?} is not a semver version", m.version)));
    }
    if m.platforms.is_empty() {
        out.push(Problem::new("platforms", "must list at least one of macos, linux, windows"));
    }
    let mut seen = BTreeSet::new();
    for (i, p) in m.permissions.iter().enumerate() {
        if !seen.insert(p.as_str()) {
            out.push(Problem::new(format!("permissions[{i}]"), format!("{:?} is listed twice", p.as_str())));
        }
    }

    let mut action_ids = BTreeSet::new();
    for (i, a) in m.actions.iter().enumerate() {
        let at = format!("actions[{i}]");
        if !valid_local_id(&a.id) {
            out.push(Problem::new(format!("{at}.id"), "must match ^[a-z][a-z0-9_-]{0,31}$ (no dots or colons)"));
        } else if !action_ids.insert(a.id.as_str()) {
            out.push(Problem::new(format!("{at}.id"), format!("duplicate action id {:?}", a.id)));
        }
        if a.title.trim().is_empty() {
            out.push(Problem::new(format!("{at}.title"), "must not be empty"));
        }
        if a.contexts.is_empty() {
            out.push(Problem::new(format!("{at}.contexts"), "must not be empty"));
        }
        match (&a.command, &a.pane) {
            (None, None) => out.push(Problem::new(
                format!("{at}.command"),
                "missing (an action needs exactly one of command or pane)",
            )),
            (Some(_), Some(_)) => {
                out.push(Problem::new(at.clone(), "has both command and pane (exactly one is allowed)"))
            }
            (Some(c), None) => command_problems(&format!("{at}.command"), c, &mut out),
            (None, Some(p)) => {
                if m.pane(p).is_none() {
                    out.push(Problem::new(format!("{at}.pane"), format!("no pane {p:?} in [[panes]]")));
                }
            }
        }
        if a.default_key.as_deref().is_some_and(|k| k.trim().is_empty()) {
            out.push(Problem::new(format!("{at}.default_key"), "must not be empty"));
        }
        range_problem(&format!("{at}.timeout_secs"), a.timeout_secs, 1, 300, &mut out);
    }

    for (i, e) in m.events.iter().enumerate() {
        let at = format!("events[{i}]");
        match (e.on, e.every_secs) {
            (EventName::Timer, None) => out.push(Problem::new(format!("{at}.every_secs"), "missing (required with on = \"timer\")")),
            (EventName::Timer, Some(n)) if n < 5 => {
                out.push(Problem::new(format!("{at}.every_secs"), format!("{n} is below the minimum of 5")))
            }
            (EventName::Timer, Some(_)) => {}
            (_, Some(_)) => out.push(Problem::new(format!("{at}.every_secs"), "is only valid with on = \"timer\"")),
            (_, None) => {}
        }
        command_problems(&format!("{at}.command"), &e.command, &mut out);
        range_problem(&format!("{at}.timeout_secs"), e.timeout_secs, 1, 60, &mut out);
    }

    let mut pane_ids = BTreeSet::new();
    for (i, p) in m.panes.iter().enumerate() {
        let at = format!("panes[{i}]");
        if !valid_local_id(&p.id) {
            out.push(Problem::new(format!("{at}.id"), "must match ^[a-z][a-z0-9_-]{0,31}$ (no dots or colons)"));
        } else if !pane_ids.insert(p.id.as_str()) {
            out.push(Problem::new(format!("{at}.id"), format!("duplicate pane id {:?}", p.id)));
        }
        if p.title.trim().is_empty() {
            out.push(Problem::new(format!("{at}.title"), "must not be empty"));
        }
        command_problems(&format!("{at}.command"), &p.command, &mut out);
        if p.size.as_ref().is_some_and(|s| s.cols == 0 || s.rows == 0 || (s.pct && (s.cols > 100 || s.rows > 100))) {
            out.push(Problem::new(format!("{at}.size"), "cols and rows must be positive (and at most 100 with pct)"));
        }
    }

    let mut widget_ids = BTreeSet::new();
    for (i, w) in m.widgets.iter().enumerate() {
        if !valid_local_id(&w.id) {
            out.push(Problem::new(format!("widgets[{i}].id"), "must match ^[a-z][a-z0-9_-]{0,31}$ (no dots or colons)"));
        } else if !widget_ids.insert(w.id.as_str()) {
            out.push(Problem::new(format!("widgets[{i}].id"), format!("duplicate widget id {:?}", w.id)));
        }
    }

    for (i, b) in m.buttons.iter().enumerate() {
        let at = format!("buttons[{i}]");
        if matches!(b.slot, Slot::TileGutter | Slot::RailExtraRows) {
            out.push(Problem::new(
                format!("{at}.slot"),
                format!("{} holds pushed items, not buttons", b.slot.as_str()),
            ));
        }
        if b.label.is_empty() {
            out.push(Problem::new(format!("{at}.label"), "must not be empty"));
        }
        if let Some(p) = on_click_problem(m, &b.on_click) {
            out.push(Problem::new(format!("{at}.on_click"), p));
        }
    }

    let mut cli_names = BTreeSet::new();
    for (i, c) in m.cli.iter().enumerate() {
        let at = format!("cli[{i}]");
        if !valid_cli_name(&c.name) {
            out.push(Problem::new(format!("{at}.name"), "must match ^[a-z][a-z0-9-]{0,31}$"));
        } else if RESERVED_CLI.contains(&c.name.as_str()) {
            out.push(Problem::new(format!("{at}.name"), format!("{:?} is a core claude-garage command", c.name)));
        } else if !cli_names.insert(c.name.as_str()) {
            out.push(Problem::new(format!("{at}.name"), format!("duplicate cli name {:?}", c.name)));
        }
        command_problems(&format!("{at}.command"), &c.command, &mut out);
    }

    for (i, b) in m.build.iter().enumerate() {
        command_problems(&format!("build[{i}].command"), &b.command, &mut out);
        range_problem(&format!("build[{i}].timeout_secs"), b.timeout_secs, 1, 3600, &mut out);
    }

    if let Some(r) = &m.release {
        if r.checksums.trim().is_empty() {
            out.push(Problem::new("release.checksums", "must name the checksums asset"));
        }
        if r.assets.is_empty() {
            out.push(Problem::new("release.assets", "must list at least one os-arch asset"));
        }
        for key in r.assets.keys() {
            let ok = key.split_once('-').is_some_and(|(os, arch)| {
                ["macos", "linux", "windows"].contains(&os) && ["aarch64", "x86_64"].contains(&arch)
            });
            if !ok {
                out.push(Problem::new(format!("release.assets.{key}"), "key must be <macos|linux|windows>-<aarch64|x86_64>"));
            }
        }
    }

    // Declared permissions have to be honest about what the manifest uses.
    let needs = |perm: Permission, why: &str, out: &mut Vec<Problem>| {
        if !m.has_permission(perm) {
            out.push(Problem::new("permissions", format!("{why} needs the {:?} permission", perm.as_str())));
        }
    };
    if !m.widgets.is_empty() {
        needs(Permission::UiWidgets, "widgets", &mut out);
    }
    if !m.panes.is_empty() {
        needs(Permission::UiPanes, "panes", &mut out);
    }
    if m.buttons.iter().any(|b| core_target(&b.on_click)) {
        needs(Permission::CoreActions, "a button that triggers a core action", &mut out);
    }
    out
}

fn core_target(on_click: &str) -> bool {
    garage_protocol::widgets::is_core_action_id(on_click)
}

/// Why `on_click` does not resolve, if it does not.
fn on_click_problem(m: &Manifest, on_click: &str) -> Option<String> {
    if on_click.is_empty() {
        return Some("must not be empty".into());
    }
    if let Some((plugin, action)) = on_click.split_once(':') {
        if !valid_plugin_id(plugin) || !valid_local_id(action) {
            return Some(format!("{on_click:?} is not a valid plugin:action id"));
        }
        // Another plugin's action can only be resolved against what is
        // installed; this plugin's own must exist.
        if plugin == m.id && m.action(action).is_none() {
            return Some(format!("this plugin has no action {action:?}"));
        }
        return None;
    }
    if on_click.contains('.') {
        return match core_actions::lookup(on_click) {
            None => Some(format!("{on_click:?} is not a core action")),
            Some(a) if !a.plugin_invocable => Some(format!("core action {on_click:?} cannot be triggered by plugins")),
            Some(_) => None,
        };
    }
    if m.action(on_click).is_none() {
        return Some(format!("no action {on_click:?} in [[actions]] (use plugin:action or a core id for others)"));
    }
    None
}

/// What the running garage decides: OS, API version and `requires`. Used at
/// install and at every load.
pub fn check_host(m: &Manifest) -> Vec<Problem> {
    let mut out = Vec::new();
    if !API_VERSIONS.contains(&m.api_version) {
        let newest = API_VERSIONS.iter().max().copied().unwrap_or(1);
        let message = if m.api_version > newest {
            format!(
                "needs a newer garage: plugin API v{} is not supported here (this garage supports {:?}); upgrade claude-garage",
                m.api_version, API_VERSIONS
            )
        } else {
            format!("uses removed plugin API v{} (this garage supports {:?})", m.api_version, API_VERSIONS)
        };
        out.push(Problem::new("api_version", message));
    }
    let os = crate::argv::current_os();
    if !m.platforms.iter().any(|p| p.as_str() == os) {
        out.push(Problem::new("platforms", format!("does not include this OS ({os})")));
    }
    for (i, r) in m.requires.iter().enumerate() {
        if !features::has_feature(r) {
            out.push(Problem::new(format!("requires[{i}]"), format!("needs garage feature {r:?}, which this garage does not have")));
        }
    }
    out
}

/// A plugin refused because of another plugin.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CliConflict {
    pub plugin: String,
    pub name: String,
    pub message: String,
}

/// Cross-plugin `[[cli]]` rules over `(manifest, first_party)` in install
/// order: a name is owned by the first plugin that claims it, and at most one
/// plugin holds `fallback = true`, with a first-party plugin beating a
/// third-party one regardless of order. Losers are returned with a message
/// naming the owner.
pub fn check_cli_conflicts(plugins: &[(&Manifest, bool)]) -> Vec<CliConflict> {
    let mut out = Vec::new();
    let mut owners: HashMap<&str, &str> = HashMap::new();
    for (m, _) in plugins {
        for c in &m.cli {
            match owners.get(c.name.as_str()) {
                Some(owner) if *owner != m.id => out.push(CliConflict {
                    plugin: m.id.clone(),
                    name: c.name.clone(),
                    message: format!("cli name {:?} is already used by plugin {:?}", c.name, owner),
                }),
                _ => {
                    owners.insert(c.name.as_str(), m.id.as_str());
                }
            }
        }
    }
    let claimants: Vec<&(&Manifest, bool)> = plugins.iter().filter(|(m, _)| m.cli.iter().any(|c| c.fallback)).collect();
    let owner = claimants.iter().find(|(_, first)| *first).or(claimants.first());
    if let Some((owner, _)) = owner {
        for (m, _) in &claimants {
            if m.id != owner.id {
                out.push(CliConflict {
                    plugin: m.id.clone(),
                    name: String::new(),
                    message: format!("fallback = true is already held by plugin {:?}", owner.id),
                });
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A manifest that passes everything; each rule test breaks one thing.
    const GOOD: &str = r#"
id = "usage-bar"
name = "Usage bar"
version = "0.1.0"
api_version = 1
platforms = ["macos", "linux", "windows"]
permissions = ["ui.widgets", "ui.panes", "core.actions", "sessions.read"]

[[actions]]
id = "refresh"
title = "Refresh"
command = ["{plugin_dir}/bin/usage", "refresh"]

[[actions]]
id = "details"
title = "Details"
pane = "details"

[[events]]
on = "session.closed"
command = ["./on-close.sh"]

[[events]]
on = "timer"
every_secs = 60
command = ["./tick.sh"]

[[panes]]
id = "details"
title = "Usage details"
placement = "overlay"
command = ["./ui.sh"]

[[widgets]]
id = "bar"
slot = "strip.right"

[[buttons]]
slot = "tile.title.right"
label = " x "
on_click = "session.close"
confirm = true

[[buttons]]
slot = "rail.header"
label = "r"
on_click = "refresh"

[[cli]]
name = "usage"
command = ["./cli.sh"]

[release]
checksums = "SHA256SUMS"
[release.assets]
"macos-aarch64" = "usage-darwin-arm64"
"#;

    fn parse_ok(src: &str) -> Manifest {
        parse(src).unwrap_or_else(|p| panic!("{}", p.iter().map(|p| p.to_string()).collect::<Vec<_>>().join("; ")))
    }

    fn fail(src: &str) -> Vec<String> {
        parse(src).expect_err("manifest should be refused").iter().map(Problem::to_string).collect()
    }

    /// `GOOD` with one exact substring replaced.
    fn tweak(from: &str, to: &str) -> String {
        assert!(GOOD.contains(from), "fixture lacks {from:?}");
        GOOD.replacen(from, to, 1)
    }

    #[test]
    fn the_good_manifest_parses_with_every_section() {
        let m = parse_ok(GOOD);
        assert_eq!((m.actions.len(), m.events.len(), m.panes.len(), m.buttons.len()), (2, 2, 1, 2));
        assert!(m.panes[0].singleton, "singleton defaults to true");
        assert_eq!(m.actions[0].contexts, [ActionContext::Global]);
        assert!(check_host(&m).is_empty());
    }

    #[test]
    fn an_action_without_a_command_names_the_missing_field_and_nothing_runs() {
        let errs = fail(&tweak("command = [\"{plugin_dir}/bin/usage\", \"refresh\"]\n", ""));
        assert_eq!(errs.len(), 1, "{errs:?}");
        assert!(errs[0].starts_with("actions[0].command: missing"), "{errs:?}");
    }

    #[test]
    fn serde_level_errors_carry_the_field_path() {
        // a required key missing deep in an array of tables
        let errs = fail(&tweak("on = \"session.closed\"\ncommand = [\"./on-close.sh\"]\n", "on = \"session.closed\"\n"));
        assert!(errs[0].starts_with("events[0].command: missing"), "{errs:?}");
        // a typo'd key is an error, not ignored
        let errs = fail(&tweak("title = \"Refresh\"", "title = \"Refresh\"\ntimeout = 3"));
        assert!(errs[0].contains("unknown field `timeout`") && errs[0].starts_with("actions[0]"), "{errs:?}");
        // an unknown enum value
        let errs = fail(&tweak("slot = \"strip.right\"", "slot = \"strip.left\""));
        assert!(errs[0].starts_with("widgets[0].slot") && errs[0].contains("strip.left"), "{errs:?}");
        // plain TOML syntax
        let errs = fail("id = ");
        assert!(errs[0].starts_with("manifest:") && errs[0].contains("line 1"), "{errs:?}");
    }

    /// One failing manifest per rule: (what to break, with what, expected
    /// path prefix, expected message fragment).
    #[test]
    fn each_validation_rule_rejects_its_own_manifest() {
        let cases: &[(&str, &str, &str, &str)] = &[
            ("id = \"usage-bar\"", "id = \"Usage_Bar\"", "id", "^[a-z]"),
            ("id = \"usage-bar\"", "id = \"a\"", "id", "^[a-z]"),
            ("id = \"usage-bar\"", "id = \"daemon\"", "id", "reserved"),
            ("version = \"0.1.0\"", "version = \"one\"", "version", "semver"),
            ("platforms = [\"macos\", \"linux\", \"windows\"]", "platforms = []", "platforms", "at least one"),
            ("permissions = [\"ui.widgets\", \"ui.panes\", \"core.actions\", \"sessions.read\"]",
             "permissions = [\"ui.widgets\", \"ui.panes\", \"core.actions\", \"core.actions\"]", "permissions[3]", "twice"),
            ("permissions = [\"ui.widgets\", \"ui.panes\", \"core.actions\", \"sessions.read\"]",
             "permissions = [\"ui.widgets\", \"ui.panes\", \"core.actions\", \"root.everything\"]", "permissions[3]", "root.everything"),
            ("id = \"refresh\"", "id = \"details\"", "actions[1].id", "duplicate"),
            ("id = \"refresh\"", "id = \"re.fresh\"", "actions[0].id", "no dots"),
            ("pane = \"details\"", "pane = \"nowhere\"", "actions[1].pane", "no pane"),
            ("pane = \"details\"", "pane = \"details\"\ncommand = [\"x\"]", "actions[1]", "both command and pane"),
            ("command = [\"{plugin_dir}/bin/usage\", \"refresh\"]", "command = []", "actions[0].command", "non-empty"),
            ("title = \"Refresh\"", "title = \"Refresh\"\ntimeout_secs = 301", "actions[0].timeout_secs", "1..=300"),
            ("every_secs = 60", "every_secs = 4", "events[1].every_secs", "minimum of 5"),
            ("every_secs = 60\n", "", "events[1].every_secs", "required"),
            ("on = \"session.closed\"", "on = \"session.closed\"\nevery_secs = 9", "events[0].every_secs", "only valid with"),
            ("on = \"session.closed\"", "on = \"session.exploded\"", "events[0]", "session.exploded"),
            ("command = [\"./on-close.sh\"]", "command = [\"./on-close.sh\"]\ntimeout_secs = 61", "events[0].timeout_secs", "1..=60"),
            ("placement = \"overlay\"", "placement = \"floating\"", "panes[0]", "floating"),
            ("slot = \"strip.right\"", "slot = \"strip.right\"\n[[widgets]]\nid = \"bar\"\nslot = \"rail.header\"", "widgets[1].id", "duplicate"),
            ("slot = \"tile.title.right\"", "slot = \"tile.gutter\"", "buttons[0].slot", "not buttons"),
            ("slot = \"tile.title.right\"", "slot = \"rail.extra_rows\"", "buttons[0].slot", "not buttons"),
            ("on_click = \"session.close\"", "on_click = \"app.quit\"", "buttons[0].on_click", "cannot be triggered by plugins"),
            ("on_click = \"session.close\"", "on_click = \"session.teleport\"", "buttons[0].on_click", "not a core action"),
            ("on_click = \"refresh\"", "on_click = \"vanish\"", "buttons[1].on_click", "no action"),
            ("on_click = \"refresh\"", "on_click = \"usage-bar:vanish\"", "buttons[1].on_click", "no action \"vanish\""),
            ("on_click = \"refresh\"", "on_click = \"Bad:id\"", "buttons[1].on_click", "valid plugin:action"),
            ("name = \"usage\"", "name = \"restart\"", "cli[0].name", "core claude-garage command"),
            ("name = \"usage\"", "name = \"plugin\"", "cli[0].name", "core claude-garage command"),
            ("checksums = \"SHA256SUMS\"\n", "", "release.checksums", "missing"),
            ("\"macos-aarch64\"", "\"macos-sparc\"", "release.assets.macos-sparc", "<macos|linux|windows>"),
            ("[[cli]]\nname = \"usage\"\ncommand = [\"./cli.sh\"]", "[[cli]]\nname = \"usage\"\ncommand = [\"./cli.sh\"]\n[[cli]]\nname = \"usage\"\ncommand = [\"x\"]", "cli[1].name", "duplicate"),
        ];
        for (from, to, path, fragment) in cases {
            let errs = fail(&tweak(from, to));
            assert!(
                errs.iter().any(|e| e.starts_with(path) && e.contains(fragment)),
                "breaking {from:?} -> {to:?}: wanted {path}: ..{fragment}.., got {errs:?}"
            );
        }
    }

    #[test]
    fn manifests_must_declare_the_permissions_their_sections_use() {
        let no_widgets = tweak("\"ui.widgets\", ", "");
        assert!(fail(&no_widgets).iter().any(|e| e.contains("widgets needs the \"ui.widgets\"")));
        let no_panes = tweak("\"ui.panes\", ", "");
        assert!(fail(&no_panes).iter().any(|e| e.contains("panes needs the \"ui.panes\"")));
        let no_core = tweak("\"core.actions\", ", "");
        assert!(fail(&no_core).iter().any(|e| e.contains("core action needs the \"core.actions\"")));
    }

    #[test]
    fn install_is_not_a_reserved_cli_name() {
        let m = parse_ok(&tweak("name = \"usage\"", "name = \"install\""));
        assert_eq!(m.cli[0].name, "install");
    }

    #[test]
    fn the_close_button_example_with_no_commands_is_valid() {
        let m = parse_ok(
            r##"#:schema https://example.test/garage-plugin.schema.json
id = "close-button"
name = "Close button"
version = "0.1.0"
api_version = 1
platforms = ["macos", "linux", "windows"]
permissions = ["core.actions"]

[[buttons]]
slot = "tile.title.right"
label = " x "
tooltip = "Close this session"
on_click = "session.close"
confirm = true
priority = 5
"##,
        );
        assert!(m.actions.is_empty() && m.buttons[0].priority == 5);
    }

    #[test]
    fn host_checks_cover_api_version_platform_and_requires() {
        let mut m = parse_ok(GOOD);
        m.api_version = 2;
        let p = check_host(&m);
        assert!(p[0].path == "api_version" && p[0].message.contains("newer garage"), "{p:?}");
        m.api_version = 0;
        assert!(check_host(&m)[0].message.contains("removed plugin API v0"));
        m.api_version = 1;
        m.requires = vec!["widgets.animated".into(), "events:session.turn".into(), "warp.drive".into()];
        let p = check_host(&m);
        assert_eq!(p.len(), 1);
        assert!(p[0].path == "requires[2]" && p[0].message.contains("needs garage feature \"warp.drive\""), "{p:?}");
        m.requires.clear();
        m.platforms = vec![if crate::argv::current_os() == "macos" { Platform::Windows } else { Platform::Macos }];
        assert!(check_host(&m)[0].path == "platforms");
    }

    fn with_cli(id: &str, names: &[&str], fallback_on: Option<&str>) -> Manifest {
        let mut m = parse_ok(GOOD);
        m.id = id.into();
        m.cli = names
            .iter()
            .map(|n| Cli { name: (*n).into(), description: None, command: vec!["x".into()], fallback: fallback_on == Some(n) })
            .collect();
        m
    }

    #[test]
    fn cli_names_belong_to_the_first_claimant() {
        let (a, b) = (with_cli("one", &["usage"], None), with_cli("two", &["usage", "other"], None));
        let c = check_cli_conflicts(&[(&a, false), (&b, false)]);
        assert_eq!(c.len(), 1);
        assert_eq!((c[0].plugin.as_str(), c[0].name.as_str()), ("two", "usage"));
        assert!(c[0].message.contains("\"one\""));
        assert!(check_cli_conflicts(&[(&a, false)]).is_empty());
    }

    #[test]
    fn only_one_fallback_and_first_party_wins_over_earlier_third_party() {
        let third = with_cli("third", &["go"], Some("go"));
        let first = with_cli("extras", &["ex"], Some("ex"));
        let c = check_cli_conflicts(&[(&third, false), (&first, true)]);
        assert_eq!(c.len(), 1);
        assert_eq!(c[0].plugin, "third");
        assert!(c[0].message.contains("\"extras\""), "{}", c[0].message);
        // two third parties: the earlier install keeps it
        let other = with_cli("other", &["o"], Some("o"));
        let c = check_cli_conflicts(&[(&third, false), (&other, false)]);
        assert_eq!((c.len(), c[0].plugin.as_str()), (1, "other"));
        // a lone fallback is fine
        assert!(check_cli_conflicts(&[(&third, false)]).is_empty());
    }
}
