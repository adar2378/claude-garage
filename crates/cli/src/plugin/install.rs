//! `plugin install`, `link`, `update` and `remove` as sequences over the
//! seams (`Git`, `Fetch`, `Builder`, `Admin`, `Ui`). The order is the
//! security story:
//!
//! 1. clone and read the manifest (garage's own work; no plugin code runs),
//! 2. print the preview, 3. get consent, 4. only then build or download,
//! 5. swap the finished directory into place, 6. record the approval.
//!
//! Anything that fails before step 6 leaves the registry and the previous
//! install exactly as they were.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use garage_daemon::plugins::admin::{Registration, delete_files};
use garage_daemon::plugins::layout::{MANIFEST_FILE, Paths, PluginsFile, Source};
use garage_plugin::manifest::{Manifest, sha256_hex};
use garage_plugin::{check_cli_conflicts, check_host, parse};

use super::admin::Admin;
use super::fetch::{Builder, Fetch, ReleasePlan, download_release, release_plan, run_builds};
use super::preview::{Diff, Mode, PreviewInput, Surface, has_dangerous, question, render};
use super::registry::load_all;
use super::source::{Git, GithubSource, find_symlink_escape, parse_github};

/// Terminal seam: output, a yes/no question, and whether a person is there.
pub trait Ui {
    fn say(&mut self, text: &str);
    fn interactive(&self) -> bool;
    fn confirm(&mut self, question: &str) -> bool;
}

pub struct Cx<'a> {
    pub paths: Paths,
    pub git: &'a dyn Git,
    pub fetch: &'a dyn Fetch,
    pub builder: &'a dyn Builder,
    pub admin: &'a dyn Admin,
    pub ui: &'a mut dyn Ui,
    /// `https://github.com`; tests point it at a local `file://` folder.
    pub github_base: String,
    pub now_ms: i64,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct Opts {
    pub yes: bool,
    pub allow_dangerous: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Done {
    pub id: String,
    pub version: String,
    pub commit: Option<String>,
}

/// A scratch directory removed on drop unless it was moved into place.
struct Stage {
    path: PathBuf,
    armed: bool,
}

impl Stage {
    fn new(paths: &Paths, label: &str) -> Result<Self, String> {
        static SEQ: AtomicU64 = AtomicU64::new(0);
        std::fs::create_dir_all(paths.tmp())
            .map_err(|e| format!("cannot create {}: {e}", paths.tmp().display()))?;
        let path = paths.tmp().join(format!(
            "stage-{label}-{}-{}",
            std::process::id(),
            SEQ.fetch_add(1, Ordering::Relaxed)
        ));
        Ok(Self { path, armed: true })
    }

    fn keep(&mut self) {
        self.armed = false;
    }
}

impl Drop for Stage {
    fn drop(&mut self) {
        if self.armed {
            let _ = std::fs::remove_dir_all(&self.path);
        }
    }
}

fn problems_text(file: &str, ps: &[garage_plugin::Problem]) -> String {
    let mut s = format!("{file} is not usable:");
    for p in ps {
        s.push_str(&format!("\n  {p}"));
    }
    s
}

/// Read and validate the manifest in `dir` (parse, structure, host checks).
/// Returns the manifest and its raw text.
pub(super) fn read_manifest(dir: &Path) -> Result<(Manifest, String), String> {
    let file = dir.join(MANIFEST_FILE);
    let raw = std::fs::read_to_string(&file)
        .map_err(|e| format!("cannot read {}: {e}", file.display()))?;
    let m = parse(&raw).map_err(|ps| problems_text(MANIFEST_FILE, &ps))?;
    let host = check_host(&m);
    if !host.is_empty() {
        return Err(problems_text(MANIFEST_FILE, &host));
    }
    Ok((m, raw))
}

fn license_of(plugin_dir: &Path, root: &Path) -> Option<String> {
    for dir in [plugin_dir, root] {
        if let Ok(rd) = std::fs::read_dir(dir) {
            let mut names: Vec<String> = rd
                .flatten()
                .map(|e| e.file_name().to_string_lossy().into_owned())
                .collect();
            names.sort();
            if let Some(n) = names.iter().find(|n| {
                let u = n.to_ascii_uppercase();
                u.starts_with("LICENSE") || u.starts_with("LICENCE") || u.starts_with("COPYING")
            }) {
                return Some(format!("{n} in the repository"));
            }
        }
    }
    None
}

/// Show the preview, then decide. `--yes` skips the question but a dangerous
/// permission also needs `--allow-dangerous`; with no person at the terminal
/// and no `--yes` there is nobody to ask.
fn consent(ui: &mut dyn Ui, opts: &Opts, m: &Manifest, mode: &Mode) -> Result<(), String> {
    if opts.yes {
        if has_dangerous(m) && !opts.allow_dangerous {
            return Err(format!(
                "{} declares DANGEROUS permissions; scripts must pass --allow-dangerous as well as --yes. Nothing was changed.",
                m.id
            ));
        }
        return Ok(());
    }
    if !ui.interactive() {
        return Err("not running in a terminal: re-run with --yes (and --allow-dangerous if the plugin declares dangerous permissions). Nothing was changed.".into());
    }
    if ui.confirm(&question(m, mode)) {
        Ok(())
    } else {
        Err("declined; nothing was changed".into())
    }
}

fn checkout(
    cx: &Cx,
    src: &GithubSource,
    git_ref: Option<&str>,
    stage: &Stage,
) -> Result<String, String> {
    let url = src.clone_url(&cx.github_base);
    let sha = cx.git.clone_shallow(&url, git_ref, &stage.path)?;
    if let Some(bad) = find_symlink_escape(&stage.path)? {
        return Err(format!(
            "{} contains a symlink that points outside the repository or at a folder ({}); refusing it",
            src.repo,
            bad.display()
        ));
    }
    Ok(sha)
}

fn plugin_dir_in(stage: &Path, subdir: Option<&str>) -> Result<PathBuf, String> {
    if let Some(s) = subdir
        && let Some(bad) = super::source::find_symlink_in_path(stage, s)
    {
        return Err(format!(
            "the plugin folder {s} goes through a symlink ({}); refusing it",
            bad.display()
        ));
    }
    let dir = match subdir {
        Some(s) => stage.join(s),
        None => stage.to_path_buf(),
    };
    if !dir.join(MANIFEST_FILE).is_file() {
        return Err(format!(
            "no {MANIFEST_FILE} found in {}",
            match subdir {
                Some(s) => format!("the {s} folder of the repository"),
                None =>
                    "the repository root (for a plugin in a folder use owner/repo/folder)".into(),
            }
        ));
    }
    Ok(dir)
}

/// Build steps (when no release asset applies) or the verified download,
/// inside the staged directory.
fn acquire(
    cx: &Cx,
    m: &Manifest,
    plugin_dir: &Path,
    plan: Option<&ReleasePlan>,
) -> Result<(), String> {
    match plan {
        Some(p) => download_release(cx.fetch, p, plugin_dir, &m.id),
        None => run_builds(
            cx.builder,
            m,
            plugin_dir,
            &cx.paths.data(&m.id),
            &cx.paths.log(&m.id),
        ),
    }
}

/// Move `stage` to `installed/<id>`. A previous directory is parked and
/// returned so the caller can restore it if registration then fails.
fn swap_in(paths: &Paths, stage: &mut Stage, id: &str) -> Result<Option<PathBuf>, String> {
    let dest = paths.installed(id);
    std::fs::create_dir_all(dest.parent().expect("installed has a parent"))
        .map_err(|e| e.to_string())?;
    let parked = if dest.exists() {
        let p = paths.tmp().join(format!("old-{id}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&p);
        std::fs::rename(&dest, &p).map_err(|e| format!("cannot move the old {id} aside: {e}"))?;
        Some(p)
    } else {
        None
    };
    if let Err(e) = std::fs::rename(&stage.path, &dest) {
        if let Some(p) = &parked {
            let _ = std::fs::rename(p, &dest);
        }
        return Err(format!("cannot move the new {id} into place: {e}"));
    }
    stage.keep();
    Ok(parked)
}

fn rollback_swap(paths: &Paths, id: &str, parked: Option<PathBuf>) {
    let dest = paths.installed(id);
    let _ = std::fs::remove_dir_all(&dest);
    if let Some(p) = parked {
        let _ = std::fs::rename(p, dest);
    }
}

fn refuse_cli_conflict(paths: &Paths, m: &Manifest) -> Result<(), String> {
    let existing = load_all(paths)?;
    let mut candidates: Vec<(&Manifest, bool)> = existing
        .iter()
        .filter(|l| {
            l.entry.enabled
                && l.entry.granted_permissions.is_some()
                && l.problems.is_empty()
                && l.entry.id != m.id
        })
        .filter_map(|l| l.manifest.as_ref().map(|x| (x, l.entry.is_first_party())))
        .collect();
    candidates.push((m, false));
    match check_cli_conflicts(&candidates)
        .into_iter()
        .find(|c| c.plugin == m.id)
    {
        Some(c) => Err(format!("{}: {}", m.id, c.message)),
        None => Ok(()),
    }
}

/// `plugin install owner/repo[/subdir] [--ref]`.
pub fn install(
    cx: &mut Cx,
    spec: &str,
    git_ref: Option<&str>,
    opts: &Opts,
) -> Result<Done, String> {
    let src = parse_github(spec)?;
    let mut stage = Stage::new(&cx.paths, "install")?;
    let sha = checkout(cx, &src, git_ref, &stage)?;
    let plugin_dir = plugin_dir_in(&stage.path, src.subdir.as_deref())?;
    let (m, raw) =
        read_manifest(&plugin_dir).map_err(|e| format!("{e}\nNothing was installed."))?;

    if let Some(e) = PluginsFile::read(&cx.paths)?
        .plugins
        .iter()
        .find(|e| e.id == m.id)
    {
        let how = if matches!(e.source, Source::Github { .. }) {
            format!("run `claude-garage plugin update {}`", m.id)
        } else {
            format!("run `claude-garage plugin remove {}` first", m.id)
        };
        return Err(format!("{} is already installed; {how}", m.id));
    }
    refuse_cli_conflict(&cx.paths, &m)?;

    let plan = release_plan(&m, Some(&src.repo))?;
    let source_line = format!(
        "github {}@{}{}",
        src.display(),
        &sha[..sha.len().min(7)],
        git_ref.map(|r| format!(" (ref {r})")).unwrap_or_default()
    );
    let text = render(&PreviewInput {
        manifest: &m,
        mode: Mode::Install,
        source: source_line,
        license: Some(license_of(&plugin_dir, &stage.path).unwrap_or_else(|| "none found".into())),
        release: plan.as_ref(),
        install_repo: Some(&src.repo),
        diff: None,
    });
    cx.ui.say(&text);
    consent(cx.ui, opts, &m, &Mode::Install)?;

    acquire(cx, &m, &plugin_dir, plan.as_ref())
        .map_err(|e| format!("{e}\nNothing was installed."))?;
    let parked = swap_in(&cx.paths, &mut stage, &m.id)?;
    let reg = Registration {
        id: m.id.clone(),
        source: Source::Github {
            repo: src.repo.clone(),
            subdir: src.subdir.clone(),
            git_ref: git_ref.map(str::to_owned),
            commit: Some(sha.clone()),
        },
        granted_permissions: m.permission_names(),
        manifest_sha256: Some(sha256_hex(raw.as_bytes())),
        installed_at: Some(cx.now_ms),
    };
    if let Err(e) = cx.admin.register(&reg) {
        rollback_swap(&cx.paths, &m.id, parked);
        return Err(format!(
            "could not record the install: {e}\nNothing was installed."
        ));
    }
    if let Some(p) = parked {
        let _ = std::fs::remove_dir_all(p);
    }
    cx.ui.say(&format!(
        "Installed {} {} ({}).",
        m.id,
        m.version,
        cx.admin.via()
    ));
    Ok(Done {
        id: m.id,
        version: m.version,
        commit: Some(sha),
    })
}

/// `plugin link <path>`: register a folder in place. No build, no download.
/// Also the way to approve permissions a changed manifest added.
pub fn link(cx: &mut Cx, path: &Path, opts: &Opts) -> Result<Done, String> {
    let abs =
        std::path::absolute(path).map_err(|e| format!("cannot resolve {}: {e}", path.display()))?;
    if !abs.is_dir() {
        return Err(format!("{} is not a folder", abs.display()));
    }
    let (m, raw) = read_manifest(&abs)?;
    let existing = PluginsFile::read(&cx.paths)?
        .plugins
        .into_iter()
        .find(|e| e.id == m.id);
    let mut diff = None;
    if let Some(e) = &existing {
        match &e.source {
            Source::Bundled { .. } => {
                return Err(format!(
                    "{} is a bundled plugin; use `plugin enable`/`disable`",
                    m.id
                ));
            }
            Source::Github { .. } => {
                return Err(format!(
                    "{} is installed from GitHub; run `claude-garage plugin remove {}` first",
                    m.id, m.id
                ));
            }
            Source::Link { .. } => {}
        }
        if let Some(granted) = &e.granted_permissions {
            let old = Surface {
                permissions: granted.iter().cloned().collect(),
                ..Default::default()
            };
            let new = Surface {
                permissions: m.permission_names().into_iter().collect(),
                ..Default::default()
            };
            diff = Some(Diff::between(&old, &new));
        }
    }
    refuse_cli_conflict(&cx.paths, &m)?;
    let text = render(&PreviewInput {
        manifest: &m,
        mode: Mode::Link,
        source: format!("dev {}", abs.display()),
        license: None,
        release: None,
        install_repo: None,
        diff: diff.as_ref(),
    });
    cx.ui.say(&text);
    consent(cx.ui, opts, &m, &Mode::Link)?;
    let reg = Registration {
        id: m.id.clone(),
        source: Source::Link {
            path: abs.to_string_lossy().into_owned(),
        },
        granted_permissions: m.permission_names(),
        manifest_sha256: Some(sha256_hex(raw.as_bytes())),
        installed_at: Some(cx.now_ms),
    };
    cx.admin.register(&reg)?;
    cx.ui.say(&format!(
        "Linked {} {} ({}).",
        m.id,
        m.version,
        cx.admin.via()
    ));
    Ok(Done {
        id: m.id,
        version: m.version,
        commit: None,
    })
}

/// What `plugin update` did for one plugin.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    Updated {
        id: String,
        from: String,
        to: String,
        prompted: bool,
    },
    UpToDate {
        id: String,
    },
    Skipped {
        id: String,
        why: String,
    },
    Failed {
        id: String,
        error: String,
    },
}

/// `plugin update [id|--all]`. Links and bundled plugins are skipped. A
/// failed update leaves the old plugin working.
pub fn update(cx: &mut Cx, only: Option<&str>, opts: &Opts) -> Result<Vec<Outcome>, String> {
    let entries = PluginsFile::read(&cx.paths)?.plugins;
    if let Some(id) = only
        && !entries.iter().any(|e| e.id == id)
    {
        return Err(format!("no plugin {id:?}"));
    }
    let mut out = Vec::new();
    for e in entries
        .into_iter()
        .filter(|e| only.is_none_or(|id| e.id == id))
    {
        let Source::Github {
            repo,
            subdir,
            git_ref,
            commit,
        } = e.source.clone()
        else {
            let why = match e.source {
                Source::Bundled { .. } => "bundled plugins update with garage".to_owned(),
                _ => "linked folders are yours to update".to_owned(),
            };
            out.push(Outcome::Skipped {
                id: e.id.clone(),
                why,
            });
            continue;
        };
        let src = GithubSource { repo, subdir };
        match update_one(cx, &e.id, &src, git_ref.as_deref(), commit.as_deref(), opts) {
            Ok(o) => out.push(o),
            Err(error) => out.push(Outcome::Failed {
                id: e.id.clone(),
                error,
            }),
        }
    }
    for o in &out {
        let line = match o {
            Outcome::Updated {
                id,
                from,
                to,
                prompted,
            } => format!(
                "{id}: updated {from} -> {to}{}",
                if *prompted {
                    ""
                } else {
                    " (code only, no prompt needed)"
                }
            ),
            Outcome::UpToDate { id } => format!("{id}: already up to date"),
            Outcome::Skipped { id, why } => format!("{id}: skipped, {why}"),
            Outcome::Failed { id, error } => {
                format!("{id}: FAILED, the installed version is unchanged: {error}")
            }
        };
        cx.ui.say(&line);
    }
    Ok(out)
}

fn update_one(
    cx: &mut Cx,
    id: &str,
    src: &GithubSource,
    git_ref: Option<&str>,
    commit: Option<&str>,
    opts: &Opts,
) -> Result<Outcome, String> {
    let mut stage = Stage::new(&cx.paths, "update")?;
    let sha = checkout(cx, src, git_ref, &stage)?;
    if commit == Some(sha.as_str()) {
        return Ok(Outcome::UpToDate { id: id.to_owned() });
    }
    let plugin_dir = plugin_dir_in(&stage.path, src.subdir.as_deref())?;
    let (m, raw) = read_manifest(&plugin_dir)?;
    if m.id != id {
        return Err(format!(
            "the new manifest's id is {:?}, not {id:?}; remove and install it again if the plugin was renamed",
            m.id
        ));
    }
    let installed_dir = super::registry::manifest_dir(
        &cx.paths,
        &garage_daemon::plugins::layout::Entry::new(
            id,
            Source::Github {
                repo: src.repo.clone(),
                subdir: src.subdir.clone(),
                git_ref: None,
                commit: None,
            },
        ),
    );
    let old = read_manifest(&installed_dir).map(|(m, _)| m).ok();
    let new_surface = Surface::of(&m, Some(&src.repo));
    // An installed version we cannot read cannot vouch for anything: ask.
    let old_surface = old.as_ref().map(|o| Surface::of(o, Some(&src.repo)));
    let needs = old_surface
        .as_ref()
        .is_none_or(|o| o.needs_consent(&new_surface));
    let from = old
        .as_ref()
        .map(|o| o.version.clone())
        .unwrap_or_else(|| "?".into());
    // Even a code-only update must not take a command name another plugin holds.
    refuse_cli_conflict(&cx.paths, &m)?;
    if needs {
        let diff = old_surface.as_ref().map(|o| Diff::between(o, &new_surface));
        let plan = release_plan(&m, Some(&src.repo))?;
        let text = render(&PreviewInput {
            manifest: &m,
            mode: Mode::Update,
            source: format!("github {}@{}", src.display(), &sha[..sha.len().min(7)]),
            license: None,
            release: plan.as_ref(),
            install_repo: Some(&src.repo),
            diff: diff.as_ref(),
        });
        cx.ui.say(&text);
        consent(cx.ui, opts, &m, &Mode::Update)?;
    } else {
        cx.ui.say(&format!(
            "{id}: {from} -> {} at {}; commands, permissions and URLs are unchanged.",
            m.version,
            &sha[..sha.len().min(7)]
        ));
    }
    let plan = release_plan(&m, Some(&src.repo))?;
    acquire(cx, &m, &plugin_dir, plan.as_ref())?;
    let parked = swap_in(&cx.paths, &mut stage, id)?;
    let reg = Registration {
        id: id.to_owned(),
        source: Source::Github {
            repo: src.repo.clone(),
            subdir: src.subdir.clone(),
            git_ref: git_ref.map(str::to_owned),
            commit: Some(sha),
        },
        granted_permissions: m.permission_names(),
        manifest_sha256: Some(sha256_hex(raw.as_bytes())),
        installed_at: Some(cx.now_ms),
    };
    if let Err(e) = cx.admin.register(&reg) {
        rollback_swap(&cx.paths, id, parked);
        return Err(format!("could not record the update: {e}"));
    }
    if let Some(p) = parked {
        let _ = std::fs::remove_dir_all(p);
    }
    Ok(Outcome::Updated {
        id: id.to_owned(),
        from,
        to: m.version,
        prompted: needs,
    })
}

/// `plugin remove <id> [--purge]`.
pub fn remove(cx: &mut Cx, id: &str, purge: bool) -> Result<(), String> {
    let file = PluginsFile::read(&cx.paths)?;
    let Some(e) = file.plugins.iter().find(|e| e.id == id) else {
        return Err(format!("no plugin {id:?}"));
    };
    if e.is_first_party() {
        return Err(format!(
            "{id} is a bundled plugin and cannot be removed; use `claude-garage plugin disable {id}`"
        ));
    }
    cx.admin.remove(id)?;
    delete_files(&cx.paths, id, purge).map_err(|er| {
        format!("removed {id} from the registry but could not delete its files: {er}")
    })?;
    cx.ui.say(&format!(
        "Removed {id}{}.",
        if purge {
            " and its data and logs"
        } else {
            " (its data and logs are kept; --purge deletes them)"
        }
    ));
    Ok(())
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use crate::plugin::admin::DirectAdmin;
    use crate::plugin::source::RealGit;
    use crate::plugin::testkit::{FakeFetch, FakeUi, RecordingBuilder, repo, toml};
    use garage_plugin::argv::platform_id;

    struct Rig {
        garage: tempfile::TempDir,
        remote: tempfile::TempDir,
        builder: RecordingBuilder,
        fetch: FakeFetch,
    }

    impl Rig {
        fn new() -> Self {
            Self {
                garage: tempfile::tempdir().unwrap(),
                remote: tempfile::tempdir().unwrap(),
                builder: RecordingBuilder {
                    produces: Some("built.txt".into()),
                    ..Default::default()
                },
                fetch: FakeFetch::default(),
            }
        }

        fn paths(&self) -> Paths {
            Paths::new(self.garage.path())
        }

        fn run<T>(&self, ui: &mut FakeUi, f: impl FnOnce(&mut Cx) -> T) -> T {
            let admin = DirectAdmin {
                paths: self.paths(),
            };
            let mut cx = Cx {
                paths: self.paths(),
                git: &RealGit,
                fetch: &self.fetch,
                builder: &self.builder,
                admin: &admin,
                ui,
                github_base: format!("file://{}", self.remote.path().display()),
                now_ms: 1234,
            };
            f(&mut cx)
        }

        fn install(&self, ui: &mut FakeUi, spec: &str, opts: Opts) -> Result<Done, String> {
            self.run(ui, |cx| install(cx, spec, None, &opts))
        }

        fn entry(&self, id: &str) -> Option<garage_daemon::plugins::layout::Entry> {
            PluginsFile::read(&self.paths())
                .unwrap()
                .plugins
                .into_iter()
                .find(|e| e.id == id)
        }

        fn no_leftovers(&self) {
            let tmp = self.paths().tmp();
            let left: Vec<_> = std::fs::read_dir(&tmp)
                .map(|r| r.flatten().map(|e| e.file_name()).collect())
                .unwrap_or_default();
            assert!(left.is_empty(), "scratch left behind: {left:?}");
        }
    }

    const YES: Opts = Opts {
        yes: true,
        allow_dangerous: false,
    };

    fn build_manifest(id: &str) -> String {
        toml(
            id,
            "permissions = [\"ui.widgets\"]\n[[build]]\ncommand = [\"make\", \"all\"]\n[[actions]]\nid = \"go\"\ntitle = \"Go\"\ncommand = [\"{plugin_dir}/go.sh\"]\n",
        )
    }

    #[test]
    fn install_clones_previews_asks_then_builds_swaps_in_and_records_the_grant_and_commit() {
        let r = Rig::new();
        let dir = repo::make(
            r.remote.path(),
            "owner/tools",
            &[
                ("plugins/hello/garage-plugin.toml", &build_manifest("hello")),
                ("LICENSE", "MIT"),
                ("plugins/hello/go.sh", "#!/bin/sh\n"),
            ],
        );
        let head = repo::git(&dir, &["rev-parse", "HEAD"]);
        let calls = r.builder.calls.clone();
        let mut ui = FakeUi::new(true, &[true]);
        ui.on_confirm = Some(Box::new(move || {
            assert!(
                calls.lock().unwrap().is_empty(),
                "no build before the answer"
            )
        }));
        let done = r
            .install(&mut ui, "owner/tools/plugins/hello", Opts::default())
            .unwrap();
        assert_eq!(
            done,
            Done {
                id: "hello".into(),
                version: "1.0.0".into(),
                commit: Some(head.clone())
            }
        );

        assert!(
            ui.out.starts_with("Install: hello 1.0.0 (hello)"),
            "{}",
            ui.out
        );
        assert!(
            ui.out.contains("make all")
                && ui.out.contains("go.sh")
                && ui.out.contains("LICENSE in the repository")
        );
        assert_eq!(ui.asked, ["Install hello?"]);
        let built = r.builder.calls.lock().unwrap().clone();
        assert_eq!(built, [vec!["make".to_owned(), "all".into()]]);

        let e = r.entry("hello").unwrap();
        assert_eq!(
            e.granted_permissions.as_deref(),
            Some(&["ui.widgets".to_owned()][..])
        );
        assert_eq!(e.installed_at, Some(1234));
        assert_eq!(
            e.source,
            Source::Github {
                repo: "owner/tools".into(),
                subdir: Some("plugins/hello".into()),
                git_ref: None,
                commit: Some(head)
            }
        );
        assert_eq!(e.manifest_sha256.as_deref().map(str::len), Some(64));
        let installed = r.paths().installed("hello");
        assert!(installed.join("plugins/hello/garage-plugin.toml").is_file());
        assert!(
            installed.join("plugins/hello/built.txt").is_file(),
            "built inside the plugin folder, then moved"
        );
        assert!(!installed.join(".git").exists());
        assert!(r.paths().data("hello").is_dir() && r.paths().token("hello").is_file());
        r.no_leftovers();
        // the loader sees it as active
        let l = crate::plugin::registry::load_all(&r.paths()).unwrap();
        assert!(l[0].usable(), "{:?}", l[0].problems);
    }

    #[test]
    fn declining_or_having_no_terminal_changes_nothing_and_runs_nothing() {
        let r = Rig::new();
        repo::make(
            r.remote.path(),
            "o/p",
            &[("garage-plugin.toml", &build_manifest("pp"))],
        );
        let mut ui = FakeUi::new(true, &[false]);
        assert!(
            r.install(&mut ui, "o/p", Opts::default())
                .unwrap_err()
                .contains("declined")
        );
        let mut ui = FakeUi::new(false, &[]);
        let e = r.install(&mut ui, "o/p", Opts::default()).unwrap_err();
        assert!(e.contains("--yes"), "{e}");
        assert!(
            ui.asked.is_empty() && ui.out.contains("Install: pp"),
            "the preview is still printed"
        );
        assert!(r.builder.calls.lock().unwrap().is_empty(), "no build ran");
        assert!(r.entry("pp").is_none() && !r.paths().installed("pp").exists());
        assert!(
            !r.paths().registry_file().exists(),
            "the registry was never touched"
        );
        r.no_leftovers();
    }

    #[test]
    fn dangerous_permissions_need_allow_dangerous_even_with_yes_and_the_question_names_them() {
        let r = Rig::new();
        repo::make(
            r.remote.path(),
            "o/d",
            &[(
                "garage-plugin.toml",
                &toml("typist", "permissions = [\"sessions.input\", \"network\"]"),
            )],
        );
        let mut ui = FakeUi::new(false, &[]);
        let e = r.install(&mut ui, "o/d", YES).unwrap_err();
        assert!(
            e.contains("--allow-dangerous") && r.entry("typist").is_none(),
            "{e}"
        );
        assert!(ui.out.contains("sessions.input   DANGEROUS"), "{}", ui.out);
        let mut ui = FakeUi::new(false, &[]);
        r.install(
            &mut ui,
            "o/d",
            Opts {
                yes: true,
                allow_dangerous: true,
            },
        )
        .unwrap();
        assert_eq!(
            r.entry("typist").unwrap().granted_permissions.unwrap(),
            ["sessions.input", "network"]
        );
        assert!(ui.asked.is_empty(), "--yes asks nothing");
        // interactively the question itself carries the warning
        let r2 = Rig::new();
        repo::make(
            r2.remote.path(),
            "o/d",
            &[(
                "garage-plugin.toml",
                &toml("typist", "permissions = [\"sessions.input\"]"),
            )],
        );
        let mut ui = FakeUi::new(true, &[true]);
        r2.install(&mut ui, "o/d", Opts::default()).unwrap();
        assert!(
            ui.asked[0].contains("DANGEROUS") && ui.asked[0].contains("sessions.input"),
            "{:?}",
            ui.asked
        );
    }

    #[test]
    fn an_invalid_manifest_stops_before_anything_runs_and_names_the_field() {
        let r = Rig::new();
        repo::make(
            r.remote.path(),
            "o/bad",
            &[(
                "garage-plugin.toml",
                &toml(
                    "bad-one",
                    "[[build]]\ncommand = [\"make\"]\n[[actions]]\nid = \"x\"\ntitle = \"X\"\n",
                ),
            )],
        );
        let mut ui = FakeUi::new(true, &[true]);
        let e = r.install(&mut ui, "o/bad", Opts::default()).unwrap_err();
        assert!(e.contains("actions[0]") && e.contains("command"), "{e}");
        assert!(e.contains("Nothing was installed"));
        assert!(r.builder.calls.lock().unwrap().is_empty() && ui.asked.is_empty());
        r.no_leftovers();
        // no manifest at all, and a subdir that is not there
        repo::make(r.remote.path(), "o/empty", &[("README.md", "x")]);
        assert!(
            r.install(&mut ui, "o/empty", YES)
                .unwrap_err()
                .contains("no garage-plugin.toml")
        );
        assert!(
            r.install(&mut ui, "o/bad/missing", YES)
                .unwrap_err()
                .contains("missing")
        );
        // an unsupported platform or api version is a host error, also before consent
        repo::make(
            r.remote.path(),
            "o/future",
            &[(
                "garage-plugin.toml",
                &toml("future", "").replace("api_version = 1", "api_version = 9"),
            )],
        );
        assert!(
            r.install(&mut ui, "o/future", YES)
                .unwrap_err()
                .contains("newer garage")
        );
        r.no_leftovers();
    }

    #[test]
    fn a_listed_release_asset_is_downloaded_verified_and_no_build_runs() {
        let mut r = Rig::new();
        let plat = platform_id();
        let manifest = toml(
            "relbin",
            &format!(
                "[[build]]\ncommand = [\"make\"]\n[release]\nchecksums = \"SHA256SUMS\"\n[release.assets]\n{plat} = \"relbin-asset\"\n"
            ),
        );
        repo::make(
            r.remote.path(),
            "o/relbin",
            &[("garage-plugin.toml", &manifest)],
        );
        let body = b"#!/bin/sh\necho hello\n";
        let base = "https://github.com/o/relbin/releases/download/v1.0.0";
        let sums = format!(
            "{}  relbin-asset\n",
            garage_plugin::manifest::sha256_hex(body)
        );
        r.fetch = FakeFetch::new(&[
            (&format!("{base}/relbin-asset"), body),
            (&format!("{base}/SHA256SUMS"), sums.as_bytes()),
        ]);
        let mut ui = FakeUi::new(true, &[true]);
        r.install(&mut ui, "o/relbin", Opts::default()).unwrap();
        assert!(
            r.builder.calls.lock().unwrap().is_empty(),
            "the listed asset skips [[build]]"
        );
        assert_eq!(
            std::fs::read(r.paths().installed("relbin").join("bin/relbin")).unwrap(),
            body
        );
        assert!(
            ui.out.contains(&format!("{base}/relbin-asset"))
                && ui.out.contains(&format!("{base}/SHA256SUMS")),
            "{}",
            ui.out
        );
        assert!(
            !ui.out.contains("make"),
            "the preview does not promise a build it will not run"
        );
        assert_eq!(r.fetch.calls.lock().unwrap().len(), 2);
    }

    #[test]
    fn a_checksum_mismatch_or_a_failed_build_leaves_no_partial_install() {
        let mut r = Rig::new();
        let plat = platform_id();
        let manifest = toml(
            "badsum",
            &format!("[release]\nchecksums = \"SUMS\"\n[release.assets]\n{plat} = \"a\"\n"),
        );
        repo::make(
            r.remote.path(),
            "o/badsum",
            &[("garage-plugin.toml", &manifest)],
        );
        let base = "https://github.com/o/badsum/releases/download/v1.0.0";
        let sums = format!("{}  a\n", "0".repeat(64));
        r.fetch = FakeFetch::new(&[
            (&format!("{base}/a"), b"payload"),
            (&format!("{base}/SUMS"), sums.as_bytes()),
        ]);
        let mut ui = FakeUi::new(true, &[true]);
        let e = r.install(&mut ui, "o/badsum", Opts::default()).unwrap_err();
        assert!(
            e.contains("checksum mismatch") && e.contains("Nothing was installed"),
            "{e}"
        );
        assert!(r.entry("badsum").is_none() && !r.paths().installed("badsum").exists());
        r.no_leftovers();

        let r = Rig::new();
        repo::make(
            r.remote.path(),
            "o/p",
            &[("garage-plugin.toml", &build_manifest("pp"))],
        );
        *r.builder.fail.lock().unwrap() = true;
        let e = r
            .install(&mut FakeUi::new(true, &[true]), "o/p", Opts::default())
            .unwrap_err();
        assert!(
            e.contains("failed") && e.contains("Nothing was installed"),
            "{e}"
        );
        assert!(r.entry("pp").is_none() && !r.paths().installed("pp").exists());
        r.no_leftovers();
    }

    #[test]
    fn a_symlink_that_leaves_the_repository_is_refused_before_the_manifest_is_read() {
        let r = Rig::new();
        let dir = repo::make(
            r.remote.path(),
            "o/link",
            &[("garage-plugin.toml", &toml("linky", ""))],
        );
        std::os::unix::fs::symlink("/etc", dir.join("etc-link")).unwrap();
        repo::commit(&dir, "evil");
        let mut ui = FakeUi::new(true, &[true]);
        let e = r.install(&mut ui, "o/link", Opts::default()).unwrap_err();
        assert!(e.contains("symlink") && e.contains("etc-link"), "{e}");
        assert!(ui.out.is_empty());
        r.no_leftovers();
    }

    #[test]
    fn refs_resolve_tags_branches_and_full_shas_and_the_ref_is_remembered() {
        let r = Rig::new();
        let dir = repo::make(
            r.remote.path(),
            "o/refs",
            &[("garage-plugin.toml", &toml("refs-one", ""))],
        );
        let first = repo::git(&dir, &["rev-parse", "HEAD"]);
        repo::git(&dir, &["tag", "v1"]);
        repo::write(
            &dir,
            &[(
                "garage-plugin.toml",
                &toml("refs-one", "").replace("1.0.0", "2.0.0"),
            )],
        );
        let second = repo::commit(&dir, "two");
        for (git_ref, want_sha, want_version) in [
            (Some("v1"), &first, "1.0.0"),
            (Some(first.as_str()), &first, "1.0.0"),
            (Some("main"), &second, "2.0.0"),
            (None, &second, "2.0.0"),
        ] {
            let r2 = Rig {
                remote: tempfile::tempdir_in(r.remote.path().parent().unwrap()).unwrap(),
                ..Rig::new()
            };
            // reuse the same remote by pointing a fresh garage dir at it
            let admin = DirectAdmin { paths: r2.paths() };
            let mut ui = FakeUi::new(true, &[true]);
            let mut cx = Cx {
                paths: r2.paths(),
                git: &RealGit,
                fetch: &r2.fetch,
                builder: &r2.builder,
                admin: &admin,
                ui: &mut ui,
                github_base: format!("file://{}", r.remote.path().display()),
                now_ms: 0,
            };
            let done = install(&mut cx, "o/refs", git_ref, &Opts::default()).unwrap();
            assert_eq!(
                (done.version.as_str(), done.commit.as_deref()),
                (want_version, Some(want_sha.as_str())),
                "{git_ref:?}"
            );
            let e = r2.entry("refs-one").unwrap();
            assert!(
                matches!(e.source, Source::Github { git_ref: ref g, .. } if g.as_deref() == git_ref)
            );
        }
        let e = r.install(&mut FakeUi::new(true, &[true]), "o/refs", Opts::default());
        assert!(e.is_ok());
        let mut cx_ui = FakeUi::new(true, &[true]);
        let err = r
            .run(&mut cx_ui, |cx| {
                install(cx, "o/refs", Some("no-such-ref"), &Opts::default())
            })
            .unwrap_err();
        assert!(err.contains("git clone failed"), "{err}");
    }

    #[test]
    fn already_installed_and_contested_cli_names_are_refused_before_asking() {
        let r = Rig::new();
        repo::make(
            r.remote.path(),
            "o/one",
            &[(
                "garage-plugin.toml",
                &toml("one-p", "[[cli]]\nname = \"go\"\ncommand = [\"x\"]\n"),
            )],
        );
        repo::make(
            r.remote.path(),
            "o/two",
            &[(
                "garage-plugin.toml",
                &toml("two-p", "[[cli]]\nname = \"go\"\ncommand = [\"y\"]\n"),
            )],
        );
        r.install(&mut FakeUi::new(true, &[true]), "o/one", Opts::default())
            .unwrap();
        let mut ui = FakeUi::new(true, &[true]);
        let e = r.install(&mut ui, "o/one", Opts::default()).unwrap_err();
        assert!(
            e.contains("already installed") && e.contains("plugin update one-p"),
            "{e}"
        );
        let e = r.install(&mut ui, "o/two", Opts::default()).unwrap_err();
        assert!(
            e.contains("two-p") && e.contains("\"go\"") && e.contains("one-p"),
            "{e}"
        );
        assert!(ui.asked.is_empty() && r.entry("two-p").is_none());
    }

    #[test]
    fn update_applies_code_only_changes_without_a_prompt_keeping_bindings_and_data() {
        let r = Rig::new();
        let dir = repo::make(
            r.remote.path(),
            "o/up",
            &[
                ("garage-plugin.toml", &build_manifest("upd")),
                ("go.sh", "v1"),
            ],
        );
        r.install(&mut FakeUi::new(true, &[true]), "o/up", Opts::default())
            .unwrap();
        DirectAdmin { paths: r.paths() }
            .bind("upd", "upd:go", Some("g"))
            .unwrap();
        std::fs::write(r.paths().data("upd").join("state.json"), "keep").unwrap();
        // up to date first
        let mut ui = FakeUi::new(true, &[]);
        let out = r
            .run(&mut ui, |cx| update(cx, None, &Opts::default()))
            .unwrap();
        assert_eq!(out, [Outcome::UpToDate { id: "upd".into() }]);
        // new version, new description, new code: same commands/permissions/urls
        repo::write(
            &dir,
            &[
                (
                    "garage-plugin.toml",
                    &build_manifest("upd").replace("1.0.0", "1.1.0"),
                ),
                ("go.sh", "v2"),
            ],
        );
        let second = repo::commit(&dir, "v2");
        let builds_before = r.builder.calls.lock().unwrap().len();
        let mut ui = FakeUi::new(false, &[]);
        let out = r
            .run(&mut ui, |cx| update(cx, Some("upd"), &Opts::default()))
            .unwrap();
        assert_eq!(
            out,
            [Outcome::Updated {
                id: "upd".into(),
                from: "1.0.0".into(),
                to: "1.1.0".into(),
                prompted: false
            }]
        );
        assert!(
            ui.asked.is_empty() && ui.out.contains("code only"),
            "{}",
            ui.out
        );
        assert_eq!(
            r.builder.calls.lock().unwrap().len(),
            builds_before + 1,
            "the new code is rebuilt"
        );
        let e = r.entry("upd").unwrap();
        assert!(matches!(&e.source, Source::Github { commit: Some(c), .. } if *c == second));
        assert_eq!(e.bindings["upd:go"], "g");
        assert_eq!(
            std::fs::read_to_string(r.paths().installed("upd").join("go.sh")).unwrap(),
            "v2"
        );
        assert_eq!(
            std::fs::read_to_string(r.paths().data("upd").join("state.json")).unwrap(),
            "keep"
        );
        r.no_leftovers();
    }

    #[test]
    fn a_no_prompt_update_still_refuses_a_cli_name_another_plugin_now_holds() {
        let r = Rig::new();
        let with_cli = |v: &str| {
            toml("upd", "[[cli]]\nname = \"go\"\ncommand = [\"x\"]\n").replace("1.0.0", v)
        };
        let dir = repo::make(
            r.remote.path(),
            "o/up",
            &[("garage-plugin.toml", &with_cli("1.0.0"))],
        );
        // The rival is registered first (linked, approved), as if added while upd was unchanged.
        let rival = r.garage.path().join("rival-src");
        std::fs::create_dir_all(&rival).unwrap();
        std::fs::write(
            rival.join("garage-plugin.toml"),
            toml("rival", "[[cli]]\nname = \"go\"\ncommand = [\"y\"]\n"),
        )
        .unwrap();
        r.install(&mut FakeUi::new(true, &[true]), "o/up", Opts::default())
            .unwrap();
        DirectAdmin { paths: r.paths() }
            .register(&Registration {
                id: "rival".into(),
                source: Source::Link {
                    path: rival.to_string_lossy().into_owned(),
                },
                granted_permissions: vec![],
                manifest_sha256: None,
                installed_at: None,
            })
            .unwrap();
        repo::write(&dir, &[("garage-plugin.toml", &with_cli("1.1.0"))]);
        repo::commit(&dir, "v2");
        let mut ui = FakeUi::new(false, &[]);
        let out = r
            .run(&mut ui, |cx| update(cx, Some("upd"), &Opts::default()))
            .unwrap();
        assert!(
            matches!(&out[0], Outcome::Failed { error, .. } if error.contains("\"go\"")),
            "{out:?}"
        );
        assert!(ui.asked.is_empty());
        assert!(matches!(
            &r.entry("upd").unwrap().source,
            Source::Github {
                commit: Some(_),
                ..
            }
        ));
    }

    #[test]
    fn update_asks_again_when_a_command_permission_or_url_changes_and_declining_keeps_the_old_plugin()
     {
        let r = Rig::new();
        let dir = repo::make(
            r.remote.path(),
            "o/up",
            &[
                ("garage-plugin.toml", &build_manifest("upd")),
                ("go.sh", "v1"),
            ],
        );
        r.install(&mut FakeUi::new(true, &[true]), "o/up", Opts::default())
            .unwrap();
        let old_commit = match r.entry("upd").unwrap().source {
            Source::Github { commit, .. } => commit.unwrap(),
            _ => unreachable!(),
        };
        let changed = build_manifest("upd")
            .replace("go.sh", "evil.sh")
            .replace("ui.widgets", "ui.widgets\", \"sessions.input");
        repo::write(&dir, &[("garage-plugin.toml", &changed), ("go.sh", "v2")]);
        repo::commit(&dir, "sneaky");
        let mut ui = FakeUi::new(true, &[false]);
        let out = r
            .run(&mut ui, |cx| update(cx, None, &Opts::default()))
            .unwrap();
        assert!(
            matches!(&out[0], Outcome::Failed { error, .. } if error.contains("declined")),
            "{out:?}"
        );
        assert!(
            ui.out.contains("+ permission sessions.input")
                && ui.out.contains("+ command action go:")
                && ui.out.contains("- command action go:"),
            "{}",
            ui.out
        );
        assert!(ui.asked[0].contains("DANGEROUS"));
        let e = r.entry("upd").unwrap();
        assert!(
            matches!(&e.source, Source::Github { commit: Some(c), .. } if *c == old_commit),
            "still the approved commit"
        );
        assert_eq!(
            std::fs::read_to_string(r.paths().installed("upd").join("go.sh")).unwrap(),
            "v1"
        );
        assert_eq!(e.granted_permissions.unwrap(), ["ui.widgets"]);
        r.no_leftovers();
        // approving applies and records the new grant
        let mut ui = FakeUi::new(true, &[true]);
        let out = r
            .run(&mut ui, |cx| update(cx, None, &Opts::default()))
            .unwrap();
        assert!(
            matches!(&out[0], Outcome::Updated { prompted: true, .. }),
            "{out:?}"
        );
        assert_eq!(
            r.entry("upd").unwrap().granted_permissions.unwrap(),
            ["ui.widgets", "sessions.input"]
        );
        assert_eq!(
            std::fs::read_to_string(r.paths().installed("upd").join("go.sh")).unwrap(),
            "v2"
        );
    }

    #[test]
    fn a_failed_update_build_leaves_the_old_plugin_working_and_links_are_skipped() {
        let r = Rig::new();
        let dir = repo::make(
            r.remote.path(),
            "o/up",
            &[
                ("garage-plugin.toml", &build_manifest("upd")),
                ("go.sh", "v1"),
            ],
        );
        r.install(&mut FakeUi::new(true, &[true]), "o/up", Opts::default())
            .unwrap();
        crate::plugin::testkit::link_plugin(&r.paths(), r.garage.path(), "devlink", "", true);
        repo::write(&dir, &[("go.sh", "v2")]);
        repo::commit(&dir, "code");
        *r.builder.fail.lock().unwrap() = true;
        let mut ui = FakeUi::new(false, &[]);
        let out = r
            .run(&mut ui, |cx| {
                update(
                    cx,
                    None,
                    &Opts {
                        yes: true,
                        allow_dangerous: false,
                    },
                )
            })
            .unwrap();
        assert!(
            matches!(&out[0], Outcome::Failed { error, .. } if error.contains("failed")),
            "{out:?}"
        );
        assert!(matches!(&out[1], Outcome::Skipped { id, .. } if id == "devlink"));
        assert_eq!(
            std::fs::read_to_string(r.paths().installed("upd").join("go.sh")).unwrap(),
            "v1"
        );
        r.no_leftovers();
        assert!(
            r.run(&mut ui, |cx| update(cx, Some("ghost"), &YES))
                .unwrap_err()
                .contains("no plugin")
        );
    }

    #[test]
    fn link_previews_asks_records_the_grant_and_re_approval_shows_the_added_permission() {
        let r = Rig::new();
        let folder = r.garage.path().join("mine");
        std::fs::create_dir_all(&folder).unwrap();
        std::fs::write(
            folder.join("garage-plugin.toml"),
            toml(
                "mine-p",
                "permissions = [\"ui.widgets\"]\n[[build]]\ncommand = [\"make\"]",
            ),
        )
        .unwrap();
        let mut ui = FakeUi::new(true, &[true]);
        r.run(&mut ui, |cx| link(cx, &folder, &Opts::default()))
            .unwrap();
        assert!(
            ui.out.starts_with("Link: mine-p") && ui.out.contains("link runs no [[build]]"),
            "{}",
            ui.out
        );
        assert!(
            r.builder.calls.lock().unwrap().is_empty(),
            "link never builds"
        );
        let e = r.entry("mine-p").unwrap();
        assert_eq!(
            e.granted_permissions.as_deref(),
            Some(&["ui.widgets".to_owned()][..])
        );
        assert!(matches!(&e.source, Source::Link { path } if path.ends_with("mine")));
        // the manifest grows a permission; the daemon would keep the old one and ask for this command
        std::fs::write(
            folder.join("garage-plugin.toml"),
            toml("mine-p", "permissions = [\"ui.widgets\", \"network\"]"),
        )
        .unwrap();
        let mut ui = FakeUi::new(true, &[true]);
        r.run(&mut ui, |cx| link(cx, &folder, &Opts::default()))
            .unwrap();
        assert!(
            ui.out
                .contains("Changes since the approved version:\n  + permission network"),
            "{}",
            ui.out
        );
        assert_eq!(
            r.entry("mine-p").unwrap().granted_permissions.unwrap(),
            ["ui.widgets", "network"]
        );
        // declining the growth leaves the old grant
        std::fs::write(
            folder.join("garage-plugin.toml"),
            toml(
                "mine-p",
                "permissions = [\"ui.widgets\", \"network\", \"sessions.read\"]",
            ),
        )
        .unwrap();
        let mut ui = FakeUi::new(true, &[false]);
        assert!(
            r.run(&mut ui, |cx| link(cx, &folder, &Opts::default()))
                .is_err()
        );
        assert_eq!(
            r.entry("mine-p").unwrap().granted_permissions.unwrap(),
            ["ui.widgets", "network"]
        );
        assert!(
            r.run(&mut ui, |cx| link(
                cx,
                &r.garage.path().join("nowhere"),
                &YES
            ))
            .unwrap_err()
            .contains("not a folder")
        );
    }

    #[test]
    fn remove_drops_the_entry_keeps_data_unless_purged_never_touches_a_linked_folder_and_refuses_bundled()
     {
        let r = Rig::new();
        repo::make(
            r.remote.path(),
            "o/rm",
            &[("garage-plugin.toml", &build_manifest("rmme"))],
        );
        r.install(&mut FakeUi::new(true, &[true]), "o/rm", Opts::default())
            .unwrap();
        std::fs::write(r.paths().data("rmme").join("d"), "x").unwrap();
        let mut ui = FakeUi::new(true, &[]);
        r.run(&mut ui, |cx| remove(cx, "rmme", false)).unwrap();
        assert!(
            r.entry("rmme").is_none()
                && !r.paths().installed("rmme").exists()
                && !r.paths().token("rmme").exists()
        );
        assert!(r.paths().data("rmme").join("d").exists() && ui.out.contains("--purge"));
        r.install(&mut FakeUi::new(true, &[true]), "o/rm", Opts::default())
            .unwrap();
        r.run(&mut ui, |cx| remove(cx, "rmme", true)).unwrap();
        assert!(!r.paths().data("rmme").exists());

        crate::plugin::testkit::link_plugin(&r.paths(), r.garage.path(), "devlink", "", true);
        let folder = r.garage.path().join("linked/devlink");
        r.run(&mut ui, |cx| remove(cx, "devlink", true)).unwrap();
        assert!(
            folder.join("garage-plugin.toml").exists(),
            "the user's folder is theirs"
        );
        assert!(
            r.run(&mut ui, |cx| remove(cx, "devlink", false))
                .unwrap_err()
                .contains("no plugin")
        );

        PluginsFile::update(&r.paths(), |f| {
            f.plugins.push(garage_daemon::plugins::layout::Entry::new(
                "usage-bar",
                Source::Bundled { path: "/b".into() },
            ));
            (true, ())
        })
        .unwrap();
        assert!(
            r.run(&mut ui, |cx| remove(cx, "usage-bar", true))
                .unwrap_err()
                .contains("plugin disable usage-bar")
        );
        assert!(r.entry("usage-bar").is_some());
    }
}
