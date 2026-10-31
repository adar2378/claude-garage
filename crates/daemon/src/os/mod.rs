//! Port of `daemon/src/picker.js` (and the OS seam for `notify.js`): everything
//! that differs per operating system, behind one trait. A feature an OS lacks
//! returns `OsError::Unsupported`, which the routes map to 501. `macos` holds
//! today's darwin-only behaviour (osascript, terminal-notifier); `other` is
//! every other OS. `linux` (notify-send, zenity/kdialog) and `windows`
//! (PowerShell toast and folder dialog) are compiled everywhere so their pure
//! pieces are tested on every CI OS; `platform()` picks one. Anything an OS
//! cannot do stays `Unsupported`.

pub mod linux;
pub mod macos;
pub mod notify;
pub mod other;
pub mod process;
pub mod windows;

use std::path::PathBuf;
use std::sync::Arc;

use async_trait::async_trait;

#[derive(Debug)]
pub enum OsError {
    Unsupported,
    /// The program to run does not exist (spawn `ENOENT`).
    NotFound,
    Failed(String),
}

#[async_trait]
pub trait Os: Send + Sync {
    async fn notify(&self, title: &str, body: &str) -> Result<(), OsError>;
    /// `Ok(None)` when the user cancels the picker.
    async fn pick_directory(&self) -> Result<Option<PathBuf>, OsError>;
    /// Start `cmd` (program first) detached, stdio ignored, and return once it
    /// has actually spawned: `NotFound` when the program is missing.
    async fn open_window(&self, cmd: &[String]) -> Result<(), OsError>;
    async fn clipboard_text(&self) -> Result<String, OsError>;
}

/// The `Os` for the platform this binary was built for.
pub fn platform() -> Arc<dyn Os> {
    #[cfg(target_os = "macos")]
    {
        Arc::new(macos::MacOs::default())
    }
    #[cfg(target_os = "linux")]
    {
        Arc::new(linux::LinuxOs)
    }
    #[cfg(windows)]
    {
        Arc::new(windows::WindowsOs)
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux", windows)))]
    {
        Arc::new(other::OtherOs)
    }
}

/// Fire-and-forget GUI/CLI launch (JS `spawn(..., {stdio:"ignore", detached:true})`
/// plus `unref`). `process_group(0)` detaches from the daemon's terminal
/// signals; unlike JS's `setsid` the child stays in the daemon's session, which
/// is enough here (the daemon itself runs session-less). tokio reaps the child
/// when it exits.
pub async fn spawn_detached(cmd: &[String]) -> Result<(), OsError> {
    if cmd.is_empty() {
        return Err(OsError::Failed("empty command".into()));
    }
    // Windows tools are often `.cmd` shims (code.cmd) that CreateProcess
    // cannot start by bare name; resolve them the way the session host does.
    #[cfg(windows)]
    let resolved = garage_host::server::resolve_command(cmd);
    #[cfg(windows)]
    let cmd = &resolved[..];
    let (program, args) = cmd.split_first().expect("checked non-empty");
    let mut c = tokio::process::Command::new(program);
    c.args(args)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .kill_on_drop(false);
    #[cfg(unix)]
    c.process_group(0);
    #[cfg(windows)]
    {
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        c.creation_flags(CREATE_NO_WINDOW);
    }
    match c.spawn() {
        Ok(_child) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Err(OsError::NotFound),
        Err(e) => Err(OsError::Failed(e.to_string())),
    }
}

/// `POST /api/pick-directory` strips one trailing `/` from a folder path
/// (`POSIX path of` ends folders with one), except the root `/` itself.
pub fn strip_trailing_slash(path: &str) -> String {
    match path.strip_suffix('/') {
        Some(rest) if path.len() > 1 => rest.to_owned(),
        _ => path.to_owned(),
    }
}

/// Placeholder/default: everything unsupported.
pub struct UnsupportedOs;

#[async_trait]
impl Os for UnsupportedOs {
    async fn notify(&self, _: &str, _: &str) -> Result<(), OsError> {
        Err(OsError::Unsupported)
    }
    async fn pick_directory(&self) -> Result<Option<PathBuf>, OsError> {
        Err(OsError::Unsupported)
    }
    async fn open_window(&self, _: &[String]) -> Result<(), OsError> {
        Err(OsError::Unsupported)
    }
    async fn clipboard_text(&self) -> Result<String, OsError> {
        Err(OsError::Unsupported)
    }
}

/// Recording fake: tests inject this so no notification, picker, editor or
/// other GUI program is ever really started.
#[cfg(test)]
pub mod fake {
    use std::sync::Mutex;

    use super::*;

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub enum Mode {
        Ok,
        Unsupported,
        NotFound,
        Failed,
    }

    pub struct FakeOs {
        pub mode: Mutex<Mode>,
        /// What `pick_directory` answers (`None` = the user cancelled).
        pub pick: Mutex<Option<PathBuf>>,
        notified: Mutex<Vec<(String, String)>>,
        opened: Mutex<Vec<Vec<String>>>,
    }

    impl Default for FakeOs {
        fn default() -> Self {
            Self {
                mode: Mutex::new(Mode::Ok),
                pick: Mutex::new(None),
                notified: Mutex::default(),
                opened: Mutex::default(),
            }
        }
    }

    impl FakeOs {
        pub fn with_mode(mode: Mode) -> Self {
            let o = Self::default();
            *o.mode.lock().unwrap() = mode;
            o
        }
        pub fn unsupported() -> Self {
            Self::with_mode(Mode::Unsupported)
        }
        pub fn notifications(&self) -> Vec<(String, String)> {
            self.notified.lock().unwrap().clone()
        }
        pub fn opened(&self) -> Vec<Vec<String>> {
            self.opened.lock().unwrap().clone()
        }
        fn check(&self) -> Result<(), OsError> {
            match *self.mode.lock().unwrap() {
                Mode::Ok => Ok(()),
                Mode::Unsupported => Err(OsError::Unsupported),
                Mode::NotFound => Err(OsError::NotFound),
                Mode::Failed => Err(OsError::Failed("fake failure".into())),
            }
        }
    }

    #[async_trait]
    impl Os for FakeOs {
        async fn notify(&self, title: &str, body: &str) -> Result<(), OsError> {
            self.notified
                .lock()
                .unwrap()
                .push((title.into(), body.into()));
            self.check()
        }
        async fn pick_directory(&self) -> Result<Option<PathBuf>, OsError> {
            self.check()?;
            Ok(self.pick.lock().unwrap().clone())
        }
        async fn open_window(&self, cmd: &[String]) -> Result<(), OsError> {
            self.opened.lock().unwrap().push(cmd.to_vec());
            self.check()
        }
        async fn clipboard_text(&self) -> Result<String, OsError> {
            Err(OsError::Unsupported)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trailing_slash_is_stripped_except_for_the_root() {
        assert_eq!(strip_trailing_slash("/Users/me/proj/"), "/Users/me/proj");
        assert_eq!(strip_trailing_slash("/Users/me/proj"), "/Users/me/proj");
        assert_eq!(strip_trailing_slash("/"), "/");
        assert_eq!(strip_trailing_slash(""), "");
    }

    #[tokio::test]
    async fn spawn_detached_reports_a_missing_program_as_not_found() {
        let e = spawn_detached(&["/nonexistent/garage-editor-for-tests".into()])
            .await
            .unwrap_err();
        assert!(matches!(e, OsError::NotFound), "{e:?}");
        assert!(matches!(spawn_detached(&[]).await, Err(OsError::Failed(_))));
    }

    #[tokio::test]
    async fn spawn_detached_starts_a_harmless_program() {
        spawn_detached(&["true".into()]).await.unwrap();
    }
}
