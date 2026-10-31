//! Draft and pasted-screenshot recovery (p25): keep what a user typed into a
//! tile's prompt but has not sent, and image files pasted into it, so a
//! crash or power cut does not lose them.
//!
//! Core, not a plugin: reading a prompt means reading every session's
//! screen, and the files can hold anything the user typed. Everything here is
//! therefore small and bounded, private to the user (directory 0700, files
//! 0600 on Unix) and only ever for session tiles; plugin panes never reach
//! this module.
//!
//! The text comes from the tile's screen (the rows of Claude Code's prompt
//! box), not from a keystroke log: arrow keys, history recall and edits all
//! come out right, and nothing is kept for a prompt that is not on screen.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

/// Longest draft kept (bytes). A bigger one is cut on a character boundary.
pub const MAX_TEXT_BYTES: usize = 64 * 1024;
/// Most images kept per session.
pub const MAX_IMAGES: usize = 8;
/// Largest single image kept (bytes).
pub const MAX_IMAGE_BYTES: u64 = 20 * 1024 * 1024;
/// Files older than this are removed at startup.
pub const MAX_AGE: Duration = Duration::from_secs(30 * 24 * 3600);

const IMAGE_EXTS: [&str; 5] = ["png", "jpg", "jpeg", "gif", "webp"];

// ── reading the prompt off a screen ────────────────────────────────────

/// What a tile's screen shows of Claude Code's prompt box.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Prompt {
    /// The user has typed this and not sent it.
    Text(String),
    /// A prompt box is showing and holds nothing typed (an empty box or the
    /// dim "Try ..." suggestion).
    Empty,
    /// No prompt box to read: a dialog, startup, a shell. Nothing may be
    /// concluded from it, least of all that a draft was sent.
    Unknown,
}

fn is_rule(line: &str) -> bool {
    let t = line.trim();
    t.chars().count() >= 10 && t.chars().all(|c| c == '─')
}

pub fn read_prompt(screen: &vt100::Screen) -> Prompt {
    let (rows, cols) = screen.size();
    let lines: Vec<String> = screen.rows(0, cols).collect();
    let rules: Vec<usize> = lines
        .iter()
        .enumerate()
        .filter(|(_, l)| is_rule(l))
        .map(|(i, _)| i)
        .collect();
    // The box is the last pair of rules with at least one row between them.
    let Some((a, b)) = rules
        .windows(2)
        .rev()
        .map(|w| (w[0], w[1]))
        .find(|(a, b)| b - a >= 2)
    else {
        return Prompt::Unknown;
    };
    let mut out: Vec<String> = Vec::new();
    for (r, line) in lines.iter().enumerate().take(b).skip(a + 1) {
        let Ok(row) = u16::try_from(r) else {
            return Prompt::Unknown;
        };
        if row >= rows {
            break;
        }
        let body: &str = if r == a + 1 {
            match line.strip_prefix('❯').or_else(|| line.strip_prefix('>')) {
                Some(rest) => rest.trim_start_matches(' '),
                None => return Prompt::Unknown, // not a prompt line
            }
        } else {
            line.strip_prefix("  ").unwrap_or(line)
        };
        if body.trim().is_empty() {
            out.push(String::new());
            continue;
        }
        // A suggestion is drawn dim; dim text on the row means the user has
        // not typed here.
        let dim = (0..cols).any(|c| {
            screen
                .cell(row, c)
                .is_some_and(|cell| !cell.contents().trim().is_empty() && cell.dim())
        });
        if dim || body.trim_start().starts_with("Try \"") {
            return Prompt::Empty;
        }
        out.push(body.trim_end().to_owned());
    }
    let text = out.join("\n").trim().to_owned();
    if text.is_empty() {
        Prompt::Empty
    } else {
        Prompt::Text(text)
    }
}

/// `text` with Claude Code's `[Image #N]` placeholders removed (the images
/// come back as files, which Claude numbers afresh).
pub fn strip_image_placeholders(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(i) = rest.find("[Image #") {
        out.push_str(&rest[..i]);
        let tail = &rest[i + "[Image #".len()..];
        let digits = tail.chars().take_while(char::is_ascii_digit).count();
        if digits > 0 && tail[digits..].starts_with(']') {
            rest = &tail[digits + 1..];
        } else {
            out.push_str("[Image #");
            rest = tail;
        }
    }
    out.push_str(rest);
    out.trim().to_owned()
}

// ── pasted file paths ──────────────────────────────────────────────────

fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%'
            && let Some(hex) = s.get(i + 1..i + 3)
            && let Ok(v) = u8::from_str_radix(hex, 16)
        {
            out.push(v);
            i += 3;
        } else {
            out.push(b[i]);
            i += 1;
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// The image files a bracketed paste names: one absolute path per line,
/// possibly quoted, backslash-escaped (Finder and terminals escape spaces)
/// or a `file://` URL. Only paths that are existing image files count.
pub fn pasted_image_paths(text: &str) -> Vec<PathBuf> {
    text.lines()
        .filter_map(|line| {
            let mut s = line.trim().to_owned();
            if s.len() >= 2
                && ((s.starts_with('\'') && s.ends_with('\''))
                    || (s.starts_with('"') && s.ends_with('"')))
            {
                s = s[1..s.len() - 1].to_owned();
            }
            if let Some(rest) = s.strip_prefix("file://") {
                s = percent_decode(rest);
            }
            let mut unescaped = String::with_capacity(s.len());
            let mut chars = s.chars();
            while let Some(c) = chars.next() {
                if c == '\\' && cfg!(unix) {
                    if let Some(n) = chars.next() {
                        unescaped.push(n);
                    }
                } else {
                    unescaped.push(c);
                }
            }
            let path = PathBuf::from(unescaped);
            let ext = path.extension()?.to_str()?.to_ascii_lowercase();
            (path.is_absolute() && IMAGE_EXTS.contains(&ext.as_str()) && path.is_file())
                .then_some(path)
        })
        .collect()
}

// ── the store ──────────────────────────────────────────────────────────

/// Per-session draft files under one directory (`<GARAGE_DIR>/drafts`).
#[derive(Debug, Clone)]
pub struct DraftStore {
    root: PathBuf,
}

/// A file name for a session id: letters, digits, `-`, `_` stay; anything
/// else (including `/`, and a leading `.`) becomes `%XX`, so ids cannot
/// collide or climb out of the directory.
pub fn stem(id: &str) -> String {
    let mut out = String::with_capacity(id.len());
    for (i, b) in id.bytes().enumerate() {
        let keep = b.is_ascii_alphanumeric() || b == b'-' || b == b'_' || (b == b'.' && i > 0);
        if keep {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

#[cfg(unix)]
fn private_dir(path: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
    fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(path)?;
    // `recursive` leaves an existing directory's mode alone: tighten it.
    fs::set_permissions(path, fs::Permissions::from_mode(0o700))
}

#[cfg(not(unix))]
fn private_dir(path: &Path) -> std::io::Result<()> {
    fs::create_dir_all(path)
}

fn private_file(path: &Path) -> std::io::Result<fs::File> {
    let mut o = fs::OpenOptions::new();
    o.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        o.mode(0o600);
    }
    o.open(path)
}

impl DraftStore {
    pub fn new(root: PathBuf) -> DraftStore {
        DraftStore { root }
    }

    fn text_path(&self, id: &str) -> PathBuf {
        self.root.join(format!("{}.draft", stem(id)))
    }

    fn images_dir(&self, id: &str) -> PathBuf {
        self.root.join(format!("{}.images", stem(id)))
    }

    /// Keep `text` as the session's draft (atomic: a crash mid-write leaves
    /// the previous draft). Cut to [`MAX_TEXT_BYTES`].
    pub fn save_text(&self, id: &str, text: &str) -> std::io::Result<()> {
        private_dir(&self.root)?;
        let mut end = text.len().min(MAX_TEXT_BYTES);
        while !text.is_char_boundary(end) {
            end -= 1;
        }
        let path = self.text_path(id);
        let tmp = path.with_extension("draft.tmp");
        let mut f = private_file(&tmp)?;
        f.write_all(&text.as_bytes()[..end])?;
        f.sync_all()?;
        fs::rename(&tmp, &path)
    }

    pub fn load_text(&self, id: &str) -> Option<String> {
        fs::read_to_string(self.text_path(id))
            .ok()
            .filter(|t| !t.trim().is_empty())
    }

    /// Copy a pasted image file into the session's recovery folder. `Ok(None)`
    /// when it was refused (not an image file, too big, folder full).
    pub fn add_image(&self, id: &str, src: &Path) -> std::io::Result<Option<PathBuf>> {
        let ext = src
            .extension()
            .and_then(|e| e.to_str())
            .map(str::to_ascii_lowercase)
            .filter(|e| IMAGE_EXTS.contains(&e.as_str()));
        let (Some(ext), Ok(meta)) = (ext, fs::metadata(src)) else {
            return Ok(None);
        };
        if !meta.is_file() || meta.len() > MAX_IMAGE_BYTES || self.images(id).len() >= MAX_IMAGES {
            return Ok(None);
        }
        let dir = self.images_dir(id);
        private_dir(&self.root)?;
        private_dir(&dir)?;
        let ms = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .map_or(0, |d| d.as_millis());
        // Zero-padded so a plain sort is oldest-first.
        let dest = dir.join(format!("img-{ms:016}-{}.{ext}", self.images(id).len()));
        let mut input = fs::File::open(src)?;
        let mut out = private_file(&dest)?;
        std::io::copy(&mut input, &mut out)?;
        Ok(Some(dest))
    }

    /// The kept images, oldest first.
    pub fn images(&self, id: &str) -> Vec<PathBuf> {
        let Ok(rd) = fs::read_dir(self.images_dir(id)) else {
            return Vec::new();
        };
        let mut v: Vec<PathBuf> = rd.flatten().map(|e| e.path()).collect();
        v.sort();
        v
    }

    pub fn has_anything(&self, id: &str) -> bool {
        self.load_text(id).is_some() || !self.images(id).is_empty()
    }

    /// Forget everything kept for the session.
    pub fn clear(&self, id: &str) {
        let _ = fs::remove_file(self.text_path(id));
        let _ = fs::remove_dir_all(self.images_dir(id));
    }

    /// Remove whatever is older than `max_age` (a session that never came
    /// back). Run once at startup.
    pub fn prune_older_than(&self, max_age: Duration) {
        let Ok(rd) = fs::read_dir(&self.root) else {
            return;
        };
        for entry in rd.flatten() {
            let old = entry
                .metadata()
                .and_then(|m| m.modified())
                .ok()
                .and_then(|t| t.elapsed().ok())
                .is_some_and(|age| age > max_age);
            if old {
                let p = entry.path();
                let _ = fs::remove_file(&p).or_else(|_| fs::remove_dir_all(&p));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn screen_with(prompt_rows: &str) -> vt100::Parser {
        let mut p = vt100::Parser::new(12, 40, 0);
        let rule = "─".repeat(30);
        p.process(format!("some output\r\n{rule}\r\n{prompt_rows}\r\n{rule}\r\nfooter").as_bytes());
        p
    }

    #[test]
    fn typed_text_is_read_off_the_prompt_box() {
        let p = screen_with("❯ hello world");
        assert_eq!(read_prompt(p.screen()), Prompt::Text("hello world".into()));
    }

    #[test]
    fn a_multi_line_draft_keeps_its_line_breaks() {
        let p = screen_with("❯ first line\r\n  second line");
        assert_eq!(
            read_prompt(p.screen()),
            Prompt::Text("first line\nsecond line".into())
        );
    }

    #[test]
    fn an_empty_box_and_a_dim_suggestion_are_empty_not_drafts() {
        assert_eq!(read_prompt(screen_with("❯ ").screen()), Prompt::Empty);
        let p = screen_with("❯ \x1b[2mTry \"fix a bug\"\x1b[0m");
        assert_eq!(read_prompt(p.screen()), Prompt::Empty);
    }

    #[test]
    fn a_screen_without_a_prompt_box_proves_nothing() {
        let mut p = vt100::Parser::new(6, 40, 0);
        p.process(b"$ ls\r\nfile\r\n");
        assert_eq!(read_prompt(p.screen()), Prompt::Unknown);
        // A dialog between rules is not a prompt line either.
        let dialog = screen_with("Do you want to proceed?");
        assert_eq!(read_prompt(dialog.screen()), Prompt::Unknown);
    }

    #[test]
    fn image_placeholders_are_removed_and_lookalikes_kept() {
        assert_eq!(
            strip_image_placeholders("look [Image #1] and [Image #12] here"),
            "look  and  here"
        );
        assert_eq!(
            strip_image_placeholders("[Image #x] [Image #"),
            "[Image #x] [Image #"
        );
    }

    #[test]
    fn pasted_paths_in_every_finder_spelling_resolve_to_image_files() {
        let dir = tempfile::tempdir().unwrap();
        let img = dir.path().join("shot one.png");
        fs::write(&img, b"png").unwrap();
        fs::write(dir.path().join("notes.txt"), b"x").unwrap();
        let p = img.to_str().unwrap();
        let escaped = p.replace(' ', "\\ ");
        let url = format!("file://{}", p.replace(' ', "%20"));
        for form in [
            escaped,
            format!("'{p}'"),
            format!("\"{p}\""),
            url,
            p.to_owned(),
        ] {
            assert_eq!(pasted_image_paths(&form), vec![img.clone()], "{form}");
        }
        let txt = dir.path().join("notes.txt");
        assert!(pasted_image_paths(txt.to_str().unwrap()).is_empty());
        assert!(pasted_image_paths("see /nope/missing.png please").is_empty());
        assert!(pasted_image_paths("plain words").is_empty());
    }

    #[test]
    fn stems_cannot_collide_or_escape_the_folder() {
        assert_eq!(stem("garage/ws/claude-1"), "garage%2Fws%2Fclaude-1");
        assert_ne!(stem("a/b"), stem("a%2Fb"));
        assert_eq!(stem(".."), "%2E.");
        assert!(!stem("../../etc/passwd").contains('/'));
    }

    #[test]
    fn a_draft_round_trips_clears_and_is_cut_when_huge() {
        let root = tempfile::tempdir().unwrap();
        let s = DraftStore::new(root.path().join("drafts"));
        let id = "garage/ws/one";
        assert!(!s.has_anything(id));
        s.save_text(id, "unsent words").unwrap();
        assert_eq!(s.load_text(id).as_deref(), Some("unsent words"));
        s.save_text(id, &"é".repeat(MAX_TEXT_BYTES)).unwrap();
        assert!(s.load_text(id).unwrap().len() <= MAX_TEXT_BYTES);
        s.clear(id);
        assert!(!s.has_anything(id));
        s.clear(id); // clearing nothing is fine
    }

    #[cfg(unix)]
    #[test]
    fn everything_kept_is_private_to_the_user() {
        use std::os::unix::fs::PermissionsExt;
        let root = tempfile::tempdir().unwrap();
        let s = DraftStore::new(root.path().join("drafts"));
        let src = root.path().join("a.png");
        fs::write(&src, b"png").unwrap();
        s.save_text("x", "secret").unwrap();
        let copy = s.add_image("x", &src).unwrap().unwrap();
        let mode = |p: &Path| fs::metadata(p).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode(&root.path().join("drafts")), 0o700);
        assert_eq!(mode(&s.text_path("x")), 0o600);
        assert_eq!(mode(copy.parent().unwrap()), 0o700);
        assert_eq!(mode(&copy), 0o600);
    }

    #[test]
    fn images_are_copied_in_order_and_bounded() {
        let root = tempfile::tempdir().unwrap();
        let s = DraftStore::new(root.path().join("drafts"));
        let src = root.path().join("a.png");
        fs::write(&src, b"png-bytes").unwrap();
        let first = s.add_image("x", &src).unwrap().unwrap();
        assert_eq!(fs::read(&first).unwrap(), b"png-bytes");
        for _ in 1..MAX_IMAGES {
            assert!(s.add_image("x", &src).unwrap().is_some());
        }
        assert_eq!(s.add_image("x", &src).unwrap(), None, "folder is full");
        let all = s.images("x");
        assert_eq!(all.len(), MAX_IMAGES);
        assert_eq!(all[0], first, "oldest first");
        // Not an image, or not there: refused, nothing created.
        let txt = root.path().join("n.txt");
        fs::write(&txt, b"x").unwrap();
        assert_eq!(s.add_image("y", &txt).unwrap(), None);
        assert_eq!(
            s.add_image("y", &root.path().join("gone.png")).unwrap(),
            None
        );
        assert!(!s.has_anything("y"));
        s.clear("x");
        assert!(s.images("x").is_empty());
    }

    #[test]
    fn pruning_removes_only_what_is_old() {
        let root = tempfile::tempdir().unwrap();
        let s = DraftStore::new(root.path().join("drafts"));
        s.save_text("keep", "fresh").unwrap();
        s.prune_older_than(MAX_AGE);
        assert!(s.load_text("keep").is_some());
        std::thread::sleep(Duration::from_millis(20));
        s.prune_older_than(Duration::from_millis(5));
        assert!(s.load_text("keep").is_none());
    }
}
