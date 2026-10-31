//! Where plugins live on disk (design section 1) and the `plugins.json`
//! registry. Everything is under `<GARAGE_DIR>/plugins`; nothing is created
//! until something is registered or run.
//!
//! ```text
//! plugins.json        registry of installed/linked plugins
//! installed/<id>/     checkout of a GitHub plugin
//! data/<id>/          GARAGE_PLUGIN_STATE_DIR, survives update
//! logs/<id>.log       stdout+stderr of every run (1 MiB, 2 files)
//! tokens/<id>         per-plugin API token, mode 0600
//! ```

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

pub const FILE_VERSION: u32 = 1;
pub const MANIFEST_FILE: &str = "garage-plugin.toml";

#[derive(Debug, Clone)]
pub struct Paths {
    root: PathBuf,
}

impl Paths {
    pub fn new(garage_dir: &Path) -> Self {
        Self { root: garage_dir.join("plugins") }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn registry_file(&self) -> PathBuf {
        self.root.join("plugins.json")
    }

    pub fn installed(&self, id: &str) -> PathBuf {
        self.root.join("installed").join(id)
    }

    pub fn data(&self, id: &str) -> PathBuf {
        self.root.join("data").join(id)
    }

    pub fn log(&self, id: &str) -> PathBuf {
        self.root.join("logs").join(format!("{id}.log"))
    }

    pub fn token(&self, id: &str) -> PathBuf {
        self.root.join("tokens").join(id)
    }

    /// Scratch space for oversized context files.
    pub fn tmp(&self) -> PathBuf {
        self.root.join("tmp")
    }
}

/// Where a plugin came from.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum Source {
    Github {
        repo: String,
        #[serde(default)]
        subdir: Option<String>,
        #[serde(default, rename = "ref")]
        git_ref: Option<String>,
        #[serde(default)]
        commit: Option<String>,
    },
    /// A development folder, read in place.
    Link { path: String },
    /// First-party code shipped in the package.
    Bundled { path: String },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Entry {
    pub id: String,
    pub source: Source,
    #[serde(default = "yes")]
    pub enabled: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub installed_at: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub manifest_sha256: Option<String>,
    /// What the user approved. `None` means "not recorded yet": the host
    /// records the manifest's permissions at the first successful load.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub granted_permissions: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub bindings: BTreeMap<String, String>,
    /// Keys written by a newer garage survive a rewrite.
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

fn yes() -> bool {
    true
}

impl Entry {
    pub fn new(id: &str, source: Source) -> Self {
        Self {
            id: id.into(),
            source,
            enabled: true,
            installed_at: None,
            manifest_sha256: None,
            granted_permissions: None,
            bindings: BTreeMap::new(),
            extra: Map::new(),
        }
    }

    pub fn is_first_party(&self) -> bool {
        matches!(self.source, Source::Bundled { .. })
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PluginsFile {
    pub version: u32,
    #[serde(default)]
    pub plugins: Vec<Entry>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

impl Default for PluginsFile {
    fn default() -> Self {
        Self { version: FILE_VERSION, plugins: Vec::new(), extra: Map::new() }
    }
}

impl PluginsFile {
    /// A missing file is an empty registry. A file that cannot be read or
    /// parsed is an error that names the path, and is never overwritten.
    pub fn read(paths: &Paths) -> Result<Self, String> {
        let path = paths.registry_file();
        let raw = match std::fs::read_to_string(&path) {
            Ok(r) => r,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Self::default()),
            Err(e) => return Err(format!("cannot read {}: {e}", path.display())),
        };
        let file: Self = serde_json::from_str(&raw)
            .map_err(|e| format!("{} is corrupt ({e}); fix or delete it and run `claude-garage plugin list`", path.display()))?;
        if file.version > FILE_VERSION {
            return Err(format!(
                "{} is version {}, newer than this garage understands ({FILE_VERSION}); upgrade claude-garage",
                path.display(),
                file.version
            ));
        }
        Ok(file)
    }

    /// Temp file next to the target, then rename: a crash leaves the old or
    /// the new complete file.
    pub fn write(&self, paths: &Paths) -> std::io::Result<()> {
        static SEQ: AtomicU64 = AtomicU64::new(0);
        let path = paths.registry_file();
        std::fs::create_dir_all(paths.root())?;
        let tmp = paths.root().join(format!(".plugins.json.{}.{}.tmp", std::process::id(), SEQ.fetch_add(1, Ordering::Relaxed)));
        let body = serde_json::to_string_pretty(self).map_err(std::io::Error::other)?;
        let res = std::fs::write(&tmp, body).and_then(|_| std::fs::rename(&tmp, &path));
        if res.is_err() {
            let _ = std::fs::remove_file(&tmp);
        }
        res
    }
}

/// The token for `id`, created (random 32 bytes, hex, mode 0600) when absent.
pub fn ensure_token(paths: &Paths, id: &str) -> std::io::Result<String> {
    let path = paths.token(id);
    if let Ok(t) = std::fs::read_to_string(&path) {
        let t = t.trim();
        if t.len() >= 32 {
            return Ok(t.to_owned());
        }
    }
    let mut bytes = [0u8; 32];
    getrandom::fill(&mut bytes).map_err(|e| std::io::Error::other(e.to_string()))?;
    let token: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
    std::fs::create_dir_all(path.parent().expect("token has a parent"))?;
    write_private(&path, &token)?;
    Ok(token)
}

#[cfg(unix)]
fn write_private(path: &Path, body: &str) -> std::io::Result<()> {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;
    let mut f = std::fs::OpenOptions::new().write(true).create(true).truncate(true).mode(0o600).open(path)?;
    f.write_all(body.as_bytes())
}

#[cfg(not(unix))]
fn write_private(path: &Path, body: &str) -> std::io::Result<()> {
    std::fs::write(path, body)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn paths() -> (tempfile::TempDir, Paths) {
        let d = tempfile::tempdir().unwrap();
        let p = Paths::new(d.path());
        (d, p)
    }

    #[test]
    fn missing_file_reads_empty_and_nothing_is_created() {
        let (d, p) = paths();
        assert_eq!(PluginsFile::read(&p).unwrap(), PluginsFile::default());
        assert!(!d.path().join("plugins").exists());
    }

    #[test]
    fn the_design_example_round_trips_and_unknown_keys_survive() {
        let (_d, p) = paths();
        std::fs::create_dir_all(p.root()).unwrap();
        let raw = json!({
            "version": 1,
            "seeded": 3,
            "plugins": [{
                "id": "close-button",
                "source": {"kind": "github", "repo": "owner/repo", "subdir": null, "ref": "v0.1.0", "commit": "9f2c"},
                "enabled": true, "installedAt": 1761900000000i64, "manifestSha256": "ab12",
                "grantedPermissions": ["core.actions"], "bindings": {"close-button:close": "x"}, "futureKey": [1]
            }, {
                "id": "dev-plugin", "source": {"kind": "link", "path": "/Users/me/src/dev-plugin"}, "enabled": true
            }]
        });
        std::fs::write(p.registry_file(), raw.to_string()).unwrap();
        let file = PluginsFile::read(&p).unwrap();
        assert_eq!(file.plugins.len(), 2);
        assert_eq!(file.plugins[0].bindings["close-button:close"], "x");
        assert!(matches!(&file.plugins[0].source, Source::Github { git_ref: Some(r), .. } if r == "v0.1.0"));
        file.write(&p).unwrap();
        let back: Value = serde_json::from_str(&std::fs::read_to_string(p.registry_file()).unwrap()).unwrap();
        assert_eq!(back["seeded"], 3);
        assert_eq!(back["plugins"][0]["futureKey"], json!([1]));
        assert_eq!(back["plugins"][0]["source"]["ref"], "v0.1.0");
        assert!(back["plugins"][1].get("manifestSha256").is_none());
        let names: Vec<_> = std::fs::read_dir(p.root()).unwrap().map(|e| e.unwrap().file_name().into_string().unwrap()).collect();
        assert_eq!(names, ["plugins.json"], "no temp file left behind");
    }

    #[test]
    fn a_corrupt_or_newer_file_is_a_clear_error_naming_the_path() {
        let (_d, p) = paths();
        std::fs::create_dir_all(p.root()).unwrap();
        std::fs::write(p.registry_file(), "{nope").unwrap();
        let e = PluginsFile::read(&p).unwrap_err();
        assert!(e.contains("plugins.json") && e.contains("corrupt"), "{e}");
        std::fs::write(p.registry_file(), r#"{"version": 9, "plugins": []}"#).unwrap();
        assert!(PluginsFile::read(&p).unwrap_err().contains("newer"));
    }

    #[cfg(unix)]
    #[test]
    fn tokens_are_random_stable_and_private() {
        use std::os::unix::fs::PermissionsExt;
        let (_d, p) = paths();
        let a = ensure_token(&p, "one").unwrap();
        assert_eq!(a.len(), 64);
        assert_eq!(ensure_token(&p, "one").unwrap(), a, "stable across calls");
        assert_ne!(ensure_token(&p, "two").unwrap(), a);
        assert_eq!(std::fs::metadata(p.token("one")).unwrap().permissions().mode() & 0o777, 0o600);
    }
}
