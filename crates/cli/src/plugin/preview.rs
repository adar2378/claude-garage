//! The consent screen for `plugin install`, `link` and `update` (design
//! section 11, "honest consent"), and the update policy: which changes need a
//! new prompt. Pure functions over a manifest; nothing here touches the disk.

use std::collections::BTreeSet;

use garage_plugin::core_actions;
use garage_plugin::manifest::{Manifest, Permission};

use super::fetch::{all_release_urls, builds_for_os, ReleasePlan};

pub const UPDATE_POLICY: &str = "Later updates that change only plugin code (no new commands, permissions or URLs) apply without asking again.";
pub const NOT_SANDBOXED: &str = "Plugins are not sandboxed. They run as you and can read, write and delete anything you can.";

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

pub fn quote(argv: &[String]) -> String {
    argv.iter()
        .map(|a| {
            if !a.is_empty() && a.bytes().all(|b| b.is_ascii_alphanumeric() || b"-_./:=@%+,{}".contains(&b)) {
                a.clone()
            } else {
                format!("'{}'", a.replace('\'', "'\\''"))
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

impl Surface {
    pub fn of(m: &Manifest, install_repo: Option<&str>) -> Self {
        let mut s = Surface { permissions: m.permission_names().into_iter().collect(), ..Default::default() };
        for b in &m.build {
            s.commands.insert(format!("build: {}", quote(&b.command)));
        }
        for a in &m.actions {
            if let Some(c) = &a.command {
                s.commands.insert(format!("action {}: {}", a.id, quote(c)));
            }
            if let Some(k) = &a.default_key {
                s.keys.insert(format!("{k} -> {}:{}", m.id, a.id));
            }
        }
        for e in &m.events {
            s.commands.insert(format!("event {}: {}", e.on.as_str(), quote(&e.command)));
        }
        for p in &m.panes {
            s.commands.insert(format!("pane {}: {}", p.id, quote(&p.command)));
        }
        for c in &m.cli {
            s.commands.insert(format!("cli {}: {}", c.name, quote(&c.command)));
        }
        s.urls = all_release_urls(m, install_repo).into_iter().collect();
        s
    }

    /// Does going from `self` (installed) to `new` need the user's yes?
    pub fn needs_consent(&self, new: &Surface) -> bool {
        self.permissions != new.permissions || self.commands != new.commands || self.urls != new.urls
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
        "core.actions" => "trigger garage's own actions, such as closing a session (closes and restarts still need a second confirmation)",
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
    o.push_str(&format!("{verb}: {} {} ({})\n", m.name, m.version, m.id));
    o.push_str(&format!("Source: {}\n", i.source));
    if let Some(l) = &i.license {
        o.push_str(&format!("License: {l}\n"));
    }
    if let Some(d) = &m.description {
        o.push_str(&format!("{d}\n"));
    }
    o.push_str(&format!("\n{NOT_SANDBOXED}\n"));

    o.push_str("\nPermissions:\n");
    if m.permissions.is_empty() {
        o.push_str("  none declared\n");
    }
    for p in &m.permissions {
        let danger = if p.is_dangerous() { "DANGEROUS" } else { "" };
        o.push_str(&format!("  {:<16} {:<9}  {}\n", p.as_str(), danger, permission_blurb(p.as_str())));
    }

    // Commands: only what will actually run, in the order it runs.
    o.push_str("\nCommands garage will run:\n");
    let mut any = false;
    let mut line = |o: &mut String, kind: &str, name: &str, argv: &[String]| {
        any = true;
        o.push_str(&format!("  {kind:<8}{name:<22}{}\n", quote(argv)));
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
    for e in &m.events {
        line(&mut o, "event", e.on.as_str(), &e.command);
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
            o.push_str(&format!("  {}\n  {}\n  (checked against the checksums file; no [[build]] runs)\n", r.asset_url, r.checksums_url));
        }
        _ => o.push_str("  none\n"),
    }

    let mut ui = Vec::new();
    for a in &m.actions {
        if let Some(k) = &a.default_key {
            ui.push(format!("  key     {k} -> {}:{} ({})", m.id, a.id, a.title));
        }
    }
    for b in &m.buttons {
        let target = match core_actions::lookup(&b.on_click) {
            Some(c) if c.dangerous => format!("{} (DANGEROUS core action; asks for a second click)", b.on_click),
            _ => b.on_click.clone(),
        };
        ui.push(format!("  button  {} {:?} triggers {target}", b.slot.as_str(), b.label));
    }
    for w in &m.widgets {
        ui.push(format!("  widget  {} in {}", w.id, w.slot.as_str()));
    }
    for p in &m.panes {
        ui.push(format!("  pane    {} ({})", p.id, format!("{:?}", p.placement).to_lowercase()));
    }
    for c in &m.cli {
        let fb = if c.fallback { " (also receives any unknown first word)" } else { "" };
        ui.push(format!("  cli     claude-garage {}{fb}", c.name));
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
            o.push_str(&format!("  {l}\n"));
        }
    }
    o.push_str(&format!("\n{UPDATE_POLICY}\n"));
    let _ = i.install_repo;
    o
}

/// The one-line question.
pub fn question(m: &Manifest, mode: &Mode) -> String {
    let verb = match mode {
        Mode::Install => "Install",
        Mode::Link => "Link",
        Mode::Update => "Update",
    };
    if has_dangerous(m) {
        let names: Vec<&str> = m.permissions.iter().filter(|p| p.is_dangerous()).map(|p: &Permission| p.as_str()).collect();
        format!("{verb} {} including DANGEROUS permissions ({})?", m.id, names.join(", "))
    } else {
        format!("{verb} {}?", m.id)
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

    const CLOSE: &str = "permissions = [\"core.actions\"]\n[[buttons]]\nslot = \"tile.title.right\"\nlabel = \" x \"\non_click = \"session.close\"\nconfirm = true\n";

    fn input<'a>(manifest: &'a Manifest, mode: Mode, diff: Option<&'a Diff>) -> PreviewInput<'a> {
        PreviewInput { manifest, mode, source: "github owner/repo@9f2c1ab (ref v0.1.0)".into(), license: Some("LICENSE file present".into()), release: None, install_repo: Some("owner/repo"), diff }
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
        assert!(got.contains("sessions.input   DANGEROUS  type text into any session"), "{got}");
        assert!(got.lines().any(|l| l.contains("network") && !l.contains("DANGEROUS")));
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
        let man = m("[[build]]\ncommand = [\"make\"]\n[release]\nchecksums = \"SUMS\"\n[release.assets]\nmacos-aarch64 = \"a\"\n");
        let plan = ReleasePlan { platform: "macos-aarch64".into(), asset: "a".into(), asset_url: "https://github.com/owner/repo/releases/download/v0.1.0/a".into(), checksums_url: "https://github.com/owner/repo/releases/download/v0.1.0/SUMS".into() };
        let mut i = input(&man, Mode::Install, None);
        i.release = Some(&plan);
        let got = render(&i);
        assert!(!got.contains("make"), "{got}");
        assert!(got.contains("https://github.com/owner/repo/releases/download/v0.1.0/a\n  https://github.com/owner/repo/releases/download/v0.1.0/SUMS"));
        let got = render(&input(&man, Mode::Install, None));
        assert!(got.contains("build   (at install)          make") && got.contains("Downloads:\n  none"));
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
        let base = m("permissions = [\"ui.widgets\"]\n[[actions]]\nid = \"a\"\ntitle = \"A\"\ncommand = [\"x\"]\ndefault_key = \"k\"\n");
        let s0 = Surface::of(&base, Some("o/r"));
        // a new version, description and key are code-only
        let mut bumped = base.clone();
        bumped.version = "0.2.0".into();
        bumped.description = Some("changed words".into());
        bumped.actions[0].default_key = Some("z".into());
        bumped.actions[0].title = "Renamed".into();
        let s1 = Surface::of(&bumped, Some("o/r"));
        assert!(!s0.needs_consent(&s1));
        assert_eq!(Diff::between(&s0, &s1).lines, ["+ key z -> close-button:a", "- key k -> close-button:a"]);
        // each consent-bearing change prompts
        type Change = Box<dyn Fn(&mut Manifest)>;
        let cases: Vec<Change> = vec![
            Box::new(|m| m.permissions.push(Permission::Network)),
            Box::new(|m| m.actions[0].command = Some(vec!["y".into()])),
            Box::new(|m| m.events.push(garage_plugin::manifest::EventHook { on: garage_plugin::manifest::EventName::SessionClosed, every_secs: None, command: vec!["h".into()], timeout_secs: None })),
            Box::new(|m| m.build.push(garage_plugin::manifest::Build { platforms: vec![], command: vec!["make".into()], timeout_secs: None })),
            Box::new(|m| {
                m.release = Some(garage_plugin::manifest::Release { repo: None, tag: None, checksums: "SUMS".into(), assets: [("macos-aarch64".to_owned(), "a".to_owned())].into() })
            }),
        ];
        for (n, change) in cases.iter().enumerate() {
            let mut changed = base.clone();
            change(&mut changed);
            assert!(s0.needs_consent(&Surface::of(&changed, Some("o/r"))), "case {n}");
        }
        // moving the release repo changes the URLs
        let mut a = base.clone();
        a.release = Some(garage_plugin::manifest::Release { repo: None, tag: None, checksums: "S".into(), assets: [("macos-aarch64".to_owned(), "a".to_owned())].into() });
        assert!(Surface::of(&a, Some("o/r")).needs_consent(&Surface::of(&a, Some("evil/r"))));
        let d = Diff::between(&Surface::of(&a, Some("o/r")), &Surface::of(&a, Some("evil/r")));
        assert!(d.lines.iter().any(|l| l.starts_with("+ url ") && l.contains("evil/r")), "{d:?}");
    }

    #[test]
    fn the_diff_section_and_question_follow_the_manifest() {
        let man = m(CLOSE);
        let d = Diff { lines: vec!["+ permission network".into()] };
        let got = render(&input(&man, Mode::Update, Some(&d)));
        assert!(got.contains("Changes since the approved version:\n  + permission network\n"), "{got}");
        assert_eq!(question(&man, &Mode::Install), "Install close-button including DANGEROUS permissions (core.actions)?");
        assert_eq!(question(&m(""), &Mode::Link), "Link close-button?");
    }
}
