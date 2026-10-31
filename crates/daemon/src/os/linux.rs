//! Linux: desktop notifications through `notify-send` and the folder picker
//! through `zenity` or `kdialog`, whichever is installed. Neither installed
//! is `Unsupported` (a headless box has no desktop to notify). Compiled on
//! every OS so the argv builders and result mapping are unit-tested
//! everywhere; tests never start a notifier or a dialog.

use std::path::PathBuf;
use std::time::Duration;

use async_trait::async_trait;
use tokio::process::Command;

use super::{spawn_detached, Os, OsError};

pub const PICKER_TIMEOUT: Duration = Duration::from_secs(120);
const TITLE: &str = "Add claude-garage workspace";

/// `notify-send <title> <body>`; `--` keeps a title starting with `-` a title.
pub fn notify_args(title: &str, body: &str) -> Vec<String> {
    vec!["--app-name=claude-garage".into(), "--".into(), title.into(), body.into()]
}

/// The pickers to try, in order: (program, args).
pub fn picker_commands() -> Vec<(&'static str, Vec<String>)> {
    vec![
        ("zenity", vec!["--file-selection".into(), "--directory".into(), format!("--title={TITLE}")]),
        ("kdialog", vec!["--getexistingdirectory".into(), ".".into(), "--title".into(), TITLE.into()]),
    ]
}

/// Both pickers exit 0 with the path, and 1 with no output when cancelled.
pub fn interpret_picker(code: Option<i32>, stdout: &str, stderr: &str) -> Result<Option<PathBuf>, OsError> {
    match code {
        Some(0) => {
            let dir = stdout.trim_end_matches(['\n', '\r']);
            Ok((!dir.is_empty()).then(|| PathBuf::from(dir)))
        }
        Some(1) if stderr.trim().is_empty() => Ok(None),
        _ => {
            let detail = stderr.trim();
            Err(OsError::Failed(if detail.is_empty() { "folder picker failed".into() } else { detail.to_owned() }))
        }
    }
}

pub struct LinuxOs;

#[async_trait]
impl Os for LinuxOs {
    async fn notify(&self, title: &str, body: &str) -> Result<(), OsError> {
        let mut cmd = Command::new("notify-send");
        cmd.args(notify_args(title, body))
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .kill_on_drop(true);
        match tokio::time::timeout(Duration::from_secs(10), cmd.status()).await {
            Ok(Ok(_)) => Ok(()),
            Ok(Err(e)) if e.kind() == std::io::ErrorKind::NotFound => Err(OsError::Unsupported),
            Ok(Err(e)) => Err(OsError::Failed(e.to_string())),
            Err(_) => Err(OsError::Failed("notification timed out".into())),
        }
    }

    async fn pick_directory(&self) -> Result<Option<PathBuf>, OsError> {
        for (program, args) in picker_commands() {
            let mut cmd = Command::new(program);
            cmd.args(args).stdin(std::process::Stdio::null()).kill_on_drop(true);
            let out = match tokio::time::timeout(PICKER_TIMEOUT, cmd.output()).await {
                Ok(Ok(out)) => out,
                Ok(Err(e)) if e.kind() == std::io::ErrorKind::NotFound => continue,
                Ok(Err(e)) => return Err(OsError::Failed(format!("{program} failed: {e}"))),
                Err(_) => return Err(OsError::Failed("folder picker timed out".into())),
            };
            return interpret_picker(
                out.status.code(),
                &String::from_utf8_lossy(&out.stdout),
                &String::from_utf8_lossy(&out.stderr),
            );
        }
        Err(OsError::Unsupported)
    }

    async fn open_window(&self, cmd: &[String]) -> Result<(), OsError> {
        spawn_detached(cmd).await
    }

    async fn clipboard_text(&self) -> Result<String, OsError> {
        Err(OsError::Unsupported)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn notify_send_gets_title_and_body_after_a_double_dash() {
        assert_eq!(notify_args("-t", "b"), ["--app-name=claude-garage", "--", "-t", "b"]);
    }

    #[test]
    fn zenity_is_tried_before_kdialog() {
        let cmds = picker_commands();
        assert_eq!(cmds[0].0, "zenity");
        assert!(cmds[0].1.contains(&"--directory".to_string()));
        assert_eq!(cmds[1].0, "kdialog");
    }

    #[test]
    fn picker_exit_codes_map_to_path_cancel_or_error() {
        assert_eq!(interpret_picker(Some(0), "/home/me/proj\n", "").unwrap(), Some(PathBuf::from("/home/me/proj")));
        assert_eq!(interpret_picker(Some(0), "\n", "").unwrap(), None);
        assert_eq!(interpret_picker(Some(1), "", "").unwrap(), None, "cancel");
        assert!(matches!(interpret_picker(Some(1), "", "no display"), Err(OsError::Failed(m)) if m == "no display"));
        assert!(matches!(interpret_picker(None, "", ""), Err(OsError::Failed(_))), "killed by a signal");
        assert!(matches!(interpret_picker(Some(5), "", ""), Err(OsError::Failed(_))));
    }
}
