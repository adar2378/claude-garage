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

/// Whether `dir` lies inside `profile` (the user's profile directory), by
/// case-insensitive component comparison. A path with `..` never counts: it
/// could climb out of the profile.
pub fn is_under_profile(dir: &Path, profile: &Path) -> bool {
    use std::path::Component;
    let norm = |p: &Path| -> Option<Vec<String>> {
        p.components()
            .map(|c| match c {
                Component::ParentDir => None,
                c => Some(c.as_os_str().to_string_lossy().to_lowercase()),
            })
            .collect()
    };
    match (norm(dir), norm(profile)) {
        (Some(d), Some(p)) => !p.is_empty() && d.starts_with(&p),
        _ => false,
    }
}

/// Create the hosts directory. Records hold each host's token, so the
/// directory must be private to the user.
///
/// Unix: mode 0700, including when it already exists. Windows: the directory
/// normally sits under the per-user profile (`%USERPROFILE%\.garage`), which
/// other users cannot read by default. If `GARAGE_DIR` points elsewhere (for
/// example `D:\garage`, whose ACL is typically inherited from a shared
/// drive root) an owner-only ACL is set on it, and failing to set it is an
/// error: the host refuses to put tokens in a directory it cannot protect.
pub fn ensure_dir(hosts_dir: &Path) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
        std::fs::DirBuilder::new().recursive(true).mode(0o700).create(hosts_dir)?;
        std::fs::set_permissions(hosts_dir, std::fs::Permissions::from_mode(0o700))
    }
    #[cfg(windows)]
    {
        std::fs::create_dir_all(hosts_dir)?;
        let inside = std::env::var_os("USERPROFILE").is_some_and(|p| is_under_profile(hosts_dir, Path::new(&p)));
        if inside {
            return Ok(());
        }
        windows_acl::restrict_to_owner(hosts_dir).map_err(|e| {
            std::io::Error::other(format!(
                "{} is outside your user profile and could not be made private ({e}); set GARAGE_DIR under {}",
                hosts_dir.display(),
                std::env::var("USERPROFILE").unwrap_or_else(|_| "your profile".into())
            ))
        })
    }
    #[cfg(not(any(unix, windows)))]
    std::fs::create_dir_all(hosts_dir)
}

#[cfg(windows)]
mod windows_acl {
    use std::os::windows::ffi::OsStrExt;
    use std::path::Path;

    use windows_sys::Win32::Foundation::{LocalFree, ERROR_SUCCESS};
    use windows_sys::Win32::Security::Authorization::{
        ConvertStringSecurityDescriptorToSecurityDescriptorW, SetNamedSecurityInfoW, SDDL_REVISION_1, SE_FILE_OBJECT,
    };
    use windows_sys::Win32::Security::{
        GetSecurityDescriptorDacl, ACL, DACL_SECURITY_INFORMATION, PROTECTED_DACL_SECURITY_INFORMATION,
    };

    /// Replace the directory's ACL with one that grants full control to the
    /// owner only (SDDL `OW` = OWNER RIGHTS) and does not inherit from the
    /// parent, so files created inside inherit the same owner-only access.
    pub fn restrict_to_owner(dir: &Path) -> Result<(), String> {
        let path: Vec<u16> = dir.as_os_str().encode_wide().chain(std::iter::once(0)).collect();
        let sddl: Vec<u16> = "D:PAI(A;OICI;FA;;;OW)".encode_utf16().chain(std::iter::once(0)).collect();
        // SAFETY: FFI. `sd` is allocated by the call and freed with LocalFree
        // on every path; the DACL pointer points into `sd` and is used only
        // before that free; `path` and `sddl` are NUL-terminated.
        unsafe {
            let mut sd = std::ptr::null_mut();
            if ConvertStringSecurityDescriptorToSecurityDescriptorW(sddl.as_ptr(), SDDL_REVISION_1, &mut sd, std::ptr::null_mut()) == 0 {
                return Err(format!("security descriptor: {}", std::io::Error::last_os_error()));
            }
            let (mut present, mut defaulted) = (0i32, 0i32);
            let mut dacl: *mut ACL = std::ptr::null_mut();
            let result = if GetSecurityDescriptorDacl(sd, &mut present, &mut dacl, &mut defaulted) == 0 || present == 0 {
                Err(format!("dacl: {}", std::io::Error::last_os_error()))
            } else {
                let rc = SetNamedSecurityInfoW(
                    path.as_ptr().cast_mut(),
                    SE_FILE_OBJECT,
                    DACL_SECURITY_INFORMATION | PROTECTED_DACL_SECURITY_INFORMATION,
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                    dacl,
                    std::ptr::null_mut(),
                );
                if rc == ERROR_SUCCESS { Ok(()) } else { Err(format!("SetNamedSecurityInfo error {rc}")) }
            };
            LocalFree(sd);
            result
        }
    }
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

/// Remove the record for `seen.id` only if the file on disk still names the
/// same host (pid AND token). Anyone who decided to delete a record after a
/// delay (a probe, a shutdown wait) must use this: in the meantime a respawn
/// may have written a new record under the same id, which must survive.
/// The file is re-read immediately before the removal; the remaining window
/// is that of a single read-then-unlink. Returns whether a record was removed.
pub fn remove_if_same(hosts_dir: &Path, seen: &Entry) -> bool {
    match read(hosts_dir, &seen.id) {
        Some(now) if now.pid == seen.pid && now.token == seen.token => {
            std::fs::remove_file(path_for(hosts_dir, &seen.id)).is_ok()
        }
        _ => false,
    }
}

/// Whether a process with this pid exists (a zombie or a reused pid counts as
/// alive: the caller only uses this to avoid deleting records of live hosts).
pub fn pid_alive(pid: u32) -> bool {
    #[cfg(unix)]
    {
        let Ok(p) = i32::try_from(pid) else { return false };
        if p <= 0 {
            return false;
        }
        // SAFETY: signal 0 only checks that the pid can be signalled.
        unsafe { libc::kill(p, 0) == 0 || std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM) }
    }
    #[cfg(windows)]
    {
        use windows_sys::Win32::Foundation::{CloseHandle, ERROR_ACCESS_DENIED, STILL_ACTIVE};
        use windows_sys::Win32::System::Threading::{GetExitCodeProcess, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION};
        if pid == 0 {
            return false;
        }
        // SAFETY: plain Win32 calls; the handle is closed before returning.
        unsafe {
            let h = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
            if h.is_null() {
                return std::io::Error::last_os_error().raw_os_error() == Some(ERROR_ACCESS_DENIED as i32);
            }
            let mut code = 0u32;
            let ok = GetExitCodeProcess(h, &mut code) != 0;
            CloseHandle(h);
            !ok || code == STILL_ACTIVE as u32
        }
    }
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
    fn remove_if_same_spares_a_record_a_respawn_wrote_in_the_meantime() {
        let d = tempfile::tempdir().unwrap();
        let old = entry("garage/ws/a");
        write(d.path(), &old).unwrap();
        // Same pid, same token: ours, removed.
        assert!(remove_if_same(d.path(), &old));
        assert_eq!(read(d.path(), "garage/ws/a"), None);
        assert!(!remove_if_same(d.path(), &old), "already gone");
        // A respawn replaced it: a different pid or a different token both survive.
        for fresh in [Entry { pid: 99, ..entry("garage/ws/a") }, Entry { token: "z".repeat(32), ..entry("garage/ws/a") }] {
            write(d.path(), &fresh).unwrap();
            assert!(!remove_if_same(d.path(), &old));
            assert_eq!(read(d.path(), "garage/ws/a"), Some(fresh));
        }
    }

    #[test]
    fn pid_alive_knows_this_process_and_not_pid_zero() {
        assert!(pid_alive(std::process::id()));
        assert!(!pid_alive(0));
    }

    #[test]
    fn profile_membership_is_by_component_case_insensitive_and_never_through_dotdot() {
        let p = Path::new("C:/Users/Me");
        assert!(is_under_profile(Path::new("C:/Users/Me/.garage/hosts"), p));
        assert!(is_under_profile(Path::new("c:/users/me/.garage"), p));
        assert!(!is_under_profile(Path::new("D:/garage/hosts"), p));
        assert!(!is_under_profile(Path::new("C:/Users/Meagan/.garage"), p), "prefix of a name is not a parent");
        assert!(!is_under_profile(Path::new("C:/Users/Me/../Other/.garage"), p));
        assert!(!is_under_profile(Path::new("anything"), Path::new("")));
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
