//! Registry edits made by the CLI lane (`plugin link`, `install`, `enable`,
//! `disable`, `remove`, `bind`). Each is a single [`PluginsFile::update`], so
//! it holds the registry lock for its whole read-modify-write. The daemon's
//! routes call these and then reload; a CLI that finds the daemon down calls
//! them directly. Approval is recorded here, at the moment the user says yes:
//! `granted_permissions` is what `plugin link` / `install` showed.

use super::layout::{ensure_token, Entry, Paths, PluginsFile, Source};

/// What the CLI hands over after the user confirmed a preview.
#[derive(Debug, Clone, PartialEq)]
pub struct Registration {
    pub id: String,
    pub source: Source,
    pub granted_permissions: Vec<String>,
    pub manifest_sha256: Option<String>,
    pub installed_at: Option<i64>,
}

/// Add or replace an entry for `registration.id`, keeping the user's key
/// bindings and `enabled` choice, and create its data directory and token.
/// Bundled entries are seeded by garage itself and cannot be registered here.
pub fn register(paths: &Paths, r: &Registration) -> Result<(), String> {
    if matches!(r.source, Source::Bundled { .. }) {
        return Err("bundled plugins are seeded by garage, not registered by hand".into());
    }
    if !garage_plugin::validate::valid_plugin_id(&r.id) {
        return Err(format!("{:?} is not a valid plugin id", r.id));
    }
    PluginsFile::update(paths, |file| {
        if file.plugins.iter().any(|e| e.id == r.id && e.is_first_party()) {
            return (false, Err(format!("{} is a bundled plugin; use enable/disable", r.id)));
        }
        let keep = file.plugins.iter().find(|e| e.id == r.id).cloned();
        file.plugins.retain(|e| e.id != r.id);
        let mut entry = Entry::new(&r.id, r.source.clone());
        entry.granted_permissions = Some(r.granted_permissions.clone());
        entry.manifest_sha256 = r.manifest_sha256.clone();
        entry.installed_at = r.installed_at.or_else(|| keep.as_ref().and_then(|k| k.installed_at));
        if let Some(k) = keep {
            entry.bindings = k.bindings;
            entry.enabled = k.enabled;
            entry.extra = k.extra;
        }
        file.plugins.push(entry);
        (true, Ok(()))
    })??;
    std::fs::create_dir_all(paths.data(&r.id)).map_err(|e| format!("cannot create {}: {e}", paths.data(&r.id).display()))?;
    ensure_token(paths, &r.id).map_err(|e| format!("cannot create the plugin token: {e}"))?;
    Ok(())
}

pub fn set_enabled(paths: &Paths, id: &str, enabled: bool) -> Result<(), String> {
    PluginsFile::update(paths, |file| match file.plugins.iter_mut().find(|e| e.id == id) {
        Some(e) => {
            let changed = e.enabled != enabled;
            e.enabled = enabled;
            (changed, Ok(()))
        }
        None => (false, Err(format!("no plugin {id:?}"))),
    })?
}

/// Drop the registry entry. Bundled plugins can only be disabled. Returns the
/// removed entry so the caller can delete its files.
pub fn remove_entry(paths: &Paths, id: &str) -> Result<Entry, String> {
    PluginsFile::update(paths, |file| match file.plugins.iter().position(|e| e.id == id) {
        None => (false, Err(format!("no plugin {id:?}"))),
        Some(i) if file.plugins[i].is_first_party() => (false, Err(format!("{id} is a bundled plugin and cannot be removed; use `plugin disable {id}`"))),
        Some(i) => (true, Ok(file.plugins.remove(i))),
    })?
}

/// Bind `key` to the full action id `<plugin>:<action>` (None clears it).
pub fn set_binding(paths: &Paths, id: &str, action: &str, key: Option<&str>) -> Result<(), String> {
    PluginsFile::update(paths, |file| match file.plugins.iter_mut().find(|e| e.id == id) {
        Some(e) => {
            match key {
                Some(k) => e.bindings.insert(action.to_owned(), k.to_owned()),
                None => e.bindings.remove(action),
            };
            (true, Ok(()))
        }
        None => (false, Err(format!("no plugin {id:?}"))),
    })?
}

/// Delete a removed plugin's files: always `installed/<id>` and the token;
/// `data/<id>` and the logs only with `purge`.
pub fn delete_files(paths: &Paths, id: &str, purge: bool) -> std::io::Result<()> {
    let ignore_missing = |r: std::io::Result<()>| match r {
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e),
        _ => Ok(()),
    };
    ignore_missing(std::fs::remove_dir_all(paths.installed(id)))?;
    ignore_missing(std::fs::remove_file(paths.token(id)))?;
    if purge {
        ignore_missing(std::fs::remove_dir_all(paths.data(id)))?;
        ignore_missing(std::fs::remove_file(paths.log(id)))?;
        let mut rotated = paths.log(id).into_os_string();
        rotated.push(".1");
        ignore_missing(std::fs::remove_file(rotated))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn paths() -> (tempfile::TempDir, Paths) {
        let d = tempfile::tempdir().unwrap();
        let p = Paths::new(d.path());
        (d, p)
    }

    fn reg(id: &str, perms: &[&str]) -> Registration {
        Registration {
            id: id.into(),
            source: Source::Link { path: "/x".into() },
            granted_permissions: perms.iter().map(|s| s.to_string()).collect(),
            manifest_sha256: Some("ab".into()),
            installed_at: Some(5),
        }
    }

    #[test]
    fn register_records_the_grant_and_keeps_bindings_and_enabled_on_re_approval() {
        let (_d, p) = paths();
        register(&p, &reg("dev-one", &["ui.widgets"])).unwrap();
        set_binding(&p, "dev-one", "dev-one:go", Some("x")).unwrap();
        set_enabled(&p, "dev-one", false).unwrap();
        register(&p, &reg("dev-one", &["ui.widgets", "core.actions"])).unwrap();
        let e = PluginsFile::read(&p).unwrap().plugins.remove(0);
        assert_eq!(e.granted_permissions.unwrap(), ["ui.widgets", "core.actions"]);
        assert_eq!(e.bindings["dev-one:go"], "x");
        assert!(!e.enabled, "re-approving does not switch a disabled plugin back on");
        assert!(p.data("dev-one").is_dir());
        assert_eq!(std::fs::read_to_string(p.token("dev-one")).unwrap().trim().len(), 64);
    }

    #[test]
    fn bundled_plugins_cannot_be_registered_over_or_removed_but_can_be_disabled() {
        let (_d, p) = paths();
        PluginsFile::update(&p, |f| {
            f.plugins.push(Entry::new("usage-bar", Source::Bundled { path: "/b".into() }));
            (true, ())
        })
        .unwrap();
        assert!(register(&p, &reg("usage-bar", &[])).unwrap_err().contains("bundled"));
        assert!(remove_entry(&p, "usage-bar").unwrap_err().contains("plugin disable"));
        set_enabled(&p, "usage-bar", false).unwrap();
        assert!(!PluginsFile::read(&p).unwrap().plugins[0].enabled);
        let mut r = reg("other", &[]);
        r.source = Source::Bundled { path: "/b".into() };
        assert!(register(&p, &r).is_err());
    }

    #[test]
    fn remove_drops_the_entry_and_purge_decides_what_files_go() {
        let (_d, p) = paths();
        register(&p, &reg("gone", &[])).unwrap();
        std::fs::create_dir_all(p.installed("gone")).unwrap();
        std::fs::create_dir_all(p.log("gone").parent().unwrap()).unwrap();
        std::fs::write(p.log("gone"), "x").unwrap();
        std::fs::write(p.data("gone").join("keep.txt"), "x").unwrap();
        assert_eq!(remove_entry(&p, "gone").unwrap().id, "gone");
        assert!(remove_entry(&p, "gone").unwrap_err().contains("no plugin"));
        delete_files(&p, "gone", false).unwrap();
        assert!(!p.installed("gone").exists() && !p.token("gone").exists());
        assert!(p.data("gone").join("keep.txt").exists() && p.log("gone").exists(), "kept without --purge");
        delete_files(&p, "gone", true).unwrap();
        assert!(!p.data("gone").exists() && !p.log("gone").exists());
        delete_files(&p, "gone", true).unwrap();
    }

    #[test]
    fn bad_ids_and_unknown_plugins_are_errors_and_bindings_can_be_cleared() {
        let (_d, p) = paths();
        assert!(register(&p, &reg("Bad Id", &[])).is_err());
        assert!(set_enabled(&p, "nope", true).is_err());
        register(&p, &reg("ok-one", &[])).unwrap();
        set_binding(&p, "ok-one", "ok-one:a", Some("g")).unwrap();
        set_binding(&p, "ok-one", "ok-one:a", None).unwrap();
        assert!(PluginsFile::read(&p).unwrap().plugins[0].bindings.is_empty());
    }

    #[test]
    fn a_github_subdir_is_joined_but_never_allowed_to_leave_the_checkout() {
        let (_d, p) = paths();
        assert_eq!(super::super::layout::github_plugin_dir(&p, "a", Some("tools/x")), p.installed("a").join("tools/x"));
        assert_eq!(super::super::layout::github_plugin_dir(&p, "a", Some("../../etc")), p.installed("a"));
        assert_eq!(super::super::layout::github_plugin_dir(&p, "a", Some("/etc")), p.installed("a"));
        assert_eq!(super::super::layout::github_plugin_dir(&p, "a", None), p.installed("a"));
    }
}
