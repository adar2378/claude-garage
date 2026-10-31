//! Port of `daemon/src/settings-install.js` and `daemon/src/claude-home.js`:
//! settings.json merge with parse-or-refuse, timestamped backup and atomic write.
//!
//! Safety order, shared by the hooks and statusline installs: parse-or-refuse
//! (a corrupt file is a 422 and stays byte-for-byte untouched) -> caller
//! merges its fragment -> timestamped backup of the pre-write file -> atomic
//! write (tmp + rename in the same directory, so a crash mid-write can never
//! leave a half-written settings.json).

use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::{Map, Value};

/// Claude Code's config root: `GARAGE_CLAUDE_HOME`, else `<home>/.claude`.
/// Both inputs are passed in so no caller has to read the process environment.
pub fn claude_home(garage_claude_home: Option<&str>, home: Option<&str>) -> PathBuf {
    match garage_claude_home.filter(|s| !s.is_empty()) {
        Some(h) => PathBuf::from(h),
        None => PathBuf::from(home.unwrap_or("")).join(".claude"),
    }
}

/// `claude_home` read from the real environment (what production uses).
pub fn claude_home_from_env() -> PathBuf {
    claude_home(
        std::env::var("GARAGE_CLAUDE_HOME").ok().as_deref(),
        std::env::var("HOME").ok().as_deref(),
    )
}

/// An install failure carrying the HTTP status the route answers with.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstallError {
    pub status: u16,
    pub message: String,
}

impl InstallError {
    fn new(status: u16, message: impl Into<String>) -> Self {
        Self {
            status,
            message: message.into(),
        }
    }
    fn io(err: std::io::Error) -> Self {
        Self::new(500, err.to_string())
    }
}

/// `settings` is `{}` for a missing/empty file; `raw` is the exact original
/// text, or `None` if the file did not exist (no backup is needed then).
pub struct Settings {
    pub settings: Map<String, Value>,
    pub raw: Option<String>,
}

pub async fn read_settings_or_refuse(path: &Path) -> Result<Settings, InstallError> {
    let raw = match tokio::fs::read_to_string(path).await {
        Ok(s) => Some(s),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        Err(e) => return Err(InstallError::io(e)),
    };
    let mut settings = Map::new();
    if let Some(text) = raw.as_deref().filter(|t| !t.trim().is_empty()) {
        match serde_json::from_str::<Value>(text) {
            Ok(Value::Object(o)) => settings = o,
            Ok(_) => {
                return Err(InstallError::new(
                    422,
                    format!("{} is not a JSON object — the file was left untouched", path.display()),
                ))
            }
            Err(_) => {
                return Err(InstallError::new(
                    422,
                    format!(
                        "{} is not valid JSON — fix it (or remove it) and retry; the file was left untouched",
                        path.display()
                    ),
                ))
            }
        }
    }
    Ok(Settings { settings, raw })
}

/// Backs up (when `raw` is `Some`) then atomically writes `merged`. Returns
/// the backup path, or `None` when there was nothing to back up.
pub async fn write_settings_atomic(
    path: &Path,
    raw: Option<&str>,
    merged: &Map<String, Value>,
) -> Result<Option<PathBuf>, InstallError> {
    // A symlinked settings.json (dotfiles managers) is written through: the
    // link stays a link and the resolved target gets the new content.
    let resolved = match tokio::fs::canonicalize(path).await {
        Ok(real) => real,
        Err(_) => path.to_path_buf(),
    };
    let path = resolved.as_path();
    if let Some(dir) = path.parent() {
        tokio::fs::create_dir_all(dir)
            .await
            .map_err(InstallError::io)?;
    }
    let mut backup = None;
    if raw.is_some() {
        let b = suffixed(
            path,
            &format!(".garage-backup-{}", iso_stamp(SystemTime::now())),
        );
        tokio::fs::copy(path, &b).await.map_err(InstallError::io)?;
        backup = Some(b);
    }
    let tmp = suffixed(path, &format!(".garage-tmp-{}", std::process::id()));
    let mut text =
        serde_json::to_string_pretty(merged).map_err(|e| InstallError::new(500, e.to_string()))?;
    text.push('\n');
    tokio::fs::write(&tmp, text)
        .await
        .map_err(InstallError::io)?;
    if raw.is_some() {
        // Keep the original mode (a private 0600 settings.json must not widen).
        let perms = tokio::fs::metadata(path)
            .await
            .map_err(InstallError::io)?
            .permissions();
        tokio::fs::set_permissions(&tmp, perms)
            .await
            .map_err(InstallError::io)?;
    }
    tokio::fs::rename(&tmp, path)
        .await
        .map_err(InstallError::io)?;
    Ok(backup)
}

fn suffixed(path: &Path, suffix: &str) -> PathBuf {
    let mut s = path.as_os_str().to_owned();
    s.push(suffix);
    PathBuf::from(s)
}

/// `new Date().toISOString().replace(/[:.]/g, "-")`, e.g. `2026-10-31T12-00-00-123Z`.
fn iso_stamp(t: SystemTime) -> String {
    let ms = t
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0);
    let (secs, milli) = (ms.div_euclid(1000), ms.rem_euclid(1000));
    let (days, tod) = (secs.div_euclid(86_400), secs.rem_euclid(86_400));
    // Civil-from-days (Howard Hinnant).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!(
        "{y:04}-{m:02}-{d:02}T{:02}-{:02}-{:02}-{milli:03}Z",
        tod / 3600,
        tod % 3600 / 60,
        tod % 60
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn claude_home_prefers_the_override() {
        assert_eq!(claude_home(Some("/x"), Some("/h")), PathBuf::from("/x"));
        assert_eq!(claude_home(None, Some("/h")), PathBuf::from("/h/.claude"));
        assert_eq!(
            claude_home(Some(""), Some("/h")),
            PathBuf::from("/h/.claude")
        );
    }

    #[test]
    fn iso_stamp_matches_to_iso_string_with_separators_dashed() {
        let t = UNIX_EPOCH + std::time::Duration::from_millis(1_782_864_000_123);
        assert_eq!(iso_stamp(t), "2026-07-01T00-00-00-123Z");
        assert_eq!(iso_stamp(UNIX_EPOCH), "1970-01-01T00-00-00-000Z");
    }

    #[tokio::test]
    async fn missing_and_empty_files_read_as_empty_settings() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("settings.json");
        let s = read_settings_or_refuse(&p).await.unwrap();
        assert!(s.settings.is_empty() && s.raw.is_none());
        std::fs::write(&p, "  \n").unwrap();
        let s = read_settings_or_refuse(&p).await.unwrap();
        assert!(s.settings.is_empty() && s.raw.is_some());
    }

    #[tokio::test]
    async fn invalid_json_and_non_objects_are_refused_with_422() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("settings.json");
        for body in ["{ not: valid json !!", "[1]", "3"] {
            std::fs::write(&p, body).unwrap();
            let e = read_settings_or_refuse(&p).await.err().unwrap();
            assert_eq!(e.status, 422, "{body}");
            assert_eq!(std::fs::read_to_string(&p).unwrap(), body);
        }
    }

    #[tokio::test]
    async fn write_backs_up_an_existing_file_and_creates_missing_dirs() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("nested/settings.json");
        let mut m = Map::new();
        m.insert("a".into(), Value::from(1));
        assert_eq!(write_settings_atomic(&p, None, &m).await.unwrap(), None);
        assert_eq!(std::fs::read_to_string(&p).unwrap(), "{\n  \"a\": 1\n}\n");

        let raw = std::fs::read_to_string(&p).unwrap();
        m.insert("b".into(), Value::from(2));
        let backup = write_settings_atomic(&p, Some(&raw), &m)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(std::fs::read_to_string(backup).unwrap(), raw);
        assert!(std::fs::read_to_string(&p).unwrap().contains("\"b\": 2"));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn write_through_a_symlink_updates_the_target_and_keeps_the_link() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("dotfiles/settings.json");
        std::fs::create_dir_all(target.parent().unwrap()).unwrap();
        std::fs::write(&target, "{}\n").unwrap();
        let link = dir.path().join("settings.json");
        std::os::unix::fs::symlink(&target, &link).unwrap();

        let mut m = Map::new();
        m.insert("a".into(), Value::from(1));
        write_settings_atomic(&link, Some("{}\n"), &m)
            .await
            .unwrap();

        assert!(std::fs::symlink_metadata(&link)
            .unwrap()
            .file_type()
            .is_symlink());
        assert_eq!(std::fs::read_link(&link).unwrap(), target);
        assert!(std::fs::read_to_string(&target)
            .unwrap()
            .contains("\"a\": 1"));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn write_preserves_the_original_file_mode() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("settings.json");
        std::fs::write(&p, "{}\n").unwrap();
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o600)).unwrap();

        let mut m = Map::new();
        m.insert("a".into(), Value::from(1));
        write_settings_atomic(&p, Some("{}\n"), &m).await.unwrap();

        assert_eq!(
            std::fs::metadata(&p).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }
}
