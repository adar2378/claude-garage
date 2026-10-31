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

/// Session ids contain `/` (`garage/<ws>/<label>`), which a file name cannot.
pub fn file_stem(id: &str) -> String {
    id.replace('/', "__")
}

pub fn path_for(hosts_dir: &Path, id: &str) -> PathBuf {
    hosts_dir.join(format!("{}.json", file_stem(id)))
}

/// Where a host's stdout/stderr go (the daemon points the spawned process here).
pub fn log_path(hosts_dir: &Path, id: &str) -> PathBuf {
    hosts_dir.join(format!("{}.log", file_stem(id)))
}

pub fn read(hosts_dir: &Path, id: &str) -> Option<Entry> {
    parse(&std::fs::read_to_string(path_for(hosts_dir, id)).ok()?)
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

/// Atomic write (temp file, then rename), so a reader never sees half a file.
pub fn write(hosts_dir: &Path, entry: &Entry) -> std::io::Result<()> {
    std::fs::create_dir_all(hosts_dir)?;
    let path = path_for(hosts_dir, &entry.id);
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, serde_json::to_string(entry).map_err(std::io::Error::other)?)?;
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
        assert!(d.path().join("garage__ws__a.json").exists());
        assert_eq!(read(d.path(), "garage/ws/a"), Some(e.clone()));
        assert_eq!(read(d.path(), "garage/ws/b"), None);
        assert_eq!(e.addr(), "127.0.0.1:4000");
        remove(d.path(), "garage/ws/a");
        assert_eq!(read(d.path(), "garage/ws/a"), None);
    }

    #[test]
    fn read_all_skips_garbage_temp_files_and_other_extensions() {
        let d = tempfile::tempdir().unwrap();
        write(d.path(), &entry("garage/ws/b")).unwrap();
        write(d.path(), &entry("garage/ws/a")).unwrap();
        std::fs::write(d.path().join("broken.json"), "{not json").unwrap();
        std::fs::write(d.path().join("x.json.tmp"), "{}").unwrap();
        std::fs::write(d.path().join("garage__ws__a.log"), "log").unwrap();
        let ids: Vec<_> = read_all(d.path()).into_iter().map(|e| e.id).collect();
        assert_eq!(ids, ["garage/ws/a", "garage/ws/b"]);
        assert!(read_all(&d.path().join("missing")).is_empty());
    }
}
