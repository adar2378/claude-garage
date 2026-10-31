//! Getting text onto the system clipboard (p25 mouse selection and copy).
//!
//! Two routes, both best effort: the OSC 52 escape (works over ssh and in
//! terminals that allow it, such as Ghostty and kitty) and the platform's
//! own clipboard tool (`pbcopy`, `wl-copy`, `xclip`, `xsel`, `clip`). Using
//! both costs nothing: they write the same text.

use std::io::Write;
use std::process::{Command, Stdio};

/// Largest selection copied. OSC 52 payloads past ~100 KB are dropped by
/// several terminals, so a bigger selection is cut rather than lost whole.
pub const MAX_COPY_BYTES: usize = 100_000;

/// Standard base64 with padding (RFC 4648). Small enough to own; no crate.
pub fn base64(bytes: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let n = (u32::from(chunk[0]) << 16)
            | (u32::from(*chunk.get(1).unwrap_or(&0)) << 8)
            | u32::from(*chunk.get(2).unwrap_or(&0));
        out.push(T[(n >> 18) as usize & 63] as char);
        out.push(T[(n >> 12) as usize & 63] as char);
        out.push(if chunk.len() > 1 {
            T[(n >> 6) as usize & 63] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            T[n as usize & 63] as char
        } else {
            '='
        });
    }
    out
}

/// `text` cut to [`MAX_COPY_BYTES`] on a character boundary.
pub fn clamp_copy(text: &str) -> &str {
    if text.len() <= MAX_COPY_BYTES {
        return text;
    }
    let mut end = MAX_COPY_BYTES;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    &text[..end]
}

/// The OSC 52 "set clipboard" sequence for `text`.
pub fn osc52(text: &str) -> String {
    format!("\x1b]52;c;{}\x07", base64(clamp_copy(text).as_bytes()))
}

/// Candidate commands for this OS, tried in order.
fn native_commands() -> &'static [(&'static str, &'static [&'static str])] {
    if cfg!(target_os = "macos") {
        &[("pbcopy", &[])]
    } else if cfg!(windows) {
        &[("clip", &[])]
    } else {
        &[
            ("wl-copy", &[]),
            ("xclip", &["-selection", "clipboard"]),
            ("xsel", &["--clipboard", "--input"]),
        ]
    }
}

/// Pipe `text` to the first clipboard tool that exists. A missing tool is
/// not an error: OSC 52 may have worked, and there is nowhere to report to.
pub fn native_copy(text: &str) -> bool {
    let text = clamp_copy(text);
    for (bin, args) in native_commands() {
        let Ok(mut child) = Command::new(bin)
            .args(*args)
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
        else {
            continue;
        };
        if let Some(mut stdin) = child.stdin.take() {
            let _ = stdin.write_all(text.as_bytes());
        }
        // Reap on a thread: xclip stays alive to serve the selection.
        std::thread::spawn(move || {
            let _ = child.wait();
        });
        return true;
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64_matches_the_rfc_vectors() {
        for (plain, enc) in [
            ("", ""),
            ("f", "Zg=="),
            ("fo", "Zm8="),
            ("foo", "Zm9v"),
            ("foob", "Zm9vYg=="),
            ("fooba", "Zm9vYmE="),
            ("foobar", "Zm9vYmFy"),
        ] {
            assert_eq!(base64(plain.as_bytes()), enc, "{plain:?}");
        }
    }

    #[test]
    fn osc52_wraps_the_encoded_text_for_the_clipboard_selection() {
        assert_eq!(osc52("hi"), "\x1b]52;c;aGk=\x07");
    }

    #[test]
    fn an_oversized_copy_is_cut_on_a_character_boundary() {
        let text = "é".repeat(MAX_COPY_BYTES); // 2 bytes each
        let cut = clamp_copy(&text);
        assert!(cut.len() <= MAX_COPY_BYTES);
        assert!(cut.chars().all(|c| c == 'é'));
        assert_eq!(clamp_copy("short"), "short");
    }
}
