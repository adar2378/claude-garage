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
//!
//! # Who writes `plugins.json`
//!
//! The daemon owns it. Every change goes through [`PluginsFile::update`],
//! which holds an in-process mutex and (on unix) an exclusive `flock` on
//! `.plugins.lock` for the whole read-modify-write, so two writers never lose
//! each other's edit. A CLI that changes the registry must either call the
//! daemon's routes while the daemon is up, or use `PluginsFile::update`
//! itself when it is not; it must never read, edit and `write` by hand.

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

    /// Advisory lock file that serialises read-modify-write of the registry.
    pub fn registry_lock(&self) -> PathBuf {
        self.root.join(".plugins.lock")
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

    /// Read, change and write back under the registry lock. `change` returns
    /// whether it modified anything; an unmodified file is not rewritten.
    /// This is the only supported way to edit the registry.
    pub fn update<T>(paths: &Paths, change: impl FnOnce(&mut Self) -> (bool, T)) -> Result<T, String> {
        let _guard = RegistryLock::acquire(paths).map_err(|e| format!("cannot lock {}: {e}", paths.registry_lock().display()))?;
        let mut file = Self::read(paths)?;
        let (dirty, out) = change(&mut file);
        if dirty {
            file.write(paths).map_err(|e| format!("cannot write {}: {e}", paths.registry_file().display()))?;
        }
        Ok(out)
    }

    /// Temp file next to the target, then rename: a crash leaves the old or
    /// the new complete file. Callers that read first must use
    /// [`PluginsFile::update`] so the pair is atomic.
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

/// Holds the registry lock until dropped.
struct RegistryLock {
    #[allow(dead_code)] // held for its Drop
    process: std::sync::MutexGuard<'static, ()>,
    #[cfg(unix)]
    file: std::fs::File,
}

static REGISTRY_MUTEX: std::sync::Mutex<()> = std::sync::Mutex::new(());

impl RegistryLock {
    fn acquire(paths: &Paths) -> std::io::Result<Self> {
        // A panic inside a previous critical section must not wedge the registry.
        let process = REGISTRY_MUTEX.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        std::fs::create_dir_all(paths.root())?;
        #[cfg(unix)]
        {
            use std::os::fd::AsRawFd;
            let file = std::fs::OpenOptions::new().read(true).write(true).create(true).truncate(false).open(paths.registry_lock())?;
            // SAFETY: flock(2) on a descriptor this function owns.
            if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX) } != 0 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(Self { process, file })
        }
        #[cfg(not(unix))]
        {
            // Other platforms: the in-process mutex only. Other processes
            // must go through the daemon's routes (see the module docs).
            Ok(Self { process })
        }
    }
}

#[cfg(unix)]
impl Drop for RegistryLock {
    fn drop(&mut self) {
        use std::os::fd::AsRawFd;
        // SAFETY: releasing the lock taken in `acquire`; closing the file would too.
        unsafe { libc::flock(self.file.as_raw_fd(), libc::LOCK_UN) };
    }
}

/// A random 32-byte hex token.
pub fn random_token() -> std::io::Result<String> {
    let mut bytes = [0u8; 32];
    getrandom::fill(&mut bytes).map_err(|e| std::io::Error::other(e.to_string()))?;
    Ok(bytes.iter().map(|b| format!("{b:02x}")).collect())
}

/// The token in `path`, created (random 32 bytes, hex, mode 0600) when absent
/// or too short. An existing file with looser permissions is tightened.
pub fn ensure_secret_file(path: &Path) -> std::io::Result<String> {
    if let Ok(t) = std::fs::read_to_string(path) {
        let t = t.trim();
        if t.len() >= 32 {
            restrict(path)?;
            return Ok(t.to_owned());
        }
    }
    let token = random_token()?;
    create_private_dir(path.parent().expect("secret file has a parent"))?;
    write_private(path, &token)?;
    Ok(token)
}

/// The token for `id`, created (random 32 bytes, hex, mode 0600) when absent.
pub fn ensure_token(paths: &Paths, id: &str) -> std::io::Result<String> {
    ensure_secret_file(&paths.token(id))
}

/// `<GARAGE_DIR>/ui-token`: the per-install credential only the TUI (and the
/// CLI) reads. It authorises triggering any plugin's actions, reloading the
/// registry and reporting slot geometry.
pub fn ui_token_path(garage_dir: &Path) -> PathBuf {
    garage_dir.join("ui-token")
}

pub fn ensure_ui_token(garage_dir: &Path) -> std::io::Result<String> {
    ensure_secret_file(&ui_token_path(garage_dir))
}

/// Create `dir` (and parents) with mode 0700 where the OS has modes.
pub fn create_private_dir(dir: &Path) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        std::fs::DirBuilder::new().recursive(true).mode(0o700).create(dir)
    }
    #[cfg(not(unix))]
    {
        std::fs::create_dir_all(dir)
    }
}

/// Force mode 0600 on an existing file.
#[cfg(unix)]
pub fn restrict(path: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    if std::fs::metadata(path)?.permissions().mode() & 0o777 != 0o600 {
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
    }
    Ok(())
}

#[cfg(not(unix))]
pub fn restrict(_path: &Path) -> std::io::Result<()> {
    Ok(())
}

/// Write `body` to `path` readable by the owner only, whether or not the file
/// existed (an existing file is tightened, not just truncated).
#[cfg(unix)]
pub fn write_private(path: &Path, body: &str) -> std::io::Result<()> {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;
    let mut f = std::fs::OpenOptions::new().write(true).create(true).truncate(true).mode(0o600).open(path)?;
    restrict(path)?;
    f.write_all(body.as_bytes())
}

#[cfg(not(unix))]
pub fn write_private(path: &Path, body: &str) -> std::io::Result<()> {
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

    #[test]
    fn concurrent_updates_never_lose_an_edit() {
        let (_d, p) = paths();
        let handles: Vec<_> = (0..8)
            .map(|t| {
                let p = p.clone();
                std::thread::spawn(move || {
                    for i in 0..10 {
                        let id = format!("p{t}-{i}");
                        PluginsFile::update(&p, |f| {
                            f.plugins.push(Entry::new(&id, Source::Link { path: "/x".into() }));
                            (true, ())
                        })
                        .unwrap();
                    }
                })
            })
            .collect();
        handles.into_iter().for_each(|h| h.join().unwrap());
        assert_eq!(PluginsFile::read(&p).unwrap().plugins.len(), 80);
    }

    #[test]
    fn an_update_that_changes_nothing_writes_nothing_and_a_corrupt_file_is_never_replaced() {
        let (_d, p) = paths();
        PluginsFile::update(&p, |_| (false, ())).unwrap();
        assert!(!p.registry_file().exists());
        std::fs::write(p.registry_file(), "{broken").unwrap();
        let e = PluginsFile::update(&p, |f| {
            f.plugins.clear();
            (true, ())
        })
        .unwrap_err();
        assert!(e.contains("corrupt"), "{e}");
        assert_eq!(std::fs::read_to_string(p.registry_file()).unwrap(), "{broken");
    }

    #[cfg(unix)]
    #[test]
    fn existing_secret_files_with_loose_modes_are_tightened_and_new_ones_are_private() {
        use std::os::unix::fs::PermissionsExt;
        let mode = |p: &Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
        let (d, p) = paths();
        let tok = ensure_token(&p, "loose").unwrap();
        std::fs::set_permissions(p.token("loose"), std::fs::Permissions::from_mode(0o644)).unwrap();
        assert_eq!(ensure_token(&p, "loose").unwrap(), tok, "the token itself is kept");
        assert_eq!(mode(&p.token("loose")), 0o600);
        assert_eq!(mode(p.token("loose").parent().unwrap()), 0o700, "the tokens dir is private");
        let ui = ensure_ui_token(d.path()).unwrap();
        assert_eq!(ui.len(), 64);
        assert_eq!(mode(&ui_token_path(d.path())), 0o600);
        assert_ne!(ui, tok);
        write_private(&ui_token_path(d.path()), "x").unwrap();
        assert_eq!(mode(&ui_token_path(d.path())), 0o600);
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
