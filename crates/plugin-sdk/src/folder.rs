//! Turn the word a user typed into an existing folder: `~`, `~/x`, `~user`,
//! `~user/x`, a relative path (against the caller's working directory) or an
//! absolute one. The result is canonical, so a symlinked spelling and the real
//! one name the same folder.
//!
//! Used by `claude-garage <folder>` in the launcher (to decide, before any
//! daemon starts, whether the word is a folder at all) and by the cli-extras
//! plugin (to register it).

use std::path::{Path, PathBuf};

/// Why a typed word is not a usable folder.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FolderError {
    /// Not an existing directory (a typo, a file, a dangling link).
    NotADirectory,
    /// `~name` for a user the system does not know.
    UnknownUser(String),
    /// `~` with no home directory to expand it to.
    NoHome,
    /// The folder's real path is not valid UTF-8, and a project's directory is
    /// stored as text. The text is the path with the bad bytes replaced.
    NotUtf8(String),
}

impl std::fmt::Display for FolderError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FolderError::NotADirectory => write!(f, "not a directory"),
            FolderError::UnknownUser(u) => write!(f, "no such user: {u}"),
            FolderError::NoHome => write!(f, "cannot expand ~: no home directory"),
            FolderError::NotUtf8(p) => write!(
                f,
                "the folder's path is not valid UTF-8 and claude-garage cannot store it: {p}"
            ),
        }
    }
}

impl std::error::Error for FolderError {}

/// The home directory of a named user, from the system's user database.
#[cfg(unix)]
pub fn user_home(name: &str) -> Option<PathBuf> {
    use uzers::os::unix::UserExt;
    uzers::get_user_by_name(name).map(|u| u.home_dir().to_path_buf())
}

#[cfg(not(unix))]
pub fn user_home(_name: &str) -> Option<PathBuf> {
    None
}

/// [`resolve_folder_with`] using the system's user database.
pub fn resolve_folder(word: &str, cwd: &Path, home: Option<&Path>) -> Result<PathBuf, FolderError> {
    resolve_folder_with(word, cwd, home, user_home)
}

/// `home` is the current user's home (`$HOME`); `lookup` finds another user's.
pub fn resolve_folder_with(
    word: &str,
    cwd: &Path,
    home: Option<&Path>,
    lookup: impl Fn(&str) -> Option<PathBuf>,
) -> Result<PathBuf, FolderError> {
    let path = match word.strip_prefix('~') {
        None => cwd.join(word),
        Some(rest) => {
            let (user, tail) = match rest.split_once('/') {
                Some((u, t)) => (u, t),
                None => (rest, ""),
            };
            let base = if user.is_empty() {
                home.ok_or(FolderError::NoHome)?.to_path_buf()
            } else {
                lookup(user).ok_or_else(|| FolderError::UnknownUser(user.to_owned()))?
            };
            if tail.is_empty() {
                base
            } else {
                base.join(tail)
            }
        }
    };
    let path = std::fs::canonicalize(path).map_err(|_| FolderError::NotADirectory)?;
    if !path.is_dir() {
        return Err(FolderError::NotADirectory);
    }
    utf8_or_refuse(path)
}

fn utf8_or_refuse(path: PathBuf) -> Result<PathBuf, FolderError> {
    match path.to_str() {
        Some(_) => Ok(path),
        None => Err(FolderError::NotUtf8(path.to_string_lossy().into_owned())),
    }
}

/// The canonical spelling of a stored directory for comparing two folders:
/// the real path when it exists, else the text with trailing slashes removed.
pub fn canonical_text(dir: &str) -> String {
    match std::fs::canonicalize(dir)
        .ok()
        .and_then(|p| p.to_str().map(str::to_owned))
    {
        Some(p) => p,
        None if dir.len() > 1 => dir.trim_end_matches('/').to_owned(),
        None => dir.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn no_users(_: &str) -> Option<PathBuf> {
        None
    }

    #[test]
    fn tilde_forms_relative_and_absolute_paths_resolve_to_the_canonical_folder() {
        let root = tempfile::tempdir().unwrap();
        let home = root.path().join("home/me");
        let other = root.path().join("home/ann");
        std::fs::create_dir_all(home.join("dev/app")).unwrap();
        std::fs::create_dir_all(other.join("shared")).unwrap();
        std::fs::write(home.join("dev/file.txt"), "x").unwrap();
        let canon = |p: &Path| std::fs::canonicalize(p).unwrap();
        let lookup = |u: &str| (u == "ann").then(|| other.clone());
        let cwd = home.join("dev");
        let r = |w: &str| resolve_folder_with(w, &cwd, Some(&home), lookup);

        assert_eq!(r("~/dev/app"), Ok(canon(&home.join("dev/app"))));
        assert_eq!(r("~"), Ok(canon(&home)));
        assert_eq!(r("~ann"), Ok(canon(&other)));
        assert_eq!(r("~ann/shared"), Ok(canon(&other.join("shared"))));
        assert_eq!(r("app"), Ok(canon(&home.join("dev/app"))));
        assert_eq!(r("./app/"), Ok(canon(&home.join("dev/app"))));
        let abs = home.join("dev/app").to_string_lossy().into_owned();
        assert_eq!(r(&abs), Ok(canon(&home.join("dev/app"))));

        assert_eq!(r("file.txt"), Err(FolderError::NotADirectory));
        assert_eq!(r("typo"), Err(FolderError::NotADirectory));
        assert_eq!(r("~ann/missing"), Err(FolderError::NotADirectory));
        assert_eq!(r("~bob"), Err(FolderError::UnknownUser("bob".into())));
        assert_eq!(
            resolve_folder_with("~", &cwd, None, no_users),
            Err(FolderError::NoHome)
        );
        assert_eq!(
            resolve_folder_with("~/dev", &cwd, None, no_users),
            Err(FolderError::NoHome),
            "no HOME, no tilde"
        );
    }

    #[cfg(unix)]
    #[test]
    fn the_system_user_database_backs_tilde_name() {
        assert_eq!(
            resolve_folder("~no-such-user-xyz", Path::new("/"), None),
            Err(FolderError::UnknownUser("no-such-user-xyz".into()))
        );
        // root exists everywhere this runs; its home may not be readable, but
        // the lookup itself must find it
        assert!(user_home("root").is_some());
    }

    #[cfg(unix)]
    #[test]
    fn a_symlinked_spelling_resolves_to_the_real_folder() {
        let root = tempfile::tempdir().unwrap();
        let real = root.path().join("real");
        std::fs::create_dir(&real).unwrap();
        std::os::unix::fs::symlink(&real, root.path().join("link")).unwrap();
        let via_link = resolve_folder("link", root.path(), None).unwrap();
        let direct = resolve_folder("real", root.path(), None).unwrap();
        assert_eq!(via_link, direct);
        assert_eq!(
            canonical_text(&root.path().join("link").to_string_lossy()),
            canonical_text(&root.path().join("real/").to_string_lossy()),
            "stored spellings compare equal"
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_path_that_is_not_utf8_is_refused_with_a_message_that_shows_it() {
        use std::os::unix::ffi::OsStrExt;
        // built from bytes: no file system has to accept the name
        let bad = PathBuf::from(std::ffi::OsStr::from_bytes(b"/work/caf\xe9"));
        let err = utf8_or_refuse(bad).unwrap_err();
        let FolderError::NotUtf8(shown) = &err else {
            panic!("{err:?}")
        };
        assert!(shown.starts_with("/work/caf"), "{shown}");
        assert!(err.to_string().contains("not valid UTF-8"), "{err}");
        assert!(utf8_or_refuse(PathBuf::from("/work/café")).is_ok());
    }

    #[test]
    fn canonical_text_falls_back_to_the_text_for_a_folder_that_is_gone() {
        assert_eq!(
            canonical_text("/definitely/not/here/"),
            "/definitely/not/here"
        );
        assert_eq!(canonical_text("/"), "/");
    }
}
