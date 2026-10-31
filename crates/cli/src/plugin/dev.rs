//! `claude-garage plugin dev [path]` (p27): link a plugin folder, watch it,
//! rebuild and reload on change, and tail its log in the terminal.
//!
//! The pieces are small and pure so they can be tested without a daemon, a
//! clock or a terminal: [`snapshot`]/[`classify`] decide *what changed*,
//! [`sync`] decides *what to do about it*, [`LogTail`] reads new log text, and
//! [`Watcher::step`] ties them together one poll at a time. The real command
//! only adds a sleep and Ctrl+C around `step`.

use std::collections::BTreeMap;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

use garage_daemon::plugins::layout::{MANIFEST_FILE, PluginsFile, Source};

use super::fetch::run_builds;
use super::install::{self, Cx, Opts, read_manifest};

/// Folders never watched: build output and dependencies would make every
/// build trigger the next one.
const SKIP_DIRS: &[&str] = &["target", "node_modules", "dist", "build", "__pycache__"];
const MAX_FILES: usize = 5000;
const MAX_DEPTH: usize = 10;

/// Path to (modified time in ns, length) of every watched file.
pub type Snapshot = BTreeMap<PathBuf, (u128, u64)>;

fn skip_file(name: &str) -> bool {
    name.ends_with('~')
        || name.ends_with(".log")
        || name.ends_with(".swp")
        || name.ends_with(".swx")
        || name.ends_with(".tmp")
        || name == ".DS_Store"
        || name == "4913" // vim's write probe
}

pub fn snapshot(root: &Path) -> Snapshot {
    fn walk(dir: &Path, depth: usize, out: &mut Snapshot) {
        let Ok(rd) = std::fs::read_dir(dir) else {
            return;
        };
        for e in rd.flatten() {
            if out.len() >= MAX_FILES {
                return;
            }
            let name = e.file_name().to_string_lossy().into_owned();
            let Ok(ft) = e.file_type() else { continue };
            if ft.is_dir() {
                if depth < MAX_DEPTH
                    && !name.starts_with('.')
                    && !SKIP_DIRS.contains(&name.as_str())
                {
                    walk(&e.path(), depth + 1, out);
                }
            } else if ft.is_file() && !skip_file(&name) {
                if let Ok(md) = e.metadata() {
                    let mtime = md
                        .modified()
                        .ok()
                        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                        .map(|d| d.as_nanos())
                        .unwrap_or(0);
                    out.insert(e.path(), (mtime, md.len()));
                }
            }
        }
    }
    let mut out = Snapshot::new();
    walk(root, 0, &mut out);
    out
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Change {
    /// First sync when `plugin dev` starts.
    Initial,
    /// Only `garage-plugin.toml` changed: no rebuild.
    Manifest,
    /// Something other than the manifest changed (maybe the manifest too).
    Code,
}

/// What differs between two snapshots, or `None` when nothing does.
pub fn classify(old: &Snapshot, new: &Snapshot) -> Option<Change> {
    let manifest = |p: &PathBuf| p.file_name().is_some_and(|n| n == MANIFEST_FILE);
    let differs = |p: &PathBuf| old.get(p) != new.get(p);
    let changed: Vec<&PathBuf> = old
        .keys()
        .chain(new.keys())
        .filter(|p| differs(p))
        .collect();
    if changed.is_empty() {
        None
    } else if changed.iter().all(|p| manifest(p)) {
        Some(Change::Manifest)
    } else {
        Some(Change::Code)
    }
}

/// Bring the daemon up to date with the folder: link it if it is not linked
/// (or now wants permissions nobody approved), run the `[[build]]` commands
/// unless only the manifest changed, then ask the daemon to reload.
///
/// Returns a one-line summary; build output goes to the plugin's log, which
/// the watcher is already tailing.
pub fn sync(cx: &mut Cx, dir: &Path, change: Change, opts: &Opts) -> Result<String, String> {
    let abs =
        std::path::absolute(dir).map_err(|e| format!("cannot resolve {}: {e}", dir.display()))?;
    let (m, _) = read_manifest(&abs)?;

    let entry = PluginsFile::read(&cx.paths)?
        .plugins
        .into_iter()
        .find(|e| e.id == m.id);
    let wants = m.permission_names();
    let needs_link = match &entry {
        None => true,
        Some(e) => match (&e.source, &e.granted_permissions) {
            (Source::Link { path }, Some(granted)) => {
                Path::new(path) != abs || wants.iter().any(|p| !granted.contains(p))
            }
            // Not a link of this folder (bundled, GitHub) or never approved:
            // `link` says exactly what is wrong or asks.
            _ => true,
        },
    };
    if needs_link {
        install::link(cx, &abs, opts)?;
    }
    if change != Change::Manifest {
        run_builds(
            cx.builder,
            &m,
            &abs,
            &cx.paths.data(&m.id),
            &cx.paths.log(&m.id),
        )?;
    }
    cx.admin.reload()?;
    Ok(format!(
        "Reloaded {} {} ({}).",
        m.id,
        m.version,
        cx.admin.via()
    ))
}

/// Reads what a log file gained since the last call.
pub struct LogTail {
    path: PathBuf,
    pos: u64,
}

impl LogTail {
    /// Start at the end; the last `show` lines are returned so the terminal
    /// opens with some context.
    pub fn start(path: &Path, show: usize) -> (Self, String) {
        let text = std::fs::read_to_string(path).unwrap_or_default();
        let lines: Vec<&str> = text.lines().collect();
        let tail = lines[lines.len().saturating_sub(show)..].join("\n");
        (
            Self {
                path: path.to_owned(),
                pos: text.len() as u64,
            },
            tail,
        )
    }

    pub fn poll(&mut self) -> String {
        let Ok(mut f) = std::fs::File::open(&self.path) else {
            return String::new();
        };
        let len = f.metadata().map(|m| m.len()).unwrap_or(0);
        if len < self.pos {
            self.pos = 0; // rotated or truncated
        }
        if len == self.pos {
            return String::new();
        }
        let _ = f.seek(SeekFrom::Start(self.pos));
        let mut buf = Vec::new();
        let _ = f.read_to_end(&mut buf);
        self.pos += buf.len() as u64;
        String::from_utf8_lossy(&buf).into_owned()
    }
}

/// Watches one plugin folder. A change must be seen in two polls in a row
/// before it is acted on, so a save that writes several files, or a build
/// that is still touching sources, produces one reload.
pub struct Watcher {
    dir: PathBuf,
    last: Snapshot,
    pending: Option<Snapshot>,
    tail: LogTail,
    opts: Opts,
}

impl Watcher {
    /// Does the first sync (the plugin must at least parse) and opens the log.
    pub fn start(cx: &mut Cx, dir: &Path, opts: Opts) -> Result<Self, String> {
        let abs = std::path::absolute(dir).map_err(|e| e.to_string())?;
        let (m, _) = read_manifest(&abs)?;
        let (tail, recent) = LogTail::start(&cx.paths.log(&m.id), 5);
        let last = snapshot(&abs);
        // A failed first build must not stop the loop: fixing it is what dev is for.
        match sync(cx, &abs, Change::Initial, &opts) {
            Ok(msg) => cx.ui.say(&msg),
            Err(e) => {
                // Declined or refused linking: there is nothing to watch.
                let linked = PluginsFile::read(&cx.paths)?
                    .plugins
                    .iter()
                    .any(|p| p.id == m.id);
                if !linked {
                    return Err(e);
                }
                cx.ui.say(&format!("error: {e}"));
            }
        }
        if !recent.is_empty() {
            cx.ui
                .say(&format!("--- recent log ---\n{recent}\n--- live ---"));
        }
        Ok(Self {
            dir: abs,
            last,
            pending: None,
            tail,
            opts,
        })
    }

    /// One poll: print new log text, and act on a settled change.
    pub fn step(&mut self, cx: &mut Cx) {
        let log = self.tail.poll();
        if !log.is_empty() {
            cx.ui.say(log.trim_end());
        }
        let now = snapshot(&self.dir);
        if now == self.last {
            self.pending = None;
            return;
        }
        if self.pending.as_ref() != Some(&now) {
            self.pending = Some(now);
            return;
        }
        self.pending = None;
        let change = classify(&self.last, &now).unwrap_or(Change::Code);
        self.last = now;
        cx.ui.say(&match change {
            Change::Manifest => "garage-plugin.toml changed".to_owned(),
            _ => "files changed".to_owned(),
        });
        match sync(cx, &self.dir, change, &self.opts) {
            Ok(msg) => cx.ui.say(&msg),
            // Includes manifest problems: the previous version keeps running.
            Err(e) => cx.ui.say(&format!(
                "error: {e}\n(the last good version keeps running)"
            )),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plugin::admin::{Admin, DirectAdmin};
    use crate::plugin::fetch::CurlFetch;
    use crate::plugin::source::RealGit;
    use crate::plugin::testkit::{FakeUi, RecordingBuilder, toml};
    use garage_daemon::plugins::admin::Registration;
    use garage_daemon::plugins::layout::Paths;
    use std::sync::Mutex;

    /// Wraps the direct admin and counts reloads.
    struct CountingAdmin {
        inner: DirectAdmin,
        reloads: Mutex<u32>,
    }

    impl Admin for CountingAdmin {
        fn via(&self) -> &'static str {
            "test"
        }
        fn register(&self, r: &Registration) -> Result<(), String> {
            self.inner.register(r)
        }
        fn set_enabled(&self, id: &str, e: bool) -> Result<(), String> {
            self.inner.set_enabled(id, e)
        }
        fn remove(&self, id: &str) -> Result<(), String> {
            self.inner.remove(id)
        }
        fn bind(&self, id: &str, a: &str, k: Option<&str>) -> Result<(), String> {
            self.inner.bind(id, a, k)
        }
        fn reload(&self) -> Result<(), String> {
            *self.reloads.lock().unwrap() += 1;
            Ok(())
        }
    }

    struct Rig {
        _d: tempfile::TempDir,
        plugin: PathBuf,
        paths: Paths,
        admin: CountingAdmin,
        builder: RecordingBuilder,
        ui: FakeUi,
    }

    fn rig(extra: &str) -> Rig {
        let d = tempfile::tempdir().unwrap();
        let paths = Paths::new(&d.path().join("garage"));
        let plugin = d.path().join("devplug");
        std::fs::create_dir_all(plugin.join("src")).unwrap();
        std::fs::write(plugin.join(MANIFEST_FILE), toml("devplug", extra)).unwrap();
        std::fs::write(plugin.join("src/main.sh"), "echo 1").unwrap();
        Rig {
            admin: CountingAdmin {
                inner: DirectAdmin {
                    paths: paths.clone(),
                },
                reloads: Mutex::new(0),
            },
            _d: d,
            plugin,
            paths,
            builder: RecordingBuilder::default(),
            ui: FakeUi::new(true, &[true, true, true]),
        }
    }

    impl Rig {
        fn cx(&mut self) -> Cx<'_> {
            Cx {
                paths: self.paths.clone(),
                git: &RealGit,
                fetch: &CurlFetch,
                builder: &self.builder,
                admin: &self.admin,
                ui: &mut self.ui,
                github_base: String::new(),
                now_ms: 0,
            }
        }
        fn builds(&self) -> usize {
            self.builder.calls.lock().unwrap().len()
        }
        fn reloads(&self) -> u32 {
            *self.admin.reloads.lock().unwrap()
        }
    }

    const BUILD: &str = "[[build]]\ncommand = [\"make\"]\n";

    #[test]
    fn the_snapshot_ignores_build_output_dependencies_hidden_and_editor_files() {
        let r = rig("");
        for f in [
            "target/release/x",
            "node_modules/a/b.js",
            "dist/main.js",
            ".git/HEAD",
            "src/.hidden/x",
            "src/main.sh~",
            "run.log",
            ".main.sh.swp",
        ] {
            let p = r.plugin.join(f);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, "x").unwrap();
        }
        let names: Vec<String> = snapshot(&r.plugin)
            .keys()
            .map(|p| {
                p.strip_prefix(&r.plugin)
                    .unwrap()
                    .to_string_lossy()
                    .into_owned()
            })
            .collect();
        assert_eq!(names, vec!["garage-plugin.toml", "src/main.sh"]);
    }

    #[test]
    fn classify_tells_a_manifest_only_edit_from_a_code_edit() {
        let r = rig("");
        let before = snapshot(&r.plugin);
        assert_eq!(classify(&before, &snapshot(&r.plugin)), None);
        std::fs::write(r.plugin.join(MANIFEST_FILE), toml("devplug", "# edited")).unwrap();
        let manifest_only = snapshot(&r.plugin);
        assert_eq!(classify(&before, &manifest_only), Some(Change::Manifest));
        std::fs::write(r.plugin.join("src/main.sh"), "echo 22").unwrap();
        assert_eq!(classify(&before, &snapshot(&r.plugin)), Some(Change::Code));
        std::fs::write(r.plugin.join("new.sh"), "x").unwrap();
        assert_eq!(
            classify(&manifest_only, &snapshot(&r.plugin)),
            Some(Change::Code)
        );
        std::fs::remove_file(r.plugin.join("new.sh")).unwrap();
        std::fs::remove_file(r.plugin.join("src/main.sh")).unwrap();
        assert_eq!(
            classify(&manifest_only, &snapshot(&r.plugin)),
            Some(Change::Code),
            "deletes count"
        );
    }

    #[test]
    fn the_first_sync_links_builds_and_reloads_and_later_ones_only_build_and_reload() {
        let mut r = rig(BUILD);
        let plugin = r.plugin.clone();
        let msg = sync(&mut r.cx(), &plugin, Change::Initial, &Opts::default()).unwrap();
        assert!(msg.starts_with("Reloaded devplug 1.0.0"), "{msg}");
        let entry = PluginsFile::read(&r.paths).unwrap().plugins.remove(0);
        assert!(matches!(entry.source, Source::Link { .. }));
        assert_eq!((r.builds(), r.reloads()), (1, 1));
        assert_eq!(r.ui.asked.len(), 1, "the consent prompt was shown once");
        sync(&mut r.cx(), &plugin, Change::Code, &Opts::default()).unwrap();
        assert_eq!((r.builds(), r.reloads()), (2, 2));
        assert_eq!(r.ui.asked.len(), 1, "no second prompt");
    }

    #[test]
    fn a_manifest_only_change_reloads_without_a_rebuild() {
        let mut r = rig(BUILD);
        let plugin = r.plugin.clone();
        sync(&mut r.cx(), &plugin, Change::Initial, &Opts::default()).unwrap();
        sync(&mut r.cx(), &plugin, Change::Manifest, &Opts::default()).unwrap();
        assert_eq!((r.builds(), r.reloads()), (1, 2));
    }

    #[test]
    fn a_failed_build_stops_before_the_reload() {
        let mut r = rig(BUILD);
        let plugin = r.plugin.clone();
        sync(&mut r.cx(), &plugin, Change::Initial, &Opts::default()).unwrap();
        *r.builder.fail.lock().unwrap() = true;
        let e = sync(&mut r.cx(), &plugin, Change::Code, &Opts::default()).unwrap_err();
        assert!(e.contains("failed"), "{e}");
        assert_eq!(r.reloads(), 1, "the broken build is not loaded");
    }

    #[test]
    fn new_permissions_ask_again_and_a_refusal_changes_nothing() {
        let mut r = rig("");
        let plugin = r.plugin.clone();
        sync(&mut r.cx(), &plugin, Change::Initial, &Opts::default()).unwrap();
        std::fs::write(
            plugin.join(MANIFEST_FILE),
            toml("devplug", "permissions = [\"sessions.read\"]"),
        )
        .unwrap();
        r.ui = FakeUi::new(true, &[false]);
        let e = sync(&mut r.cx(), &plugin, Change::Manifest, &Opts::default()).unwrap_err();
        assert!(!e.is_empty());
        let granted = PluginsFile::read(&r.paths).unwrap().plugins[0]
            .granted_permissions
            .clone()
            .unwrap();
        assert!(granted.is_empty(), "nothing was granted: {granted:?}");
        assert_eq!(r.reloads(), 1);
        // Approving does grant it.
        r.ui = FakeUi::new(true, &[true]);
        sync(&mut r.cx(), &plugin, Change::Manifest, &Opts::default()).unwrap();
        let granted = PluginsFile::read(&r.paths).unwrap().plugins[0]
            .granted_permissions
            .clone()
            .unwrap();
        assert_eq!(granted, vec!["sessions.read".to_owned()]);
    }

    #[test]
    fn a_broken_manifest_is_reported_with_the_field_and_not_loaded() {
        let mut r = rig("");
        let plugin = r.plugin.clone();
        sync(&mut r.cx(), &plugin, Change::Initial, &Opts::default()).unwrap();
        std::fs::write(
            plugin.join(MANIFEST_FILE),
            toml("devplug", "[[actions]]\nid = \"a\"\n"),
        )
        .unwrap();
        let e = sync(&mut r.cx(), &plugin, Change::Manifest, &Opts::default()).unwrap_err();
        assert!(e.contains("actions"), "{e}");
        assert_eq!(r.reloads(), 1);
    }

    #[test]
    fn the_log_tail_returns_only_new_text_and_survives_truncation() {
        let d = tempfile::tempdir().unwrap();
        let log = d.path().join("p.log");
        std::fs::write(&log, "a\nb\nc\n").unwrap();
        let (mut t, recent) = LogTail::start(&log, 2);
        assert_eq!(recent, "b\nc");
        assert_eq!(t.poll(), "");
        std::fs::write(&log, "a\nb\nc\nd\n").unwrap();
        assert_eq!(t.poll(), "d\n");
        std::fs::write(&log, "x\n").unwrap();
        assert_eq!(t.poll(), "x\n", "a rotated log is read from the top");
        let (mut missing, _) = LogTail::start(&d.path().join("none.log"), 5);
        assert_eq!(missing.poll(), "");
    }

    #[test]
    fn the_watcher_waits_for_a_settled_change_then_reloads_once() {
        let mut r = rig(BUILD);
        let plugin = r.plugin.clone();
        let mut w = Watcher::start(&mut r.cx(), &plugin, Opts::default()).unwrap();
        assert_eq!((r.builds(), r.reloads()), (1, 1));
        w.step(&mut r.cx());
        assert_eq!(r.reloads(), 1, "no change, no reload");
        std::fs::write(plugin.join("src/main.sh"), "echo changed").unwrap();
        w.step(&mut r.cx());
        assert_eq!(r.reloads(), 1, "first sighting only waits");
        // Still being written: the debounce restarts.
        std::fs::write(plugin.join("src/main.sh"), "echo changed more").unwrap();
        w.step(&mut r.cx());
        assert_eq!(r.reloads(), 1);
        w.step(&mut r.cx());
        assert_eq!((r.builds(), r.reloads()), (2, 2));
        w.step(&mut r.cx());
        assert_eq!(r.reloads(), 2, "settled and handled once");
        assert!(r.ui.out.contains("files changed"));
    }

    #[test]
    fn declining_the_first_link_ends_dev_instead_of_watching_nothing() {
        let mut r = rig("");
        r.ui = FakeUi::new(true, &[false]);
        let plugin = r.plugin.clone();
        let e = Watcher::start(&mut r.cx(), &plugin, Opts::default())
            .err()
            .unwrap();
        assert!(e.contains("declined"), "{e}");
        assert!(PluginsFile::read(&r.paths).unwrap().plugins.is_empty());
    }

    #[test]
    fn the_watcher_prints_new_log_lines_and_keeps_going_after_a_failed_build() {
        let mut r = rig(BUILD);
        let plugin = r.plugin.clone();
        let mut w = Watcher::start(&mut r.cx(), &plugin, Opts::default()).unwrap();
        let log = r.paths.log("devplug");
        std::fs::create_dir_all(log.parent().unwrap()).unwrap();
        std::fs::write(&log, "plugin said hi\n").unwrap();
        *r.builder.fail.lock().unwrap() = true;
        std::fs::write(plugin.join("src/main.sh"), "oops").unwrap();
        w.step(&mut r.cx());
        w.step(&mut r.cx());
        assert!(r.ui.out.contains("plugin said hi"));
        assert!(
            r.ui.out.contains("the last good version keeps running"),
            "{}",
            r.ui.out
        );
        *r.builder.fail.lock().unwrap() = false;
        std::fs::write(plugin.join("src/main.sh"), "fixed it").unwrap();
        w.step(&mut r.cx());
        w.step(&mut r.cx());
        assert_eq!(r.reloads(), 2, "the fix reloads");
    }
}
