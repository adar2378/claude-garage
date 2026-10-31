//! First-party plugins shipped with garage (design section 12): where they
//! live, and first-run seeding of `plugins.json`.
//!
//! A bundled root holds one directory per plugin:
//! `<root>/<id>/garage-plugin.toml` and `<root>/<id>/bin/<id>`. Release builds
//! and the npm package put it next to the `claude-garage` binary
//! (`scripts/stage-plugins.sh`); a checkout uses `<repo>/plugins`.
//!
//! Seeding rules:
//! * The first run registers every bundled plugin; the five default-on ones
//!   are enabled, the rest are off.
//! * `bundledSeeded` in `plugins.json` lists every id ever seeded. An id in
//!   that list is never re-added and its `enabled` is never touched, so a
//!   plugin the user disabled stays disabled and one they removed from the
//!   file stays gone.
//! * A plugin new in a later release is added off (nothing is default-on
//!   after the first run).
//! * Every start refreshes bundled entries: the path follows the package, and
//!   a changed manifest drops the recorded grant so the host records the new
//!   one (first-party code needs no preview).
//! * A user's own entry with the same id (link, GitHub) is left alone.

use std::path::{Path, PathBuf};

use garage_plugin::manifest::sha256_hex;

use super::layout::{Entry, Paths, PluginsFile, Source, MANIFEST_FILE};

/// On at first run. Every other bundled plugin starts off.
pub const DEFAULT_ON: [&str; 5] = ["usage-bar", "closed-chats", "quick-send", "tile-buttons", "pit-pet"];

/// `plugins.json` key listing the bundled ids already seeded.
pub const SEEDED_KEY: &str = "bundledSeeded";

/// Override for the bundled root (tests, unusual packaging).
pub const ROOT_ENV: &str = "GARAGE_BUNDLED_PLUGINS";

/// The bundled root: `$GARAGE_BUNDLED_PLUGINS`, then `plugins/` next to the
/// running binary (and one level up, for `bin/`-style layouts), then the
/// checkout this binary was built from. The first candidate holding at least
/// one manifest wins.
pub fn discover() -> Option<PathBuf> {
    let mut candidates: Vec<PathBuf> = Vec::new();
    if let Some(p) = std::env::var_os(ROOT_ENV).filter(|p| !p.is_empty()) {
        candidates.push(p.into());
    }
    if let Some(dir) = std::env::current_exe().ok().and_then(|e| e.parent().map(Path::to_path_buf)) {
        candidates.push(dir.join("plugins"));
        if let Some(up) = dir.parent() {
            candidates.push(up.join("plugins"));
        }
    }
    candidates.push(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../plugins"));
    candidates.into_iter().find(|c| !plugin_dirs(c).is_empty())
}

/// `(id, dir)` of every `<root>/<id>/garage-plugin.toml`, sorted by id.
pub fn plugin_dirs(root: &Path) -> Vec<(String, PathBuf)> {
    let mut out: Vec<(String, PathBuf)> = std::fs::read_dir(root)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.path().join(MANIFEST_FILE).is_file())
        .filter_map(|e| Some((e.file_name().into_string().ok()?, e.path())))
        .collect();
    out.sort();
    out
}

/// What a seeding pass changed (for a log line and for tests).
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Seeded {
    pub added: Vec<String>,
    pub refreshed: Vec<String>,
}

/// Register and refresh the bundled plugins under `root`. Writes
/// `plugins.json` only when something changed.
pub fn seed(paths: &Paths, root: &Path) -> Result<Seeded, String> {
    let found = plugin_dirs(root);
    if found.is_empty() {
        return Ok(Seeded::default());
    }
    PluginsFile::update(paths, |file| {
        let first_run = !file.extra.contains_key(SEEDED_KEY);
        let mut seeded: Vec<String> = file
            .extra
            .get(SEEDED_KEY)
            .and_then(|v| v.as_array())
            .map(|a| a.iter().filter_map(|v| v.as_str().map(str::to_owned)).collect())
            .unwrap_or_default();
        let mut out = Seeded::default();
        for (id, dir) in &found {
            let path = dir.to_string_lossy().into_owned();
            let sha = std::fs::read(dir.join(MANIFEST_FILE)).ok().map(|b| sha256_hex(&b));
            match file.plugins.iter_mut().find(|e| &e.id == id) {
                Some(entry) => {
                    // Only bundled entries follow the package.
                    let Source::Bundled { path: old } = &mut entry.source else { continue };
                    let mut changed = false;
                    if *old != path {
                        *old = path;
                        changed = true;
                    }
                    if sha.is_some() && entry.manifest_sha256 != sha && entry.granted_permissions.is_some() {
                        // New manifest: the host re-grants and records the sha.
                        entry.granted_permissions = None;
                        changed = true;
                    }
                    if changed {
                        out.refreshed.push(id.clone());
                    }
                }
                None if !seeded.contains(id) => {
                    let mut entry = Entry::new(id, Source::Bundled { path });
                    entry.enabled = first_run && DEFAULT_ON.contains(&id.as_str());
                    file.plugins.push(entry);
                    out.added.push(id.clone());
                }
                None => {} // seeded before, then removed by hand: stays gone
            }
            if !seeded.contains(id) {
                seeded.push(id.clone());
            }
        }
        let dirty = !out.added.is_empty() || !out.refreshed.is_empty() || file.extra.get(SEEDED_KEY).is_none_or(|v| v.as_array().map(Vec::len) != Some(seeded.len()));
        file.extra.insert(SEEDED_KEY.into(), seeded.into());
        (dirty, out)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A bundled root with the given plugin ids, each a valid manifest.
    fn root(ids: &[&str]) -> tempfile::TempDir {
        let d = tempfile::tempdir().unwrap();
        for id in ids {
            add(d.path(), id, "1.0.0");
        }
        d
    }

    fn add(root: &Path, id: &str, version: &str) {
        std::fs::create_dir_all(root.join(id)).unwrap();
        std::fs::write(
            root.join(id).join(MANIFEST_FILE),
            format!("id = \"{id}\"\nname = \"{id}\"\nversion = \"{version}\"\napi_version = 1\nplatforms = [\"macos\", \"linux\", \"windows\"]\npermissions = []\n"),
        )
        .unwrap();
    }

    fn enabled(paths: &Paths) -> Vec<(String, bool)> {
        PluginsFile::read(paths).unwrap().plugins.into_iter().map(|e| (e.id, e.enabled)).collect()
    }

    const ALL: [&str; 9] =
        ["usage-bar", "closed-chats", "quick-send", "tile-buttons", "pit-pet", "jump-palette", "turn-markers", "cli-extras", "prune-empty-projects"];

    #[test]
    fn every_plugin_crate_in_the_checkout_ships_a_valid_manifest_named_after_its_directory() {
        let repo = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../plugins");
        let found = plugin_dirs(&repo);
        assert!(!found.is_empty(), "plugins/ holds at least the hello sample");
        for (id, dir) in found {
            let raw = std::fs::read_to_string(dir.join(MANIFEST_FILE)).unwrap();
            let m = garage_plugin::parse(&raw).unwrap_or_else(|e| panic!("{id}: {e:?}"));
            assert_eq!(m.id, id, "the directory name is the plugin id");
            let problems = garage_plugin::check_host(&m);
            assert!(problems.is_empty(), "{id}: {problems:?}");
        }
    }

    #[test]
    fn first_run_registers_everything_with_the_documented_defaults() {
        let (garage, bundle) = (tempfile::tempdir().unwrap(), root(&ALL));
        let paths = Paths::new(garage.path());
        let out = seed(&paths, bundle.path()).unwrap();
        assert_eq!(out.added.len(), 9);
        for (id, on) in enabled(&paths) {
            assert_eq!(on, DEFAULT_ON.contains(&id.as_str()), "{id}");
        }
        let file = PluginsFile::read(&paths).unwrap();
        assert!(file.plugins.iter().all(Entry::is_first_party));
        assert_eq!(file.extra[SEEDED_KEY].as_array().unwrap().len(), 9);
    }

    #[test]
    fn a_second_pass_changes_nothing_and_never_re_enables_what_the_user_disabled() {
        let (garage, bundle) = (tempfile::tempdir().unwrap(), root(&ALL));
        let paths = Paths::new(garage.path());
        seed(&paths, bundle.path()).unwrap();
        // user turns pit-pet off and jump-palette on, and deletes quick-send from the file
        PluginsFile::update(&paths, |f| {
            f.plugins.retain(|e| e.id != "quick-send");
            for e in &mut f.plugins {
                match e.id.as_str() {
                    "pit-pet" => e.enabled = false,
                    "jump-palette" => e.enabled = true,
                    _ => {}
                }
            }
            (true, ())
        })
        .unwrap();
        let before = std::fs::read_to_string(paths.registry_file()).unwrap();
        assert_eq!(seed(&paths, bundle.path()).unwrap(), Seeded::default());
        assert_eq!(std::fs::read_to_string(paths.registry_file()).unwrap(), before, "an unchanged pass does not rewrite the file");
        let now = enabled(&paths);
        assert!(now.contains(&("pit-pet".into(), false)) && now.contains(&("jump-palette".into(), true)));
        assert!(!now.iter().any(|(id, _)| id == "quick-send"), "a plugin the user removed stays gone");
    }

    #[test]
    fn a_plugin_new_in_a_later_release_arrives_off_even_if_it_is_a_default_name() {
        let (garage, bundle) = (tempfile::tempdir().unwrap(), root(&["usage-bar"]));
        let paths = Paths::new(garage.path());
        seed(&paths, bundle.path()).unwrap();
        add(bundle.path(), "pit-pet", "1.0.0");
        add(bundle.path(), "extra", "1.0.0");
        assert_eq!(seed(&paths, bundle.path()).unwrap().added, ["extra", "pit-pet"]);
        let now = enabled(&paths);
        assert!(now.contains(&("usage-bar".into(), true)));
        assert!(now.contains(&("pit-pet".into(), false)) && now.contains(&("extra".into(), false)));
    }

    #[test]
    fn an_upgrade_follows_the_package_and_a_changed_manifest_drops_the_old_grant() {
        let (garage, old) = (tempfile::tempdir().unwrap(), root(&["usage-bar"]));
        let paths = Paths::new(garage.path());
        seed(&paths, old.path()).unwrap();
        // the host recorded the grant and sha at its first load
        PluginsFile::update(&paths, |f| {
            f.plugins[0].granted_permissions = Some(vec![]);
            f.plugins[0].manifest_sha256 = Some(sha256_hex(&std::fs::read(old.path().join("usage-bar").join(MANIFEST_FILE)).unwrap()));
            (true, ())
        })
        .unwrap();
        assert_eq!(seed(&paths, old.path()).unwrap(), Seeded::default());

        let new = root(&[]);
        add(new.path(), "usage-bar", "2.0.0");
        assert_eq!(seed(&paths, new.path()).unwrap().refreshed, ["usage-bar"]);
        let e = &PluginsFile::read(&paths).unwrap().plugins[0];
        assert_eq!(e.source, Source::Bundled { path: new.path().join("usage-bar").to_string_lossy().into_owned() });
        assert_eq!(e.granted_permissions, None, "first-party grant is re-recorded by the host");
        assert!(e.enabled);
    }

    #[test]
    fn a_users_own_entry_with_a_bundled_id_is_left_alone_and_no_bundle_means_no_file() {
        let (garage, bundle) = (tempfile::tempdir().unwrap(), root(&["usage-bar"]));
        let paths = Paths::new(garage.path());
        PluginsFile::update(&paths, |f| {
            f.plugins.push(Entry::new("usage-bar", Source::Link { path: "/mine".into() }));
            (true, ())
        })
        .unwrap();
        seed(&paths, bundle.path()).unwrap();
        assert_eq!(PluginsFile::read(&paths).unwrap().plugins[0].source, Source::Link { path: "/mine".into() });

        let (garage, empty) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
        let paths = Paths::new(garage.path());
        seed(&paths, empty.path()).unwrap();
        assert!(!paths.registry_file().exists(), "nothing bundled: nothing is created");
    }
}

/// The bundled path end to end with a real plugin binary: seed, load,
/// enable, run its action over the real HTTP API, see its widget.
#[cfg(all(test, unix))]
mod end_to_end {
    use super::*;
    use crate::plugins::rig::{wait_for, Rig};
    use serde_json::json;
    use std::sync::OnceLock;

    /// Builds `plugins/hello` once and returns the executable cargo reports.
    fn hello_binary() -> &'static Path {
        static BIN: OnceLock<PathBuf> = OnceLock::new();
        BIN.get_or_init(|| {
            let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".into());
            let out = std::process::Command::new(cargo)
                .args(["build", "-p", "garage-plugin-hello", "--message-format=json"])
                .current_dir(env!("CARGO_MANIFEST_DIR"))
                .output()
                .expect("cargo runs");
            assert!(out.status.success(), "building the hello plugin failed:\n{}", String::from_utf8_lossy(&out.stderr));
            String::from_utf8_lossy(&out.stdout)
                .lines()
                .filter_map(|l| serde_json::from_str::<serde_json::Value>(l).ok())
                .find_map(|v| v.get("executable").and_then(|e| e.as_str()).map(PathBuf::from))
                .expect("cargo reported the hello executable")
        })
    }

    /// `<root>/hello/{garage-plugin.toml,bin/hello}`: what packaging stages.
    fn staged_root() -> tempfile::TempDir {
        let root = tempfile::tempdir().unwrap();
        let dir = root.path().join("hello");
        std::fs::create_dir_all(dir.join("bin")).unwrap();
        let repo = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../plugins/hello");
        std::fs::copy(repo.join(MANIFEST_FILE), dir.join(MANIFEST_FILE)).unwrap();
        std::fs::copy(hello_binary(), dir.join("bin/hello")).unwrap();
        root
    }

    #[tokio::test]
    async fn a_seeded_bundled_plugin_runs_its_action_and_pushes_its_widget() {
        let bin = hello_binary(); // build before the daemon rig, off the async path's clock
        assert!(bin.is_file());
        let r = Rig::serving().await;
        let root = staged_root();

        let seeded = seed(&r.paths(), root.path()).unwrap();
        assert_eq!(seeded.added, ["hello"]);
        r.st.plugins.reload().unwrap();
        let hello = r.st.plugins.plugin("hello").unwrap();
        assert!(hello.entry.is_first_party() && !hello.entry.enabled && !hello.active(), "not a default: seeded off");
        assert_eq!(hello.entry.granted_permissions, Some(vec!["ui.widgets".to_owned()]), "first-party grant needs no preview");

        r.st.plugins.set_enabled("hello", true).unwrap();
        assert!(r.st.plugins.active("hello").is_some());

        let body = json!({"focus": {"workspace": "ws", "session": {"id": "garage/ws/a"}}}).to_string();
        let (status, _, _) = r.call_ui("POST", "/api/plugins/hello/actions/greet/run", Some(&body)).await;
        assert_eq!(status, axum::http::StatusCode::ACCEPTED);

        wait_for(|| !r.st.plugins.widgets().widgets("hello").is_empty()).await;
        let w = r.st.plugins.widgets().widgets("hello");
        let shown = serde_json::to_string(&w["greeting"].scopes).unwrap();
        assert!(shown.contains("hello, garage/ws/a"), "{shown}");
    }
}
