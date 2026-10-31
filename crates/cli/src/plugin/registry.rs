//! What is installed, read straight from `plugins.json` and the manifests so
//! `plugin list` and `[[cli]]` dispatch work with the daemon down. This is a
//! read-only view; every write goes through [`super::admin`].

use std::path::PathBuf;

use garage_daemon::plugins::layout::{
    Entry, MANIFEST_FILE, Paths, PluginsFile, Source, github_plugin_dir,
};
use garage_plugin::manifest::{Manifest, sha256_hex};
use garage_plugin::{check_cli_conflicts, check_host, parse};

/// One registry entry and what loading its manifest produced.
#[derive(Debug, Clone)]
pub struct Loaded {
    pub entry: Entry,
    pub dir: PathBuf,
    pub manifest: Option<Manifest>,
    pub manifest_sha: Option<String>,
    /// Why the manifest is not usable (parse, host or id problems).
    pub problems: Vec<String>,
    /// Refused because of another plugin (cli names, fallback).
    pub refused: Option<String>,
}

impl Loaded {
    /// Enabled, approved, loaded and not refused: what the daemon would run.
    pub fn usable(&self) -> bool {
        self.entry.enabled
            && self.entry.granted_permissions.is_some()
            && self.manifest.is_some()
            && self.problems.is_empty()
            && self.refused.is_none()
    }

    pub fn state(&self) -> &'static str {
        if !self.entry.enabled {
            "disabled"
        } else if self.manifest.is_none() || !self.problems.is_empty() {
            "error"
        } else if self.entry.granted_permissions.is_none() {
            "not approved"
        } else if self.refused.is_some() {
            "refused"
        } else {
            "active"
        }
    }

    pub fn source_label(&self) -> String {
        match &self.entry.source {
            Source::Github {
                repo,
                subdir,
                commit,
                ..
            } => {
                let sub = subdir
                    .as_deref()
                    .map(|s| format!("/{s}"))
                    .unwrap_or_default();
                let at = commit
                    .as_deref()
                    .map(|c| format!("@{}", &c[..c.len().min(7)]))
                    .unwrap_or_default();
                format!("github {repo}{sub}{at}")
            }
            Source::Link { path } => format!("dev {path}"),
            Source::Bundled { .. } => "bundled".into(),
        }
    }
}

pub fn manifest_dir(paths: &Paths, e: &Entry) -> PathBuf {
    match &e.source {
        Source::Github { subdir, .. } => github_plugin_dir(paths, &e.id, subdir.as_deref()),
        Source::Link { path } | Source::Bundled { path } => PathBuf::from(path),
    }
}

/// Load one entry's manifest the way the daemon does.
pub fn load_entry(paths: &Paths, entry: Entry) -> Loaded {
    let dir = manifest_dir(paths, &entry);
    let mut l = Loaded {
        entry,
        dir: dir.clone(),
        manifest: None,
        manifest_sha: None,
        problems: vec![],
        refused: None,
    };
    let raw = match std::fs::read_to_string(dir.join(MANIFEST_FILE)) {
        Ok(r) => r,
        Err(e) => {
            l.problems.push(format!(
                "cannot read {}: {e}",
                dir.join(MANIFEST_FILE).display()
            ));
            return l;
        }
    };
    l.manifest_sha = Some(sha256_hex(raw.as_bytes()));
    match parse(&raw) {
        Ok(m) => {
            l.problems
                .extend(check_host(&m).iter().map(ToString::to_string));
            if m.id != l.entry.id {
                l.problems.push(format!(
                    "id: manifest says {:?} but the registry entry is {:?}",
                    m.id, l.entry.id
                ));
            }
            if let Some(granted) = &l.entry.granted_permissions
                && m.permission_names().iter().any(|p| !granted.contains(p))
            {
                l.problems.push(format!(
                    "plugin {} wants new permissions - run `claude-garage plugin link` to approve",
                    l.entry.id
                ));
            }
            l.manifest = Some(m);
        }
        Err(ps) => l.problems.extend(ps.iter().map(ToString::to_string)),
    }
    l
}

/// Everything in `plugins.json`, with the cross-plugin `[[cli]]` rules applied
/// in install order (a first-party plugin wins a contested fallback).
pub fn load_all(paths: &Paths) -> Result<Vec<Loaded>, String> {
    let file = PluginsFile::read(paths)?;
    let mut all: Vec<Loaded> = file
        .plugins
        .into_iter()
        .map(|e| load_entry(paths, e))
        .collect();
    let conflicts = {
        let candidates: Vec<(&Manifest, bool)> = all
            .iter()
            .filter(|l| {
                l.entry.enabled && l.entry.granted_permissions.is_some() && l.problems.is_empty()
            })
            .filter_map(|l| l.manifest.as_ref().map(|m| (m, l.entry.is_first_party())))
            .collect();
        check_cli_conflicts(&candidates)
    };
    for c in conflicts {
        if let Some(l) = all.iter_mut().find(|l| l.entry.id == c.plugin) {
            l.refused = Some(c.message);
        }
    }
    Ok(all)
}

#[cfg(test)]
mod tests {
    use super::*;
    use garage_daemon::plugins::admin::{Registration, register};

    pub fn toml(id: &str, extra: &str) -> String {
        format!(
            "id = \"{id}\"\nname = \"{id}\"\nversion = \"1.0.0\"\napi_version = 1\nplatforms = [\"macos\",\"linux\",\"windows\"]\n{extra}\n"
        )
    }

    fn link(paths: &Paths, root: &std::path::Path, id: &str, extra: &str, granted: &[&str]) {
        let dir = root.join(id);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(MANIFEST_FILE), toml(id, extra)).unwrap();
        register(
            paths,
            &Registration {
                id: id.into(),
                source: Source::Link {
                    path: dir.to_string_lossy().into_owned(),
                },
                granted_permissions: granted.iter().map(|s| s.to_string()).collect(),
                manifest_sha256: None,
                installed_at: None,
            },
        )
        .unwrap();
    }

    #[test]
    fn states_cover_active_disabled_unapproved_broken_and_permission_growth() {
        let d = tempfile::tempdir().unwrap();
        let paths = Paths::new(d.path());
        link(&paths, d.path(), "fine", "", &[]);
        link(&paths, d.path(), "grew", "permissions = [\"network\"]", &[]);
        link(&paths, d.path(), "off", "", &[]);
        garage_daemon::plugins::admin::set_enabled(&paths, "off", false).unwrap();
        link(&paths, d.path(), "broken", "", &[]);
        std::fs::write(d.path().join("broken").join(MANIFEST_FILE), "id = 1").unwrap();
        PluginsFile::update(&paths, |f| {
            f.plugins.push(Entry::new(
                "unapproved",
                Source::Link {
                    path: d.path().join("fine").to_string_lossy().into_owned(),
                },
            ));
            (true, ())
        })
        .unwrap();
        let by: std::collections::HashMap<_, _> = load_all(&paths)
            .unwrap()
            .into_iter()
            .map(|l| (l.entry.id.clone(), l))
            .collect();
        assert_eq!(by["fine"].state(), "active");
        assert_eq!(by["off"].state(), "disabled");
        assert_eq!(by["broken"].state(), "error");
        assert!(
            by["broken"].problems[0].contains("id"),
            "{:?}",
            by["broken"].problems
        );
        assert_eq!(by["grew"].state(), "error");
        assert!(by["grew"].problems[0].contains("wants new permissions"));
        assert_eq!(
            by["unapproved"].state(),
            "error",
            "its manifest id is `fine`, not `unapproved`"
        );
        assert!(by["fine"].source_label().starts_with("dev "));
    }

    #[test]
    fn contested_cli_names_and_fallbacks_are_refused_like_the_daemon_does() {
        let d = tempfile::tempdir().unwrap();
        let paths = Paths::new(d.path());
        let cli = |name: &str, fb: bool| {
            format!("[[cli]]\nname = \"{name}\"\ncommand = [\"x\"]\nfallback = {fb}\n")
        };
        link(&paths, d.path(), "first", &cli("go", true), &[]);
        link(&paths, d.path(), "second", &cli("go", true), &[]);
        let all = load_all(&paths).unwrap();
        assert!(all[0].usable());
        assert!(!all[1].usable());
        assert_eq!(all[1].state(), "refused");
        assert!(all[1].refused.as_ref().unwrap().contains("first"));
    }

    #[test]
    fn a_github_entry_reads_its_manifest_from_the_subdir_of_the_checkout() {
        let d = tempfile::tempdir().unwrap();
        let paths = Paths::new(d.path());
        let sub = paths.installed("deep-one").join("plugins/x");
        std::fs::create_dir_all(&sub).unwrap();
        std::fs::write(sub.join(MANIFEST_FILE), toml("deep-one", "")).unwrap();
        let mut e = Entry::new(
            "deep-one",
            Source::Github {
                repo: "o/r".into(),
                subdir: Some("plugins/x".into()),
                git_ref: None,
                commit: Some("0123456789abcdef".into()),
            },
        );
        e.granted_permissions = Some(vec![]);
        let l = load_entry(&paths, e);
        assert!(l.usable(), "{:?}", l.problems);
        assert_eq!(l.source_label(), "github o/r/plugins/x@0123456");
    }
}
