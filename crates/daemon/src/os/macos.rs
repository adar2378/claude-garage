//! macOS: exactly what `picker.js` and `notify.js` do today. Compiled on every
//! OS (it only spawns processes) so its pure pieces are unit-tested on CI
//! everywhere; `os::platform()` selects it on macOS only. Tests never call
//! the `Os` methods: they would open a dialog or post a notification.

use std::path::PathBuf;
use std::time::Duration;

use async_trait::async_trait;
use tokio::process::Command;
use tokio::sync::OnceCell;

use super::{spawn_detached, Os, OsError};

/// Browsers never expose absolute paths, so the "add workspace" picker runs
/// daemon-side.
pub const PICKER_SCRIPT: &str = r#"POSIX path of (choose folder with prompt "Add claude-garage workspace")"#;

/// osascript blocks while the dialog is open (the user may leave it sitting
/// there); a generous timeout instead of failing an interaction in progress.
pub const PICKER_TIMEOUT: Duration = Duration::from_secs(120);

/// terminal-notifier when installed (`brew install terminal-notifier`),
/// osascript's `display notification` otherwise. Neither opens anything on click.
pub fn notification_command(has_terminal_notifier: bool, title: &str, body: &str) -> (String, Vec<String>) {
    if has_terminal_notifier {
        ("terminal-notifier".into(), vec!["-title".into(), title.into(), "-message".into(), body.into()])
    } else {
        // JSON string syntax doubles as AppleScript string syntax (JS used JSON.stringify).
        let script = format!(
            "display notification {} with title {}",
            serde_json::to_string(body).unwrap(),
            serde_json::to_string(title).unwrap()
        );
        ("osascript".into(), vec!["-e".into(), script])
    }
}

/// Reads the outcome of the picker's osascript run. A dismissed dialog exits 1
/// with "User canceled" in stderr: a normal outcome, not an error. On success
/// the trailing newline is dropped (the route strips the trailing `/`).
pub fn interpret_picker(success: bool, stdout: &str, stderr: &str) -> Result<Option<PathBuf>, OsError> {
    if success {
        return Ok(Some(PathBuf::from(stdout.trim_end())));
    }
    if stderr.contains("User canceled") {
        return Ok(None);
    }
    let detail = stderr.trim();
    Err(OsError::Failed(if detail.is_empty() { "osascript failed".into() } else { detail.to_owned() }))
}

#[derive(Default)]
pub struct MacOs {
    /// `which terminal-notifier`, probed once, lazily.
    terminal_notifier: OnceCell<bool>,
}

impl MacOs {
    async fn has_terminal_notifier(&self) -> bool {
        *self
            .terminal_notifier
            .get_or_init(|| async {
                Command::new("which")
                    .arg("terminal-notifier")
                    .stdin(std::process::Stdio::null())
                    .stdout(std::process::Stdio::null())
                    .stderr(std::process::Stdio::null())
                    .status()
                    .await
                    .map(|s| s.success())
                    .unwrap_or(false)
            })
            .await
    }
}

#[async_trait]
impl Os for MacOs {
    async fn notify(&self, title: &str, body: &str) -> Result<(), OsError> {
        let (program, args) = notification_command(self.has_terminal_notifier().await, title, body);
        let mut cmd = Command::new(program);
        cmd.args(args)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .kill_on_drop(true);
        let run = async { cmd.status().await.map_err(|e| OsError::Failed(e.to_string())) };
        // A hung notifier must not pile up; JS had no bound, 10 s is generous.
        match tokio::time::timeout(Duration::from_secs(10), run).await {
            Ok(status) => status.map(|_| ()),
            Err(_) => Err(OsError::Failed("notification timed out".into())),
        }
    }

    async fn pick_directory(&self) -> Result<Option<PathBuf>, OsError> {
        let mut cmd = Command::new("osascript");
        cmd.args(["-e", PICKER_SCRIPT]).stdin(std::process::Stdio::null()).kill_on_drop(true);
        let out = match tokio::time::timeout(PICKER_TIMEOUT, cmd.output()).await {
            Ok(Ok(out)) => out,
            Ok(Err(e)) => return Err(OsError::Failed(format!("osascript failed: {e}"))),
            Err(_) => return Err(OsError::Failed("osascript timed out".into())),
        };
        interpret_picker(
            out.status.success(),
            &String::from_utf8_lossy(&out.stdout),
            &String::from_utf8_lossy(&out.stderr),
        )
    }

    async fn open_window(&self, cmd: &[String]) -> Result<(), OsError> {
        spawn_detached(cmd).await
    }

    async fn clipboard_text(&self) -> Result<String, OsError> {
        // Nothing in the JS daemon reads the clipboard; arrives with p26.
        Err(OsError::Unsupported)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn terminal_notifier_is_preferred_and_gets_title_and_message() {
        let (p, a) = notification_command(true, "claude-garage", "garage/ws/a needs input");
        assert_eq!(p, "terminal-notifier");
        assert_eq!(a, ["-title", "claude-garage", "-message", "garage/ws/a needs input"]);
    }

    #[test]
    fn osascript_fallback_escapes_the_message_as_json_does() {
        let (p, a) = notification_command(false, "claude-garage", r#"garage/ws/a "q" needs input"#);
        assert_eq!(p, "osascript");
        assert_eq!(a[0], "-e");
        assert_eq!(a[1], r#"display notification "garage/ws/a \"q\" needs input" with title "claude-garage""#);
    }

    #[test]
    fn picker_script_is_the_choose_folder_prompt() {
        assert_eq!(PICKER_SCRIPT, r#"POSIX path of (choose folder with prompt "Add claude-garage workspace")"#);
    }

    #[test]
    fn picker_success_trims_the_newline_and_user_canceled_is_not_an_error() {
        let ok = interpret_picker(true, "/Users/me/proj/\n", "").unwrap();
        assert_eq!(ok, Some(PathBuf::from("/Users/me/proj/")));
        let cancelled = interpret_picker(false, "", "execution error: User canceled. (-128)").unwrap();
        assert_eq!(cancelled, None);
    }

    #[test]
    fn picker_other_failures_carry_stderr_or_a_default_message() {
        match interpret_picker(false, "", "boom\n") {
            Err(OsError::Failed(m)) => assert_eq!(m, "boom"),
            other => panic!("{other:?}"),
        }
        match interpret_picker(false, "", "") {
            Err(OsError::Failed(m)) => assert_eq!(m, "osascript failed"),
            other => panic!("{other:?}"),
        }
    }
}
