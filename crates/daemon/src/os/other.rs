//! Every OS but macOS. The JS daemon's notifications and folder picker were
//! darwin-only (a no-op / 501 elsewhere), so they stay `Unsupported`; the
//! editor launch was never OS-specific and works everywhere.

use std::path::PathBuf;

use async_trait::async_trait;

use super::{spawn_detached, Os, OsError};

pub struct OtherOs;

#[async_trait]
impl Os for OtherOs {
    async fn notify(&self, _: &str, _: &str) -> Result<(), OsError> {
        Err(OsError::Unsupported)
    }
    async fn pick_directory(&self) -> Result<Option<PathBuf>, OsError> {
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

    #[tokio::test]
    async fn notifications_picker_and_clipboard_are_unsupported() {
        let os = OtherOs;
        assert!(matches!(os.notify("t", "b").await, Err(OsError::Unsupported)));
        assert!(matches!(os.pick_directory().await, Err(OsError::Unsupported)));
        assert!(matches!(os.clipboard_text().await, Err(OsError::Unsupported)));
    }
}
