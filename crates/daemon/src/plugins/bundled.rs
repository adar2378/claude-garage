//! First-party plugins shipped with garage (design section 12): where they
//! live, which ones are trusted, and first-run seeding of `plugins.json`.
//!
//! A bundled root holds one directory per plugin:
//! `<root>/<id>/garage-plugin.toml` and `<root>/<id>/bin/<id>`. Release builds
//! and the npm package put it next to the `claude-garage` binary
//! (`scripts/stage-plugins.sh`); a debug build also finds the checkout's
//! `plugins/`.
//!
//! Trust: the daemon is built with an allowlist (`build.rs`) of every
//! first-party plugin's id and the SHA-256 of its manifest. A bundled entry is
//! auto-granted only when both match; anything else (a changed manifest, an
//! unknown id, a dev root) is treated like a linked plugin: no grant, stays
//! disabled until approved. Known limit: the plugin binaries are not hashed.
//! The p27 / cargo-dist packaging must hash them too.
//!
//! Seeding rules:
//! * The first run registers every bundled plugin; the five default-on ones
//!   are enabled (when trusted), the rest are off.
//! * `bundledSeeded` in `plugins.json` lists every id ever seeded, even when a
//!   user entry held the id at the time. An id in that list is never re-added
//!   and its `enabled` is never touched, so a plugin the user disabled stays
//!   disabled and one they removed from the file stays gone.
//! * A plugin new in a later release is added off (nothing is default-on
//!   after the first run).
//! * Every start refreshes bundled entries: the path follows the package, a
//!   changed manifest drops the recorded grant (the host records the new one
//!   only if the allowlist matches), and a bundled entry whose plugin no
//!   longer ships is removed (its data directory stays).
//! * A user's own entry with the same id (link, GitHub) is left alone.

use std::path::{Path, PathBuf};

use garage_plugin::manifest::sha256_hex;

use super::layout::{Entry, MANIFEST_FILE, Paths, PluginsFile, Source};

include!(concat!(env!("OUT_DIR"), "/bundled_allowlist.rs"));

/// On at first run. Every other bundled plugin starts off.
pub const DEFAULT_ON: [&str; 5] = [
    "usage-bar",
    "closed-chats",
    "quick-send",
    "tile-buttons",
    "pit-pet",
];

/// `plugins.json` key listing the bundled ids already seeded.
pub const SEEDED_KEY: &str = "bundledSeeded";

/// Dev override for the bundled root (debug builds only).
#[cfg(debug_assertions)]
pub const ROOT_ENV: &str = "GARAGE_BUNDLED_PLUGINS";

/// Test-only additions to the allowlist (id, manifest sha).
#[cfg(test)]
static TEST_TRUST: std::sync::Mutex<Vec<(String, String)>> = std::sync::Mutex::new(Vec::new());

/// Trust a manifest in tests (a content hash, so sharing it across tests is harmless).
#[cfg(test)]
pub fn trust_for_tests(id: &str, manifest: &[u8]) {
    TEST_TRUST
        .lock()
        .unwrap()
        .push((id.to_owned(), sha256_hex(manifest)));
}

/// Is `id` with a manifest hashing to `sha` a first-party plugin this build knows?
pub fn is_trusted(id: &str, sha: &str) -> bool {
    #[cfg(test)]
    if TEST_TRUST
        .lock()
        .unwrap()
        .iter()
        .any(|(i, s)| i == id && s == sha)
    {
        return true;
    }
    ALLOWLIST.iter().any(|(i, s)| *i == id && *s == sha)
}

/// Where to look, in order. Release: `plugins/` next to the canonicalized
/// binary, nothing else. Debug builds add `$GARAGE_BUNDLED_PLUGINS` and the
/// checkout this binary was built from.
fn candidates(exe: Option<PathBuf>, dev_env: Option<PathBuf>, dev: bool) -> Vec<PathBuf> {
    let mut out = Vec::new();
    if dev {
        out.extend(dev_env.filter(|p| !p.as_os_str().is_empty()));
    }
    if let Some(dir) = exe.and_then(|e| e.parent().map(Path::to_path_buf)) {
        out.push(dir.join("plugins"));
    }
    if dev {
        out.push(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../plugins"));
    }
    out
}

/// The bundled root: the first candidate holding at least one manifest.
pub fn discover() -> Option<PathBuf> {
    let exe = std::env::current_exe()
        .ok()
        .and_then(|e| e.canonicalize().ok());
    #[cfg(debug_assertions)]
    let (dev, env) = (true, std::env::var_os(ROOT_ENV).map(PathBuf::from));
    #[cfg(not(debug_assertions))]
    let (dev, env) = (false, None);
    candidates(exe, env, dev)
        .into_iter()
        .find(|c| !plugin_dirs(c).is_empty())
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
    pub pruned: Vec<String>,
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
            .map(|a| {
                a.iter()
                    .filter_map(|v| v.as_str().map(str::to_owned))
                    .collect()
            })
            .unwrap_or_default();
        let mut out = Seeded::default();
        for (id, dir) in &found {
            let path = dir.to_string_lossy().into_owned();
            let sha = std::fs::read(dir.join(MANIFEST_FILE))
                .ok()
                .map(|b| sha256_hex(&b));
            let trusted = sha.as_deref().is_some_and(|s| is_trusted(id, s));
            match file.plugins.iter_mut().find(|e| &e.id == id) {
                // A user's own entry (link, GitHub) is left alone.
                Some(Entry {
                    source: Source::Link { .. } | Source::Github { .. },
                    ..
                }) => {}
                Some(entry) => {
                    // Only bundled entries follow the package.
                    let Source::Bundled { path: old } = &mut entry.source else {
                        unreachable!()
                    };
                    let mut changed = false;
                    if *old != path {
                        *old = path;
                        changed = true;
                    }
                    if sha.is_some() && entry.granted_permissions.is_some() {
                        // An unknown manifest never keeps a grant; a trusted
                        // one that changed gets the host to record the new one.
                        let moved = entry.manifest_sha256.is_some() && entry.manifest_sha256 != sha;
                        if !trusted || moved {
                            entry.granted_permissions = None;
                            changed = true;
                        }
                    }
                    if changed {
                        out.refreshed.push(id.clone());
                    }
                }
                None if !seeded.contains(id) => {
                    let mut entry = Entry::new(id, Source::Bundled { path });
                    entry.enabled = first_run && trusted && DEFAULT_ON.contains(&id.as_str());
                    file.plugins.push(entry);
                    out.added.push(id.clone());
                }
                None => {} // seeded before, then removed by hand: stays gone
            }
            if !seeded.contains(id) {
                seeded.push(id.clone());
            }
        }
        // A bundled plugin this release no longer ships: drop the entry, keep its data.
        file.plugins.retain(|e| {
            let gone = e.is_first_party() && !found.iter().any(|(id, _)| *id == e.id);
            if gone {
                out.pruned.push(e.id.clone());
            }
            !gone
        });
        let dirty = !out.added.is_empty()
            || !out.refreshed.is_empty()
            || !out.pruned.is_empty()
            || file
                .extra
                .get(SEEDED_KEY)
                .is_none_or(|v| v.as_array().map(Vec::len) != Some(seeded.len()));
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

    /// Write a manifest and put it on the (test) allowlist.
    fn add(root: &Path, id: &str, version: &str) {
        let bytes = write_manifest(root, id, version);
        trust_for_tests(id, bytes.as_bytes());
    }

    fn write_manifest(root: &Path, id: &str, version: &str) -> String {
        std::fs::create_dir_all(root.join(id)).unwrap();
        let toml = format!(
            "id = \"{id}\"\nname = \"{id}\"\nversion = \"{version}\"\napi_version = 1\nplatforms = [\"macos\", \"linux\", \"windows\"]\npermissions = []\n"
        );
        std::fs::write(root.join(id).join(MANIFEST_FILE), &toml).unwrap();
        toml
    }

    fn enabled(paths: &Paths) -> Vec<(String, bool)> {
        PluginsFile::read(paths)
            .unwrap()
            .plugins
            .into_iter()
            .map(|e| (e.id, e.enabled))
            .collect()
    }

    const ALL: [&str; 9] = [
        "usage-bar",
        "closed-chats",
        "quick-send",
        "tile-buttons",
        "pit-pet",
        "jump-palette",
        "turn-markers",
        "cli-extras",
        "prune-empty-projects",
    ];

    #[test]
    fn every_plugin_crate_in_the_checkout_ships_a_valid_manifest_named_after_its_directory() {
        let repo = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../plugins");
        let found = plugin_dirs(&repo);
        assert!(
            !found.is_empty(),
            "plugins/ holds at least the hello sample"
        );
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
        assert_eq!(
            std::fs::read_to_string(paths.registry_file()).unwrap(),
            before,
            "an unchanged pass does not rewrite the file"
        );
        let now = enabled(&paths);
        assert!(
            now.contains(&("pit-pet".into(), false))
                && now.contains(&("jump-palette".into(), true))
        );
        assert!(
            !now.iter().any(|(id, _)| id == "quick-send"),
            "a plugin the user removed stays gone"
        );
    }

    #[test]
    fn a_plugin_new_in_a_later_release_arrives_off_even_if_it_is_a_default_name() {
        let (garage, bundle) = (tempfile::tempdir().unwrap(), root(&["usage-bar"]));
        let paths = Paths::new(garage.path());
        seed(&paths, bundle.path()).unwrap();
        add(bundle.path(), "pit-pet", "1.0.0");
        add(bundle.path(), "extra", "1.0.0");
        assert_eq!(
            seed(&paths, bundle.path()).unwrap().added,
            ["extra", "pit-pet"]
        );
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
            f.plugins[0].manifest_sha256 = Some(sha256_hex(
                &std::fs::read(old.path().join("usage-bar").join(MANIFEST_FILE)).unwrap(),
            ));
            (true, ())
        })
        .unwrap();
        assert_eq!(seed(&paths, old.path()).unwrap(), Seeded::default());

        let new = root(&[]);
        add(new.path(), "usage-bar", "2.0.0");
        assert_eq!(seed(&paths, new.path()).unwrap().refreshed, ["usage-bar"]);
        let e = &PluginsFile::read(&paths).unwrap().plugins[0];
        assert_eq!(
            e.source,
            Source::Bundled {
                path: new.path().join("usage-bar").to_string_lossy().into_owned()
            }
        );
        assert_eq!(
            e.granted_permissions, None,
            "first-party grant is re-recorded by the host"
        );
        assert!(e.enabled);
    }

    #[test]
    fn an_unlisted_or_edited_manifest_is_never_trusted_or_granted() {
        let (garage, bundle) = (tempfile::tempdir().unwrap(), root(&["usage-bar"]));
        let paths = Paths::new(garage.path());
        // not on the allowlist: registered, but off even though usage-bar is a default
        write_manifest(bundle.path(), "pit-pet", "6.6.6"); // versions no other test trusts
        write_manifest(bundle.path(), "unknown", "6.6.6");
        seed(&paths, bundle.path()).unwrap();
        let now = enabled(&paths);
        assert!(now.contains(&("usage-bar".into(), true)));
        assert!(
            now.contains(&("pit-pet".into(), false)) && now.contains(&("unknown".into(), false))
        );

        // a trusted plugin whose manifest later changes loses its grant, and stays without it
        PluginsFile::update(&paths, |f| {
            let e = f.plugins.iter_mut().find(|e| e.id == "usage-bar").unwrap();
            e.granted_permissions = Some(vec![]);
            (true, ())
        })
        .unwrap();
        write_manifest(bundle.path(), "usage-bar", "9.9.9"); // same id, different hash, not trusted
        assert_eq!(
            seed(&paths, bundle.path()).unwrap().refreshed,
            ["usage-bar"]
        );
        let e = PluginsFile::read(&paths)
            .unwrap()
            .plugins
            .into_iter()
            .find(|e| e.id == "usage-bar")
            .unwrap();
        assert_eq!(e.granted_permissions, None);
        assert_eq!(
            seed(&paths, bundle.path()).unwrap(),
            Seeded::default(),
            "and a second pass leaves the file alone"
        );
    }

    #[test]
    fn the_compiled_allowlist_never_lists_the_hello_fixture() {
        assert!(ALLOWLIST.iter().all(|(id, _)| *id != "hello"));
        let hello = std::fs::read(
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../plugins/hello")
                .join(MANIFEST_FILE),
        )
        .unwrap();
        assert!(!is_trusted("hello", &sha256_hex(&hello)));
    }

    /// Every first-party plugin in the checkout is on the compiled allowlist
    /// with the hash of the manifest as it is now, so a bundled plugin is
    /// granted automatically, and the three widget plugins have the defaults
    /// the proposal names (usage bar and pit pet on, turn markers off).
    #[test]
    fn every_first_party_manifest_in_the_checkout_is_trusted_as_it_is() {
        let repo = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../plugins");
        let mut ids = Vec::new();
        for (id, dir) in plugin_dirs(&repo)
            .into_iter()
            .filter(|(id, _)| id != "hello")
        {
            let bytes = std::fs::read(dir.join(MANIFEST_FILE)).unwrap();
            assert!(
                is_trusted(&id, &sha256_hex(&bytes)),
                "{id} is not on the allowlist as written"
            );
            ids.push(id);
        }
        for id in ["usage-bar", "pit-pet", "turn-markers"] {
            assert!(ids.iter().any(|i| i == id), "{id} is missing from plugins/");
        }
        assert!(DEFAULT_ON.contains(&"usage-bar") && DEFAULT_ON.contains(&"pit-pet"));
        assert!(!DEFAULT_ON.contains(&"turn-markers"));
    }

    #[test]
    fn a_release_build_looks_only_next_to_the_binary() {
        let exe = Some(PathBuf::from("/opt/garage/bin/claude-garage"));
        let env = Some(PathBuf::from("/evil"));
        assert_eq!(
            candidates(exe.clone(), env.clone(), false),
            [PathBuf::from("/opt/garage/bin/plugins")]
        );
        let dev = candidates(exe, env, true);
        assert_eq!(dev[0], PathBuf::from("/evil"));
        assert_eq!(dev.len(), 3);
    }

    #[test]
    fn a_plugin_that_stops_shipping_is_pruned_but_a_user_entry_holding_its_id_is_not_and_is_remembered_as_seeded()
     {
        let (garage, bundle) = (
            tempfile::tempdir().unwrap(),
            root(&["usage-bar", "pit-pet", "extra"]),
        );
        let paths = Paths::new(garage.path());
        PluginsFile::update(&paths, |f| {
            f.plugins.push(Entry::new(
                "extra",
                Source::Link {
                    path: "/mine".into(),
                },
            ));
            (true, ())
        })
        .unwrap();
        seed(&paths, bundle.path()).unwrap();
        let file = PluginsFile::read(&paths).unwrap();
        assert!(
            file.extra[SEEDED_KEY]
                .as_array()
                .unwrap()
                .iter()
                .any(|v| v == "extra"),
            "recorded although the user's entry held it"
        );

        std::fs::remove_dir_all(bundle.path().join("pit-pet")).unwrap();
        assert_eq!(seed(&paths, bundle.path()).unwrap().pruned, ["pit-pet"]);
        let ids: Vec<_> = PluginsFile::read(&paths)
            .unwrap()
            .plugins
            .into_iter()
            .map(|e| e.id)
            .collect();
        assert_eq!(ids, ["extra", "usage-bar"]);

        // the user drops their link: "extra" was seeded, so the bundle does not re-add it
        PluginsFile::update(&paths, |f| {
            f.plugins.retain(|e| e.id != "extra");
            (true, ())
        })
        .unwrap();
        assert_eq!(seed(&paths, bundle.path()).unwrap(), Seeded::default());
    }

    #[test]
    fn a_users_own_entry_with_a_bundled_id_is_left_alone_and_no_bundle_means_no_file() {
        let (garage, bundle) = (tempfile::tempdir().unwrap(), root(&["usage-bar"]));
        let paths = Paths::new(garage.path());
        PluginsFile::update(&paths, |f| {
            f.plugins.push(Entry::new(
                "usage-bar",
                Source::Link {
                    path: "/mine".into(),
                },
            ));
            (true, ())
        })
        .unwrap();
        seed(&paths, bundle.path()).unwrap();
        assert_eq!(
            PluginsFile::read(&paths).unwrap().plugins[0].source,
            Source::Link {
                path: "/mine".into()
            }
        );

        let (garage, empty) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
        let paths = Paths::new(garage.path());
        seed(&paths, empty.path()).unwrap();
        assert!(
            !paths.registry_file().exists(),
            "nothing bundled: nothing is created"
        );
    }
}

/// The bundled path end to end with a real plugin binary: seed, load,
/// enable, run its action over the real HTTP API, see its widget.
#[cfg(all(test, unix))]
mod end_to_end {
    use super::*;
    use crate::plugins::rig::{Rig, wait_for};
    use serde_json::json;

    fn hello_binary() -> PathBuf {
        crate::plugins::first_party::plugin_binary("garage-plugin-hello")
    }

    /// `<root>/hello/{garage-plugin.toml,bin/hello}`: what packaging stages.
    fn staged_root() -> tempfile::TempDir {
        let root = tempfile::tempdir().unwrap();
        let dir = root.path().join("hello");
        std::fs::create_dir_all(dir.join("bin")).unwrap();
        let repo = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../plugins/hello");
        std::fs::copy(repo.join(MANIFEST_FILE), dir.join(MANIFEST_FILE)).unwrap();
        // hello is a fixture, not on the build's allowlist: trust it for this test only
        trust_for_tests("hello", &std::fs::read(repo.join(MANIFEST_FILE)).unwrap());
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
        assert!(
            hello.entry.is_first_party() && !hello.entry.enabled && !hello.active(),
            "not a default: seeded off"
        );
        assert_eq!(
            hello.entry.granted_permissions,
            Some(vec!["ui.widgets".to_owned()]),
            "first-party grant needs no preview"
        );

        r.st.plugins.set_enabled("hello", true).unwrap();
        assert!(r.st.plugins.active("hello").is_some());

        let body =
            json!({"focus": {"workspace": "ws", "session": {"id": "garage/ws/a"}}}).to_string();
        let (status, _, _) = r
            .call_ui("POST", "/api/plugins/hello/actions/greet/run", Some(&body))
            .await;
        assert_eq!(status, axum::http::StatusCode::ACCEPTED);

        wait_for(|| !r.st.plugins.widgets().widgets("hello").is_empty()).await;
        let w = r.st.plugins.widgets().widgets("hello");
        let shown = serde_json::to_string(&w["greeting"].scopes).unwrap();
        assert!(shown.contains("hello, garage/ws/a"), "{shown}");
    }
}
