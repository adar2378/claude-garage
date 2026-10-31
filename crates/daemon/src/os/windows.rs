//! Windows: toast notifications and the folder picker through the stock
//! Windows PowerShell (no extra modules). Compiled on every OS so the script
//! builders and result mapping are unit-tested everywhere; `os::platform()`
//! selects it on Windows only. Tests never run PowerShell.
//!
//! Based on the scripts in PR #2 by Sabbir Hossain Khan.

use std::path::PathBuf;
use std::time::Duration;

use async_trait::async_trait;
use tokio::process::Command;

use super::{Os, OsError, spawn_detached};

/// The folder dialog blocks while it is open; same allowance as macOS.
pub const PICKER_TIMEOUT: Duration = Duration::from_secs(120);

/// The stock WinForms folder dialog. Prints the path, or nothing on cancel.
pub const PICKER_SCRIPT: &str = "Add-Type -AssemblyName System.Windows.Forms; \
$f = New-Object System.Windows.Forms.FolderBrowserDialog; \
$f.Description = 'Add claude-garage workspace'; \
$o = New-Object System.Windows.Forms.Form; $o.TopMost = $true; \
if ($f.ShowDialog($o) -eq 'OK') { $f.SelectedPath }";

/// Double single quotes: the only escape inside a PowerShell single-quoted string.
fn ps_quote(s: &str) -> String {
    s.replace('\'', "''")
}

/// A toast through the WinRT notification API, shown under Windows
/// PowerShell's own app id.
pub fn toast_script(title: &str, body: &str) -> String {
    format!(
        "[Windows.UI.Notifications.ToastNotificationManager, Windows.UI.Notifications, ContentType = WindowsRuntime] > $null; \
$t = [Windows.UI.Notifications.ToastNotificationManager]::GetTemplateContent([Windows.UI.Notifications.ToastTemplateType]::ToastText02); \
$n = $t.GetElementsByTagName('text'); \
$n.Item(0).AppendChild($t.CreateTextNode('{}')) > $null; \
$n.Item(1).AppendChild($t.CreateTextNode('{}')) > $null; \
$app = '{{1AC14E77-02E7-4E5D-B744-2EB1AE5198B7}}\\WindowsPowerShell\\v1.0\\powershell.exe'; \
[Windows.UI.Notifications.ToastNotificationManager]::CreateToastNotifier($app).Show([Windows.UI.Notifications.ToastNotification]::new($t))",
        ps_quote(title),
        ps_quote(body)
    )
}

/// Standard base64 (RFC 4648, padded).
fn base64(bytes: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for c in bytes.chunks(3) {
        let n = (u32::from(c[0]) << 16)
            | (u32::from(*c.get(1).unwrap_or(&0)) << 8)
            | u32::from(*c.get(2).unwrap_or(&0));
        out.push(T[(n >> 18) as usize & 63] as char);
        out.push(T[(n >> 12) as usize & 63] as char);
        out.push(if c.len() > 1 {
            T[(n >> 6) as usize & 63] as char
        } else {
            '='
        });
        out.push(if c.len() > 2 {
            T[n as usize & 63] as char
        } else {
            '='
        });
    }
    out
}

/// What `-EncodedCommand` takes: the script as UTF-16LE, base64-encoded. The
/// script then never passes through command-line quoting, so no character in
/// a notification title or body can change what PowerShell parses.
pub fn encode_command(script: &str) -> String {
    base64(
        &script
            .encode_utf16()
            .flat_map(u16::to_le_bytes)
            .collect::<Vec<u8>>(),
    )
}

/// argv (after `powershell.exe`) that runs `script` without a profile or prompt.
pub fn powershell_args(script: &str, sta: bool) -> Vec<String> {
    let mut a: Vec<String> = ["-NoProfile", "-NonInteractive"].map(String::from).to_vec();
    if sta {
        a.push("-STA".into());
    }
    a.extend(["-EncodedCommand".to_owned(), encode_command(script)]);
    a
}

/// The picker prints the chosen path; nothing printed means the dialog was cancelled.
pub fn interpret_picker(
    success: bool,
    stdout: &str,
    stderr: &str,
) -> Result<Option<PathBuf>, OsError> {
    if !success {
        let detail = stderr.trim();
        return Err(OsError::Failed(if detail.is_empty() {
            "folder picker failed".into()
        } else {
            detail.to_owned()
        }));
    }
    let dir = stdout.trim();
    Ok((!dir.is_empty()).then(|| PathBuf::from(dir)))
}

fn powershell(args: Vec<String>) -> Command {
    let mut cmd = Command::new("powershell.exe");
    cmd.args(args)
        .stdin(std::process::Stdio::null())
        .kill_on_drop(true);
    #[cfg(windows)]
    {
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    cmd
}

pub struct WindowsOs;

#[async_trait]
impl Os for WindowsOs {
    async fn notify(&self, title: &str, body: &str) -> Result<(), OsError> {
        let mut cmd = powershell(powershell_args(&toast_script(title, body), false));
        cmd.stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null());
        match tokio::time::timeout(Duration::from_secs(10), cmd.status()).await {
            Ok(Ok(_)) => Ok(()),
            Ok(Err(e)) => Err(OsError::Failed(e.to_string())),
            Err(_) => Err(OsError::Failed("notification timed out".into())),
        }
    }

    async fn pick_directory(&self) -> Result<Option<PathBuf>, OsError> {
        let mut cmd = powershell(powershell_args(PICKER_SCRIPT, true));
        let out = match tokio::time::timeout(PICKER_TIMEOUT, cmd.output()).await {
            Ok(Ok(out)) => out,
            Ok(Err(e)) => return Err(OsError::Failed(format!("powershell failed: {e}"))),
            Err(_) => return Err(OsError::Failed("folder picker timed out".into())),
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
        Err(OsError::Unsupported)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn toast_text_is_quoted_so_a_quote_cannot_end_the_string() {
        let s = toast_script("claude-garage", "it's 'x'; Remove-Item *");
        assert!(
            s.contains("CreateTextNode('it''s ''x''; Remove-Item *')"),
            "{s}"
        );
        assert!(s.contains("CreateTextNode('claude-garage')"));
        assert!(s.contains(
            "{1AC14E77-02E7-4E5D-B744-2EB1AE5198B7}\\WindowsPowerShell\\v1.0\\powershell.exe"
        ));
    }

    #[test]
    fn powershell_runs_without_profile_and_the_picker_needs_sta() {
        assert_eq!(
            powershell_args("x", false),
            ["-NoProfile", "-NonInteractive", "-EncodedCommand", "eAA="]
        );
        assert_eq!(
            powershell_args("x", true),
            [
                "-NoProfile",
                "-NonInteractive",
                "-STA",
                "-EncodedCommand",
                "eAA="
            ]
        );
    }

    #[test]
    fn encoded_commands_are_base64_of_utf16le() {
        // Known vectors: UTF-16LE bytes of "hi" are 68 00 69 00.
        assert_eq!(encode_command("hi"), "aABpAA==");
        assert_eq!(encode_command(""), "");
        assert_eq!(base64(b"f"), "Zg==");
        assert_eq!(base64(b"fo"), "Zm8=");
        assert_eq!(base64(b"foo"), "Zm9v");
        // Non-BMP text becomes a surrogate pair.
        assert_eq!(encode_command("\u{1F600}"), "PdgA3g==");
        // No quoting survives into argv: a hostile body is only inside the base64 blob.
        let a = powershell_args(&toast_script("t", "'; calc; '"), false);
        assert!(a.iter().all(|x| !x.contains("calc")), "{a:?}");
    }

    #[test]
    fn picker_output_maps_to_a_path_a_cancel_or_an_error() {
        assert_eq!(
            interpret_picker(true, "C:\\Users\\me\\proj\r\n", "").unwrap(),
            Some(PathBuf::from("C:\\Users\\me\\proj"))
        );
        assert_eq!(interpret_picker(true, "\r\n", "").unwrap(), None);
        assert!(
            matches!(interpret_picker(false, "", "boom\n"), Err(OsError::Failed(m)) if m == "boom")
        );
        assert!(
            matches!(interpret_picker(false, "", ""), Err(OsError::Failed(m)) if m == "folder picker failed")
        );
    }
}
