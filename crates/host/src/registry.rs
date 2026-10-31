//! `<garage dir>/hosts/<id>.json`: how the daemon and the TUI find a live host.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// One host's registry record.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Entry {
    pub id: String,
    /// The host process.
    pub pid: u32,
    /// The command running inside the host (`claude`).
    pub child_pid: u32,
    pub port: u16,
    pub token: String,
    pub dir: String,
    /// Epoch milliseconds.
    pub created_at: u64,
    /// The latest OSC window title the command set (may be empty).
    #[serde(default)]
    pub title: String,
}

impl Entry {
    /// `127.0.0.1:<port>`, the address the TUI's `AttachSpec::Socket` carries.
    pub fn addr(&self) -> String {
        format!("127.0.0.1:{}", self.port)
    }
}

/// A file name for a session id, the same on every OS and for write and read.
/// Ids look like `garage/<ws>/<label>` but are only trusted as far as the
/// caller validated them, so every byte outside `[A-Za-z0-9_-]` becomes `%XX`
/// (including `/`, `\`, `.`, `:` and `%` itself). The result can never hold a
/// path separator, a drive letter or `..`, and the mapping is injective:
/// distinct ids never share a file.
pub fn file_stem(id: &str) -> String {
    let mut out = String::with_capacity(id.len() + 8);
    for b in id.bytes() {
        if b.is_ascii_alphanumeric() || b == b'_' || b == b'-' {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

pub fn path_for(hosts_dir: &Path, id: &str) -> PathBuf {
    hosts_dir.join(format!("{}.json", file_stem(id)))
}

/// Where a host's stdout/stderr go (the daemon points the spawned process here).
pub fn log_path(hosts_dir: &Path, id: &str) -> PathBuf {
    hosts_dir.join(format!("{}.log", file_stem(id)))
}

/// The record for `id`. A record whose own `id` differs (a copied or tampered
/// file) is not a record for `id`.
pub fn read(hosts_dir: &Path, id: &str) -> Option<Entry> {
    parse(&std::fs::read_to_string(path_for(hosts_dir, id)).ok()?).filter(|e| e.id == id)
}

fn parse(text: &str) -> Option<Entry> {
    serde_json::from_str(text).ok()
}

/// Every parseable record. Unreadable or half-written files are skipped.
pub fn read_all(hosts_dir: &Path) -> Vec<Entry> {
    let Ok(dir) = std::fs::read_dir(hosts_dir) else { return Vec::new() };
    let mut out: Vec<Entry> = dir
        .flatten()
        .filter(|e| e.path().extension().is_some_and(|x| x == "json"))
        .filter_map(|e| parse(&std::fs::read_to_string(e.path()).ok()?))
        .collect();
    out.sort_by(|a, b| a.id.cmp(&b.id));
    out
}

/// Create the hosts directory. Records hold each host's token, so on Unix the
/// directory is private to the user (0700), including when it already exists.
///
/// On Windows the directory sits under the per-user profile (`%USERPROFILE%\.garage`),
/// which is readable by that user (and administrators) only by default; this
/// code relies on that and sets no ACL of its own.
pub fn ensure_dir(hosts_dir: &Path) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
        std::fs::DirBuilder::new().recursive(true).mode(0o700).create(hosts_dir)?;
        std::fs::set_permissions(hosts_dir, std::fs::Permissions::from_mode(0o700))
    }
    #[cfg(not(unix))]
    std::fs::create_dir_all(hosts_dir)
}

/// Atomic write (temp file, then rename), so a reader never sees half a file.
/// On Unix both files are created 0600, so the token is never readable by
/// another user, not even briefly.
pub fn write(hosts_dir: &Path, entry: &Entry) -> std::io::Result<()> {
    use std::io::Write;
    ensure_dir(hosts_dir)?;
    let path = path_for(hosts_dir, &entry.id);
    let tmp = path.with_extension("json.tmp");
    let _ = std::fs::remove_file(&tmp); // a leftover from a crash would block create_new
    let mut opts = std::fs::OpenOptions::new();
    opts.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(0o600);
    }
    let mut f = opts.open(&tmp)?;
    f.write_all(serde_json::to_string(entry).map_err(std::io::Error::other)?.as_bytes())?;
    drop(f);
    std::fs::rename(&tmp, &path)
}

pub fn remove(hosts_dir: &Path, id: &str) {
    let _ = std::fs::remove_file(path_for(hosts_dir, id));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(id: &str) -> Entry {
        Entry { id: id.into(), pid: 1, child_pid: 2, port: 4000, token: "t".repeat(32), dir: "/w".into(), created_at: 5, title: String::new() }
    }

    #[test]
    fn write_read_remove_roundtrip_with_slash_ids() {
        let d = tempfile::tempdir().unwrap();
        let e = entry("garage/ws/a");
        write(d.path(), &e).unwrap();
        assert!(d.path().join("garage%2Fws%2Fa.json").exists());
        assert_eq!(read(d.path(), "garage/ws/a"), Some(e.clone()));
        assert_eq!(read(d.path(), "garage/ws/b"), None);
        assert_eq!(e.addr(), "127.0.0.1:4000");
        remove(d.path(), "garage/ws/a");
        assert_eq!(read(d.path(), "garage/ws/a"), None);
    }

    #[test]
    fn stems_never_contain_separators_dots_or_colons_and_never_collide() {
        for id in [
            "garage/ws/a", "../../etc/passwd", "..\\..\\x", "C:\\Windows\\x", "a/../b", "/abs", "x:y", "a.b", "%2F", "é",
            "con", "", "a\0b", "..",
        ] {
            let stem = file_stem(id);
            assert!(
                stem.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-' | b'%')),
                "{id:?} -> {stem:?}"
            );
            assert!(!stem.contains(".."), "{id:?} -> {stem:?}");
        }
        assert_eq!(file_stem("garage/ws/a"), "garage%2Fws%2Fa");
        // '%' is escaped too, so an id that spells an escape cannot alias another id.
        assert_ne!(file_stem("garage/ws"), file_stem("garage%2Fws"));
        // Traversal ids stay inside the hosts dir.
        let dir = Path::new("/h");
        assert_eq!(path_for(dir, "../../etc/passwd").parent(), Some(dir));
    }

    #[test]
    fn a_record_filed_under_another_ids_name_is_not_that_ids_record() {
        let d = tempfile::tempdir().unwrap();
        write(d.path(), &entry("garage/ws/real")).unwrap();
        std::fs::copy(path_for(d.path(), "garage/ws/real"), path_for(d.path(), "garage/ws/alias")).unwrap();
        assert_eq!(read(d.path(), "garage/ws/alias"), None);
        assert!(read(d.path(), "garage/ws/real").is_some());
    }

    #[cfg(unix)]
    #[test]
    fn the_directory_is_0700_and_records_are_0600_even_when_the_directory_pre_exists() {
        use std::os::unix::fs::PermissionsExt;
        let mode = |p: &Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
        let d = tempfile::tempdir().unwrap();

        let fresh = d.path().join("a/hosts");
        write(&fresh, &entry("garage/ws/a")).unwrap();
        assert_eq!(mode(&fresh), 0o700);
        assert_eq!(mode(&path_for(&fresh, "garage/ws/a")), 0o600);

        let loose = d.path().join("loose");
        std::fs::create_dir(&loose).unwrap();
        std::fs::set_permissions(&loose, std::fs::Permissions::from_mode(0o755)).unwrap();
        write(&loose, &entry("garage/ws/b")).unwrap();
        write(&loose, &entry("garage/ws/b")).unwrap(); // rewrite keeps 0600
        assert_eq!(mode(&loose), 0o700, "an existing directory is tightened");
        assert_eq!(mode(&path_for(&loose, "garage/ws/b")), 0o600);
    }

    #[test]
    fn read_all_skips_garbage_temp_files_and_other_extensions() {
        let d = tempfile::tempdir().unwrap();
        write(d.path(), &entry("garage/ws/b")).unwrap();
        write(d.path(), &entry("garage/ws/a")).unwrap();
        std::fs::write(d.path().join("broken.json"), "{not json").unwrap();
        std::fs::write(d.path().join("x.json.tmp"), "{}").unwrap();
        std::fs::write(d.path().join("garage%2Fws%2Fa.log"), "log").unwrap();
        let ids: Vec<_> = read_all(d.path()).into_iter().map(|e| e.id).collect();
        assert_eq!(ids, ["garage/ws/a", "garage/ws/b"]);
        assert!(read_all(&d.path().join("missing")).is_empty());
    }
}
