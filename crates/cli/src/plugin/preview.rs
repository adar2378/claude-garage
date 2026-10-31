//! The consent screen for `plugin install`, `link` and `update` (design
//! section 11, "honest consent"), and the update policy: which changes need a
//! new prompt. Pure functions over a manifest; nothing here touches the disk.

use std::collections::BTreeSet;

use garage_plugin::core_actions;
use garage_plugin::manifest::{Manifest, Permission};
use garage_plugin::validate::{escape_hostile as e, is_hostile_char};

use super::fetch::{ReleasePlan, all_release_urls, builds_for_os};

pub const UPDATE_POLICY: &str = "Later updates that change only plugin code (no new commands, permissions or URLs) apply without asking again.";
pub const NOT_SANDBOXED: &str =
    "Plugins are not sandboxed. They run as you and can read, write and delete anything you can.";

/// Everything a user consents to. Two manifests with equal surfaces need no
/// new prompt on update; any difference does.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Surface {
    pub permissions: BTreeSet<String>,
    /// `kind name: argv` lines for every command garage may run.
    pub commands: BTreeSet<String>,
    pub urls: BTreeSet<String>,
    /// Suggested keys: shown in the diff, not a consent matter.
    pub keys: BTreeSet<String>,
}

/// The argv as one shell-looking line. Control and invisible characters are
/// shown as `\u{XXXX}`, never emitted.
pub fn quote(argv: &[String]) -> String {
    e(&quote_raw(argv))
}

fn quote_raw(argv: &[String]) -> String {
    argv.iter()
        .map(|a| {
            if !a.is_empty()
                && a.bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"-_./:=@%+,{}".contains(&b))
            {
                a.clone()
            } else {
                format!("'{}'", a.replace('\'', "'\\''"))
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

impl Surface {
    /// Everything that can run, be triggered, or be fetched. Only version,
    /// description, titles, labels and suggested keys are left out.
    pub fn of(m: &Manifest, install_repo: Option<&str>) -> Self {
        let mut s = Surface {
            permissions: m.permission_names().into_iter().collect(),
            ..Default::default()
        };
        for b in &m.build {
            let platforms: Vec<&str> = b.platforms.iter().map(|p| p.as_str()).collect();
            s.commands.insert(format!(
                "build [{}]: {}",
                platforms.join(","),
                quote(&b.command)
            ));
        }
        for a in &m.actions {
            let what = match (&a.command, &a.pane) {
                (Some(c), _) => quote(c),
                (None, Some(p)) => format!("opens pane {}", e(p)),
                (None, None) => String::new(),
            };
            let confirm = if a.confirm { " (asks first)" } else { "" };
            s.commands
                .insert(format!("action {}{confirm}: {what}", e(&a.id)));
            if let Some(k) = &a.default_key {
                s.keys
                    .insert(format!("{} -> {}:{}", e(k), e(&m.id), e(&a.id)));
            }
        }
        for h in &m.events {
            let every = h
                .every_secs
                .map(|n| format!(" every {n}s"))
                .unwrap_or_default();
            s.commands.insert(format!(
                "event {}{every}: {}",
                h.on.as_str(),
                quote(&h.command)
            ));
        }
        for p in &m.panes {
            s.commands.insert(format!(
                "pane {} {:?}: {}",
                e(&p.id),
                p.placement,
                quote(&p.command)
            ));
        }
        for c in &m.cli {
            let fb = if c.fallback { " (fallback)" } else { "" };
            s.commands
                .insert(format!("cli {}{fb}: {}", e(&c.name), quote(&c.command)));
        }
        for b in &m.buttons {
            let args = b
                .args
                .as_ref()
                .map(|a| format!(" args {}", e(&a.to_string())))
                .unwrap_or_default();
            let confirm = if b.confirm { " (asks first)" } else { "" };
            s.commands.insert(format!(
                "button {}{confirm} triggers {}{args}",
                b.slot.as_str(),
                e(&b.on_click)
            ));
        }
        for w in &m.widgets {
            s.commands
                .insert(format!("widget {} in {}", e(&w.id), w.slot.as_str()));
        }
        s.urls = all_release_urls(m, install_repo)
            .into_iter()
            .map(|u| e(&u))
            .collect();
        s
    }

    /// Does going from `self` (installed) to `new` need the user's yes?
    pub fn needs_consent(&self, new: &Surface) -> bool {
        self.permissions != new.permissions
            || self.commands != new.commands
            || self.urls != new.urls
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Diff {
    pub lines: Vec<String>,
}

impl Diff {
    pub fn between(old: &Surface, new: &Surface) -> Self {
        let mut lines = Vec::new();
        let mut set = |label: &str, a: &BTreeSet<String>, b: &BTreeSet<String>| {
            for x in b.difference(a) {
                lines.push(format!("+ {label} {x}"));
            }
            for x in a.difference(b) {
                lines.push(format!("- {label} {x}"));
            }
        };
        set("permission", &old.permissions, &new.permissions);
        set("command", &old.commands, &new.commands);
        set("url", &old.urls, &new.urls);
        set("key", &old.keys, &new.keys);
        Self { lines }
    }
}

fn permission_blurb(p: &str) -> &'static str {
    match p {
        "sessions.read" => "read session ids, status and directories",
        "sessions.input" => "type text into any session, as if you had typed it",
        "sessions.manage" => "create, close and restart sessions",
        "workspaces.read" => "read workspaces and their folders",
        "workspaces.write" => "add and remove workspaces",
        "ui.widgets" => "draw widgets in the TUI",
        "ui.panes" => "open panes (overlays, popups and tiles)",
        "ui.invoke" => "ask the TUI to run actions",
        "core.actions" => {
            "trigger garage's own actions, such as closing a session (closes and restarts still need a second confirmation)"
        }
        "network" => "plugin is expected to use the network (not enforced)",
        _ => "",
    }
}

pub enum Mode {
    Install,
    Link,
    Update,
}

pub struct PreviewInput<'a> {
    pub manifest: &'a Manifest,
    pub mode: Mode,
    /// `github owner/repo@abc1234 (ref v1)` or `dev /path`.
    pub source: String,
    pub license: Option<String>,
    /// The release download for this platform, if the manifest lists one.
    pub release: Option<&'a ReleasePlan>,
    pub install_repo: Option<&'a str>,
    /// What was installed before (update) or granted (relink).
    pub diff: Option<&'a Diff>,
}

pub fn has_dangerous(m: &Manifest) -> bool {
    m.permissions.iter().any(|p| p.is_dangerous())
}

pub fn render(i: &PreviewInput) -> String {
    let m = i.manifest;
    let mut o = String::new();
    let verb = match i.mode {
        Mode::Install => "Install",
        Mode::Link => "Link",
        Mode::Update => "Update",
    };
    o.push_str(&format!(
        "{verb}: {} {} ({})\n",
        e(&m.name),
        e(&m.version),
        e(&m.id)
    ));
    o.push_str(&format!("Source: {}\n", e(&i.source)));
    if let Some(l) = &i.license {
        o.push_str(&format!("License: {}\n", e(l)));
    }
    if let Some(d) = &m.description {
        o.push_str(&format!("{}\n", e(d)));
    }
    o.push_str(&format!("\n{NOT_SANDBOXED}\n"));

    o.push_str("\nPermissions:\n");
    if m.permissions.is_empty() {
        o.push_str("  none declared\n");
    }
    for p in &m.permissions {
        let danger = if p.is_dangerous() { "DANGEROUS" } else { "" };
        o.push_str(&format!(
            "  {:<16} {:<9}  {}\n",
            p.as_str(),
            danger,
            permission_blurb(p.as_str())
        ));
    }

    // Commands: only what will actually run, in the order it runs.
    o.push_str("\nCommands garage will run:\n");
    let mut any = false;
    let mut line = |o: &mut String, kind: &str, name: &str, argv: &[String]| {
        any = true;
        o.push_str(&format!("  {kind:<8}{:<22}{}\n", e(name), quote(argv)));
    };
    let builds_run = !matches!(i.mode, Mode::Link) && i.release.is_none();
    if builds_run {
        for b in builds_for_os(m) {
            line(&mut o, "build", "(at install)", &b.command);
        }
    }
    for a in &m.actions {
        if let Some(c) = &a.command {
            line(&mut o, "action", &a.id, c);
        }
    }
    for h in &m.events {
        let when = match h.every_secs {
            Some(n) => format!("{} every {n}s", h.on.as_str()),
            None => h.on.as_str().to_owned(),
        };
        line(&mut o, "event", &when, &h.command);
    }
    for p in &m.panes {
        line(&mut o, "pane", &p.id, &p.command);
    }
    for c in &m.cli {
        line(&mut o, "cli", &c.name, &c.command);
    }
    if !any {
        o.push_str("  none (this plugin runs no commands)\n");
    }
    if matches!(i.mode, Mode::Link) && (!m.build.is_empty() || m.release.is_some()) {
        o.push_str("  (link runs no [[build]] and downloads no release; you build it yourself)\n");
    }

    o.push_str("\nDownloads:\n");
    match (i.release, matches!(i.mode, Mode::Link)) {
        (Some(r), false) => {
            o.push_str(&format!(
                "  {}\n  {}\n  (checked against the checksums file; no [[build]] runs)\n",
                e(&r.asset_url),
                e(&r.checksums_url)
            ));
        }
        _ => o.push_str("  none\n"),
    }

    let mut ui = Vec::new();
    for a in &m.actions {
        if let Some(k) = &a.default_key {
            ui.push(format!(
                "  key     {} -> {}:{} ({})",
                e(k),
                e(&m.id),
                e(&a.id),
                e(&a.title)
            ));
        }
    }
    for b in &m.buttons {
        let target = match core_actions::lookup(&b.on_click) {
            Some(c) if c.dangerous => format!(
                "{} (DANGEROUS core action; asks for a second click)",
                e(&b.on_click)
            ),
            _ => e(&b.on_click),
        };
        let args = b
            .args
            .as_ref()
            .map(|a| format!(" with args {}", e(&a.to_string())))
            .unwrap_or_default();
        ui.push(format!(
            "  button  {} {} triggers {target}{args}",
            b.slot.as_str(),
            e(&format!("{:?}", b.label))
        ));
    }
    for w in &m.widgets {
        ui.push(format!("  widget  {} in {}", e(&w.id), w.slot.as_str()));
    }
    for p in &m.panes {
        ui.push(format!(
            "  pane    {} ({})",
            e(&p.id),
            format!("{:?}", p.placement).to_lowercase()
        ));
    }
    for c in &m.cli {
        let fb = if c.fallback {
            " (also receives any unknown first word)"
        } else {
            ""
        };
        ui.push(format!("  cli     claude-garage {}{fb}", e(&c.name)));
    }
    if !ui.is_empty() {
        o.push_str("\nAdds to garage:\n");
        for l in ui {
            o.push_str(&l);
            o.push('\n');
        }
    }

    if let Some(d) = i.diff {
        o.push_str("\nChanges since the approved version:\n");
        if d.lines.is_empty() {
            o.push_str("  none\n");
        }
        for l in &d.lines {
            o.push_str(&format!("  {}\n", e(l)));
        }
    }
    o.push_str(&format!("\n{UPDATE_POLICY}\n"));
    let _ = i.install_repo;
    // Last line of defence: nothing but newlines is a control byte.
    o.chars()
        .map(|c| {
            if c != '\n' && is_hostile_char(c) {
                e(&c.to_string())
            } else {
                c.to_string()
            }
        })
        .collect()
}

/// The one-line question.
pub fn question(m: &Manifest, mode: &Mode) -> String {
    let verb = match mode {
        Mode::Install => "Install",
        Mode::Link => "Link",
        Mode::Update => "Update",
    };
    if has_dangerous(m) {
        let names: Vec<&str> = m
            .permissions
            .iter()
            .filter(|p| p.is_dangerous())
            .map(|p: &Permission| p.as_str())
            .collect();
        format!(
            "{verb} {} including DANGEROUS permissions ({})?",
            e(&m.id),
            names.join(", ")
        )
    } else {
        format!("{verb} {}?", e(&m.id))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn m(extra: &str) -> Manifest {
        garage_plugin::parse(&format!(
            "id = \"close-button\"\nname = \"Close button\"\nversion = \"0.1.0\"\napi_version = 1\nplatforms = [\"macos\",\"linux\",\"windows\"]\ndescription = \"x on every tile\"\n{extra}"
        ))
        .unwrap()
    }

    fn button(on: &str, args: Option<&str>, confirm: bool) -> garage_plugin::manifest::Button {
        garage_plugin::manifest::Button {
            slot: garage_plugin::manifest::Slot::RailHeader,
            label: "L".into(),
            tooltip: None,
            on_click: on.into(),
            args: args.map(|a| serde_json::from_str(a).unwrap()),
            confirm,
            priority: 0,
        }
    }

    const CLOSE: &str = "permissions = [\"core.actions\"]\n[[buttons]]\nslot = \"tile.title.right\"\nlabel = \" x \"\non_click = \"session.close\"\nconfirm = true\n";

    fn input<'a>(manifest: &'a Manifest, mode: Mode, diff: Option<&'a Diff>) -> PreviewInput<'a> {
        PreviewInput {
            manifest,
            mode,
            source: "github owner/repo@9f2c1ab (ref v0.1.0)".into(),
            license: Some("LICENSE file present".into()),
            release: None,
            install_repo: Some("owner/repo"),
            diff,
        }
    }

    #[test]
    fn the_close_button_example_renders_the_documented_preview() {
        let man = m(CLOSE);
        let got = render(&input(&man, Mode::Install, None));
        let want = "\
Install: Close button 0.1.0 (close-button)
Source: github owner/repo@9f2c1ab (ref v0.1.0)
License: LICENSE file present
x on every tile

Plugins are not sandboxed. They run as you and can read, write and delete anything you can.

Permissions:
  core.actions     DANGEROUS  trigger garage's own actions, such as closing a session (closes and restarts still need a second confirmation)

Commands garage will run:
  none (this plugin runs no commands)

Downloads:
  none

Adds to garage:
  button  tile.title.right \" x \" triggers session.close (DANGEROUS core action; asks for a second click)

Later updates that change only plugin code (no new commands, permissions or URLs) apply without asking again.
";
        assert_eq!(got, want);
    }

    #[test]
    fn every_command_and_url_is_listed_dangerous_marks_appear_and_the_policy_line_is_always_last() {
        let man = m(
            "permissions = [\"sessions.input\", \"network\", \"ui.panes\"]\n\
             [[build]]\ncommand = [\"cargo\", \"build\", \"--release\"]\n\
             [[actions]]\nid = \"send\"\ntitle = \"Send\"\ncommand = [\"{bin}\", \"send\", \"two words\"]\ndefault_key = \"g\"\n\
             [[events]]\non = \"session.closed\"\ncommand = [\"./hook.sh\"]\n\
             [[panes]]\nid = \"log\"\ntitle = \"Log\"\nplacement = \"overlay\"\ncommand = [\"tail\", \"-f\", \"x\"]\n\
             [[cli]]\nname = \"send\"\ncommand = [\"{bin}\", \"cli\"]\nfallback = true\n",
        );
        let got = render(&input(&man, Mode::Install, None));
        assert!(
            got.contains("sessions.input   DANGEROUS  type text into any session"),
            "{got}"
        );
        assert!(
            got.lines()
                .any(|l| l.contains("network") && !l.contains("DANGEROUS"))
        );
        for want in [
            "build   (at install)          cargo build --release",
            "action  send                  {bin} send 'two words'",
            "event   session.closed        ./hook.sh",
            "pane    log                   tail -f x",
            "cli     send                  {bin} cli",
            "key     g -> close-button:send (Send)",
            "cli     claude-garage send (also receives any unknown first word)",
        ] {
            assert!(got.contains(want), "missing {want:?} in\n{got}");
        }
        assert!(got.trim_end().ends_with(UPDATE_POLICY));
        assert!(got.contains(NOT_SANDBOXED));
    }

    #[test]
    fn a_listed_release_asset_replaces_build_steps_in_the_preview() {
        let man = m(
            "[[build]]\ncommand = [\"make\"]\n[release]\nchecksums = \"SUMS\"\n[release.assets]\nmacos-aarch64 = \"a\"\n",
        );
        let plan = ReleasePlan {
            platform: "macos-aarch64".into(),
            asset: "a".into(),
            asset_url: "https://github.com/owner/repo/releases/download/v0.1.0/a".into(),
            checksums_url: "https://github.com/owner/repo/releases/download/v0.1.0/SUMS".into(),
        };
        let mut i = input(&man, Mode::Install, None);
        i.release = Some(&plan);
        let got = render(&i);
        assert!(!got.contains("make"), "{got}");
        assert!(got.contains("https://github.com/owner/repo/releases/download/v0.1.0/a\n  https://github.com/owner/repo/releases/download/v0.1.0/SUMS"));
        let got = render(&input(&man, Mode::Install, None));
        assert!(
            got.contains("build   (at install)          make")
                && got.contains("Downloads:\n  none")
        );
    }

    #[test]
    fn link_says_it_builds_and_downloads_nothing() {
        let man = m("[[build]]\ncommand = [\"make\"]\n");
        let mut i = input(&man, Mode::Link, None);
        i.source = "dev /home/me/src/x".into();
        let got = render(&i);
        assert!(got.starts_with("Link: Close button"), "{got}");
        assert!(got.contains("link runs no [[build]]") && !got.contains("(at install)"));
    }

    #[test]
    fn only_commands_permissions_and_urls_force_a_new_prompt_on_update() {
        let base = m(
            "permissions = [\"ui.widgets\"]\n[[actions]]\nid = \"a\"\ntitle = \"A\"\ncommand = [\"x\"]\ndefault_key = \"k\"\n",
        );
        let s0 = Surface::of(&base, Some("o/r"));
        // a new version, description and key are code-only
        let mut bumped = base.clone();
        bumped.version = "0.2.0".into();
        bumped.description = Some("changed words".into());
        bumped.actions[0].default_key = Some("z".into());
        bumped.actions[0].title = "Renamed".into();
        let s1 = Surface::of(&bumped, Some("o/r"));
        assert!(!s0.needs_consent(&s1));
        assert_eq!(
            Diff::between(&s0, &s1).lines,
            ["+ key z -> close-button:a", "- key k -> close-button:a"]
        );
        // each consent-bearing change prompts
        type Change = Box<dyn Fn(&mut Manifest)>;
        let cases: Vec<Change> = vec![
            Box::new(|m| m.permissions.push(Permission::Network)),
            Box::new(|m| m.actions[0].command = Some(vec!["y".into()])),
            Box::new(|m| m.actions[0].confirm = true),
            Box::new(|m| {
                m.events.push(garage_plugin::manifest::EventHook {
                    on: garage_plugin::manifest::EventName::SessionClosed,
                    every_secs: None,
                    command: vec!["h".into()],
                    timeout_secs: None,
                })
            }),
            Box::new(|m| {
                m.build.push(garage_plugin::manifest::Build {
                    platforms: vec![],
                    command: vec!["make".into()],
                    timeout_secs: None,
                })
            }),
            Box::new(|m| {
                m.release = Some(garage_plugin::manifest::Release {
                    repo: None,
                    tag: None,
                    checksums: "SUMS".into(),
                    assets: [("macos-aarch64".to_owned(), "a".to_owned())].into(),
                })
            }),
            Box::new(|m| {
                m.cli.push(garage_plugin::manifest::Cli {
                    name: "go".into(),
                    description: None,
                    command: vec!["x".into()],
                    fallback: false,
                })
            }),
            Box::new(|m| {
                m.widgets.push(garage_plugin::manifest::Widget {
                    id: "w".into(),
                    slot: garage_plugin::manifest::Slot::StripRight,
                })
            }),
            Box::new(|m| {
                m.panes.push(garage_plugin::manifest::Pane {
                    id: "p".into(),
                    title: "P".into(),
                    placement: garage_plugin::manifest::PanePlacement::Tile,
                    command: vec!["x".into()],
                    singleton: true,
                    size: None,
                })
            }),
            Box::new(|m| m.buttons.push(button("a", None, false))),
        ];
        for (n, change) in cases.iter().enumerate() {
            let mut changed = base.clone();
            change(&mut changed);
            assert!(
                s0.needs_consent(&Surface::of(&changed, Some("o/r"))),
                "case {n}"
            );
        }
        // fields that were skipped before: a fallback flag, a timer interval, a button's target, args and confirm
        let with = |extra: &str| {
            Surface::of(
                &m(&format!("permissions = [\"ui.widgets\"]\n{extra}")),
                Some("o/r"),
            )
        };
        let cli = |fb: bool| {
            with(&format!(
                "[[cli]]\nname = \"go\"\ncommand = [\"x\"]\nfallback = {fb}\n"
            ))
        };
        assert!(
            cli(false).needs_consent(&cli(true)),
            "taking over unknown first words is a new power"
        );
        let timer = |n: u32| {
            with(&format!(
                "[[events]]\non = \"timer\"\nevery_secs = {n}\ncommand = [\"t\"]\n"
            ))
        };
        assert!(
            timer(60).needs_consent(&timer(5)),
            "running twelve times as often is a change"
        );
        assert!(!timer(60).needs_consent(&timer(60)));
        let btn = |on: &str, args: Option<&str>, confirm: bool| {
            let mut mm = base.clone();
            mm.actions.push(garage_plugin::manifest::Action {
                id: "b".into(),
                title: "B".into(),
                contexts: vec![],
                command: Some(vec!["x".into()]),
                pane: None,
                default_key: None,
                confirm: false,
                timeout_secs: None,
            });
            mm.buttons.push(button(on, args, confirm));
            Surface::of(&mm, Some("o/r"))
        };
        assert!(
            btn("a", None, true).needs_consent(&btn("a", None, false)),
            "dropping confirm"
        );
        assert!(
            btn("a", None, false).needs_consent(&btn("b", None, false)),
            "a button retargeted"
        );
        assert!(
            btn("a", Some("1"), false).needs_consent(&btn("a", Some("2"), false)),
            "different args"
        );
        assert!(!btn("a", Some("1"), false).needs_consent(&btn("a", Some("1"), false)));
        // a code-only release update keeps its URL templates and does not re-prompt
        let rel = |ver: &str| {
            let mut mm = base.clone();
            mm.version = ver.into();
            mm.release = Some(garage_plugin::manifest::Release {
                repo: None,
                tag: None,
                checksums: "SUMS".into(),
                assets: [("macos-aarch64".to_owned(), "a".to_owned())].into(),
            });
            mm
        };
        assert!(
            !Surface::of(&rel("0.1.0"), Some("o/r"))
                .needs_consent(&Surface::of(&rel("0.2.0"), Some("o/r")))
        );
        let mut renamed = rel("0.2.0");
        renamed
            .release
            .as_mut()
            .unwrap()
            .assets
            .insert("macos-aarch64".into(), "other".into());
        assert!(
            Surface::of(&rel("0.1.0"), Some("o/r"))
                .needs_consent(&Surface::of(&renamed, Some("o/r"))),
            "a different asset name"
        );
        // moving the release repo changes the URLs
        let mut a = base.clone();
        a.release = Some(garage_plugin::manifest::Release {
            repo: None,
            tag: None,
            checksums: "S".into(),
            assets: [("macos-aarch64".to_owned(), "a".to_owned())].into(),
        });
        assert!(Surface::of(&a, Some("o/r")).needs_consent(&Surface::of(&a, Some("evil/r"))));
        let d = Diff::between(
            &Surface::of(&a, Some("o/r")),
            &Surface::of(&a, Some("evil/r")),
        );
        assert!(
            d.lines
                .iter()
                .any(|l| l.starts_with("+ url ") && l.contains("evil/r")),
            "{d:?}"
        );
    }

    #[test]
    fn a_timer_interval_is_shown_and_render_never_emits_a_control_byte() {
        let man = m("[[events]]\non = \"timer\"\nevery_secs = 30\ncommand = [\"./t.sh\"]\n");
        let got = render(&input(&man, Mode::Install, None));
        assert!(
            got.contains("event   timer every 30s       ./t.sh"),
            "{got}"
        );

        // A manifest that never went through validation: hostile text everywhere.
        let mut bad = m(CLOSE);
        let esc = "\u{1b}[2J\u{7}\u{202e}\u{200b}\r\n\u{9b}";
        bad.name = format!("N{esc}");
        bad.id = format!("i{esc}");
        bad.version = format!("1{esc}");
        bad.description = Some(format!("d{esc}"));
        bad.buttons[0].label = format!("l{esc}");
        bad.buttons[0].on_click = format!("c{esc}");
        bad.buttons[0].args = Some(serde_json::json!({ "k": esc }));
        bad.actions.push(garage_plugin::manifest::Action {
            id: format!("a{esc}"),
            title: format!("t{esc}"),
            contexts: vec![],
            command: Some(vec![format!("x{esc}"), esc.into()]),
            pane: None,
            default_key: Some(format!("k{esc}")),
            confirm: false,
            timeout_secs: None,
        });
        bad.cli.push(garage_plugin::manifest::Cli {
            name: format!("n{esc}"),
            description: None,
            command: vec![format!("c{esc}")],
            fallback: true,
        });
        bad.widgets.push(garage_plugin::manifest::Widget {
            id: format!("w{esc}"),
            slot: garage_plugin::manifest::Slot::StripRight,
        });
        let mut i = input(&bad, Mode::Update, None);
        i.source = format!("github o/r{esc}");
        i.license = Some(format!("L{esc}"));
        let d = Diff {
            lines: vec![format!("+ x{esc}")],
        };
        i.diff = Some(&d);
        let got = render(&i);
        assert!(
            !got.chars().any(|c| c != '\n' && is_hostile_char(c)),
            "{got:?}"
        );
        assert!(
            got.contains("\\u{001B}"),
            "the escape is shown, not dropped:\n{got}"
        );
        assert!(!question(&bad, &Mode::Install).chars().any(is_hostile_char));
        let s = Surface::of(&bad, Some("o/r"));
        assert!(
            s.commands
                .iter()
                .chain(&s.keys)
                .chain(&s.urls)
                .all(|l| !l.chars().any(is_hostile_char))
        );
        assert_eq!(quote(&["a\u{1b}b".to_owned()]), "'a\\u{001B}b'");
    }

    #[test]
    fn the_diff_section_and_question_follow_the_manifest() {
        let man = m(CLOSE);
        let d = Diff {
            lines: vec!["+ permission network".into()],
        };
        let got = render(&input(&man, Mode::Update, Some(&d)));
        assert!(
            got.contains("Changes since the approved version:\n  + permission network\n"),
            "{got}"
        );
        assert_eq!(
            question(&man, &Mode::Install),
            "Install close-button including DANGEROUS permissions (core.actions)?"
        );
        assert_eq!(question(&m(""), &Mode::Link), "Link close-button?");
    }
}
