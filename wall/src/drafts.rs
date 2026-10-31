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
use std::io::{Read as _, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime};

/// Longest draft kept (bytes). A bigger one is cut on a character boundary.
pub const MAX_TEXT_BYTES: usize = 64 * 1024;
/// Most images kept per session.
pub const MAX_IMAGES: usize = 8;
/// Largest single image kept (bytes).
pub const MAX_IMAGE_BYTES: u64 = 20 * 1024 * 1024;
/// Files older than this are removed (at startup and hourly while running).
pub const MAX_AGE: Duration = Duration::from_secs(7 * 24 * 3600);

static IMAGE_SEQ: AtomicU64 = AtomicU64::new(0);

const IMAGE_EXTS: [&str; 5] = ["png", "jpg", "jpeg", "gif", "webp"];

/// The image extensions with their dot (`.png`), for a cheap look at text
/// before any file is touched.
pub fn image_extensions() -> [String; 5] {
    IMAGE_EXTS.map(|e| format!(".{e}"))
}

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
    let mut out = String::new();
    // The previous row was soft-wrapped by the terminal: this row continues
    // it, with no line break and no prompt indent in between.
    let mut continued = false;
    for (r, line) in lines.iter().enumerate().take(b).skip(a + 1) {
        let Ok(row) = u16::try_from(r) else {
            return Prompt::Unknown;
        };
        if row >= rows {
            break;
        }
        let first = r == a + 1;
        let mut from = 0;
        if first {
            if !(line.starts_with('❯') || line.starts_with('>')) {
                return Prompt::Unknown; // not a prompt line
            }
            from = 1;
        } else if !continued {
            from = 2; // the box indents its continuation lines
        }
        // Only text drawn at normal intensity was typed: a dim suggestion
        // ("Try ...") or ghost completion is not, even on the same row.
        let mut text = String::new();
        for c in from..cols {
            match screen.cell(row, c) {
                Some(cell) if cell.is_wide_continuation() => {}
                Some(cell) if cell.dim() => {}
                Some(cell) if !cell.contents().is_empty() => text.push_str(cell.contents()),
                _ => text.push(' '),
            }
        }
        let wrapped = screen.row_wrapped(row);
        if !continued && !out.is_empty() {
            out.push('\n');
        }
        let text = if first {
            text.trim_start_matches(' ')
        } else {
            text.as_str()
        };
        if wrapped {
            out.push_str(text);
        } else {
            out.push_str(text.trim_end());
        }
        continued = wrapped;
    }
    let text = out.trim().to_owned();
    if text.is_empty() || text.starts_with("Try \"") {
        Prompt::Empty
    } else {
        Prompt::Text(text)
    }
}

/// Length of a Claude Code placeholder at the start of `s`: `[Image #N]` or
/// `[Pasted text #N +M lines]` (the `+M lines` part is optional).
fn placeholder_len(s: &str) -> Option<usize> {
    fn digits(t: &str) -> Option<&str> {
        let n = t.chars().take_while(char::is_ascii_digit).count();
        (n > 0).then(|| &t[n..])
    }
    let rest = if let Some(r) = s.strip_prefix("[Image #") {
        digits(r)?
    } else {
        let r = digits(s.strip_prefix("[Pasted text #")?)?;
        match r.strip_prefix(" +") {
            Some(more) => {
                let after = digits(more)?.strip_prefix(" line")?;
                after.strip_prefix('s').unwrap_or(after)
            }
            None => r,
        }
    };
    rest.strip_prefix(']').map(|after| s.len() - after.len())
}

/// `text` with Claude Code's `[Image #N]` and `[Pasted text #N +M lines]`
/// placeholders removed (images come back as files, which Claude numbers
/// afresh; a pasted block is a stand-in for text we do not hold).
pub fn strip_placeholders(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(i) = rest.find('[') {
        out.push_str(&rest[..i]);
        let tail = &rest[i..];
        match placeholder_len(tail) {
            Some(n) => rest = &tail[n..],
            None => {
                out.push('[');
                rest = &tail[1..];
            }
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

// ── the off switch ─────────────────────────────────────────────────────

/// Whether drafts may be kept: off when `GARAGE_DRAFTS` is `0`/`false`/`no`/
/// `off`, or `config.toml` says `drafts = false` (the file and the parser
/// `animations` uses). On by default. Pure; [`load_enabled`] feeds it the
/// real world.
pub fn drafts_enabled(config: Option<&str>, env: &dyn Fn(&str) -> Option<String>) -> bool {
    let env_off = env("GARAGE_DRAFTS").is_some_and(|v| {
        matches!(
            v.trim().to_ascii_lowercase().as_str(),
            "0" | "false" | "no" | "off"
        )
    });
    !env_off && config.and_then(|c| crate::ui::motion::config_bool(c, "drafts")) != Some(false)
}

pub fn load_enabled() -> bool {
    let config =
        fs::read_to_string(crate::state::persistence::garage_dir_from_env().join("config.toml"))
            .ok();
    drafts_enabled(config.as_deref(), &|name| std::env::var(name).ok())
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

/// Copy `src` to `tmp` (private, synced), at most `cap` bytes. `Ok(false)`
/// when the file is bigger than that (it grew since it was measured): the
/// partial copy is removed, so a truncated image is never kept. Any failure
/// removes `tmp` too.
fn copy_bounded(src: &Path, tmp: &Path, cap: u64) -> std::io::Result<bool> {
    let copied = (|| {
        let input = fs::File::open(src)?;
        let mut out = private_file(tmp)?;
        // One byte past the cap tells a file that outgrew it from one that
        // fits exactly.
        let n = std::io::copy(&mut input.take(cap + 1), &mut out)?;
        out.sync_all()?;
        Ok::<bool, std::io::Error>(n <= cap)
    })();
    if !matches!(copied, Ok(true)) {
        let _ = fs::remove_file(tmp);
    }
    copied
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
        self.add_image_if(id, src, &|| true)
    }

    /// [`DraftStore::add_image`], but the copy is discarded (`Ok(None)`)
    /// when `wanted()` is false by the time it is done, just before it
    /// would be renamed into place.
    pub fn add_image_if(
        &self,
        id: &str,
        src: &Path,
        wanted: &dyn Fn() -> bool,
    ) -> std::io::Result<Option<PathBuf>> {
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
        let seq = IMAGE_SEQ.fetch_add(1, Ordering::Relaxed);
        // Zero-padded so a plain sort is oldest-first.
        let dest = dir.join(format!("img-{ms:016}-{seq:08}.{ext}"));
        // Copied under a name `images()` ignores, then renamed: a copy cut
        // short (crash, full disk, a file that kept growing) is never replayed.
        let tmp = dir.join(format!(".copy-{ms:016}-{seq:08}.tmp"));
        if copy_bounded(src, &tmp, MAX_IMAGE_BYTES)? {
            if !wanted() {
                let _ = fs::remove_file(&tmp);
                return Ok(None);
            }
            fs::rename(&tmp, &dest).inspect_err(|_| {
                let _ = fs::remove_file(&tmp);
            })?;
            Ok(Some(dest))
        } else {
            Ok(None)
        }
    }

    /// The kept images, oldest first.
    pub fn images(&self, id: &str) -> Vec<PathBuf> {
        let Ok(rd) = fs::read_dir(self.images_dir(id)) else {
            return Vec::new();
        };
        let mut v: Vec<PathBuf> = rd
            .flatten()
            .map(|e| e.path())
            .filter(|p| {
                p.file_name()
                    .and_then(|n| n.to_str())
                    .is_some_and(|n| n.starts_with("img-"))
            })
            .collect();
        v.sort();
        v
    }

    pub fn has_anything(&self, id: &str) -> bool {
        self.load_text(id).is_some() || !self.images(id).is_empty()
    }

    /// Remove the whole drafts folder (recovery switched off).
    pub fn wipe(&self) {
        let _ = fs::remove_dir_all(&self.root);
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
    fn dim_ghost_text_beside_typed_text_does_not_hide_the_draft() {
        // Typed words followed by a dim completion: the draft is the typed part.
        let p = screen_with("❯ fix the \x1b[2mfailing test\x1b[0m");
        assert_eq!(read_prompt(p.screen()), Prompt::Text("fix the".into()));
        // Only dim text (and a bare "Try ..." that lost its dimness) is empty.
        let all_dim = screen_with("❯ \x1b[2mTry \"x\"\x1b[0m");
        assert_eq!(read_prompt(all_dim.screen()), Prompt::Empty);
        assert_eq!(
            read_prompt(screen_with("❯ Try \"x\"").screen()),
            Prompt::Empty
        );
    }

    #[test]
    fn a_soft_wrapped_row_joins_without_a_line_break() {
        // 40 columns: 50 characters wrap once by the terminal itself.
        let long = "word ".repeat(10);
        let p = screen_with(&format!("❯ {}", long.trim_end()));
        let Prompt::Text(t) = read_prompt(p.screen()) else {
            panic!("expected text");
        };
        assert!(!t.contains('\n'), "{t:?}");
        assert_eq!(t, long.trim_end());
        // A real line break stays one.
        let two = screen_with("❯ one\r\n  two");
        assert_eq!(read_prompt(two.screen()), Prompt::Text("one\ntwo".into()));
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
    fn placeholders_are_removed_and_lookalikes_kept() {
        assert_eq!(
            strip_placeholders("look [Image #1] and [Image #12] here"),
            "look  and  here"
        );
        assert_eq!(
            strip_placeholders("[Image #x] [Image #"),
            "[Image #x] [Image #"
        );
        // A pasted block is a stand-in for text we never held.
        assert_eq!(
            strip_placeholders("see [Pasted text #2 +35 lines] now [Pasted text #3 +1 line]"),
            "see  now"
        );
        assert_eq!(
            strip_placeholders("[Pasted text #1] [Pasted text #x +2 lines] [Pasted text #1 +2"),
            "[Pasted text #x +2 lines] [Pasted text #1 +2"
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
    fn a_copy_over_the_cap_is_dropped_and_a_half_copied_file_is_never_replayed() {
        let root = tempfile::tempdir().unwrap();
        let src = root.path().join("a.png");
        fs::write(&src, b"12345").unwrap();
        let tmp = root.path().join("t.tmp");
        // Fits exactly: kept. One byte over: no truncated copy survives.
        assert!(copy_bounded(&src, &tmp, 5).unwrap());
        assert_eq!(fs::read(&tmp).unwrap(), b"12345");
        assert!(!copy_bounded(&src, &tmp, 4).unwrap());
        assert!(!tmp.exists());
        assert!(copy_bounded(&root.path().join("gone.png"), &tmp, 4).is_err());
        assert!(!tmp.exists());

        // A leftover temp copy (crash mid-copy) is not an image to replay.
        let s = DraftStore::new(root.path().join("drafts"));
        let dir = s.images_dir("g");
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join(".copy-0-0.tmp"), b"half").unwrap();
        fs::write(dir.join("img-0000000000000001-00000001.png"), b"ok").unwrap();
        assert_eq!(s.images("g").len(), 1);
    }

    #[test]
    fn a_copy_nobody_wants_any_more_is_discarded_before_it_lands() {
        let root = tempfile::tempdir().unwrap();
        let s = DraftStore::new(root.path().join("drafts"));
        let src = root.path().join("a.png");
        fs::write(&src, b"png").unwrap();
        assert_eq!(s.add_image_if("x", &src, &|| false).unwrap(), None);
        assert!(s.images("x").is_empty());
        let leftovers = fs::read_dir(s.images_dir("x")).map_or(0, Iterator::count);
        assert_eq!(leftovers, 0, "no temp file stays");
    }

    #[test]
    fn drafts_can_be_switched_off_by_env_or_config() {
        let env = |v: Option<&'static str>| {
            move |k: &str| v.filter(|_| k == "GARAGE_DRAFTS").map(str::to_owned)
        };
        assert!(drafts_enabled(None, &env(None)), "on by default");
        assert!(!drafts_enabled(None, &env(Some("0"))));
        assert!(!drafts_enabled(None, &env(Some("off"))));
        assert!(drafts_enabled(None, &env(Some("1"))));
        assert!(!drafts_enabled(Some("drafts = false\n"), &env(None)));
        assert!(drafts_enabled(
            Some("drafts = true\nanimations = false\n"),
            &env(None)
        ));
        // The animations key is a different switch.
        assert!(drafts_enabled(Some("animations = false\n"), &env(None)));
        assert!(!drafts_enabled(Some("drafts = true\n"), &env(Some("0"))));
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
