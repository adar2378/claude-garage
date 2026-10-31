//! Port of `daemon/src/picker.js` (and the OS seam for `notify.js`): everything
//! that differs per operating system, behind one trait. One file per OS behind
//! `cfg` comes with the real implementations; a feature an OS lacks returns
//! `OsError::Unsupported`, which the routes map to 501.

pub mod notify;

use std::path::PathBuf;

use async_trait::async_trait;

#[derive(Debug)]
pub enum OsError {
    Unsupported,
    Failed(String),
}

#[async_trait]
pub trait Os: Send + Sync {
    async fn notify(&self, title: &str, body: &str) -> Result<(), OsError>;
    /// `Ok(None)` when the user cancels the picker.
    async fn pick_directory(&self) -> Result<Option<PathBuf>, OsError>;
    async fn open_window(&self, cmd: &[String]) -> Result<(), OsError>;
    async fn clipboard_text(&self) -> Result<String, OsError>;
}

/// Placeholder until the per-OS implementations land: everything unsupported.
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
