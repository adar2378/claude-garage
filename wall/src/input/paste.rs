//! Bracketed-paste forwarding (spec tui-key-routing: "Paste forwarding").
//! Port of `tui/lib/input/paste.dart`, minus the nocterm artifacts.
//!
//! crossterm delivers real bracketed pastes as `Event::Paste(String)` while
//! bracketed paste is enabled (term.rs enables it on entry), so there is no
//! `ClipboardManager` recovery flow here — the Dart `PasteForwarder` existed
//! only because the vendored nocterm framework collapsed pastes into a
//! synthetic Ctrl+V with the text parked in a clipboard buffer. The engaged
//! handler simply wraps the paste text in the guards and writes it to the
//! tile's PTY, so embedded newlines never submit lines in the remote app.
//!
//! The one rule that survives the port: a *genuine* Ctrl+V keypress is not a
//! paste — it flows through `encode_key`, which emits the raw 0x16 byte
//! (asserted below, mirroring the Dart paste suite's fall-through test).

/// Wrap `text` in bracketed-paste guards (`ESC[200~ … ESC[201~`), verbatim —
/// embedded newlines and carriage returns are preserved untouched inside the
/// guards.
pub fn wrap_bracketed_paste(text: &str) -> Vec<u8> {
    let mut out = Vec::with_capacity(text.len() + 12);
    out.extend_from_slice(b"\x1b[200~");
    out.extend_from_slice(text.as_bytes());
    out.extend_from_slice(b"\x1b[201~");
    out
}

#[cfg(test)]
mod tests {
    //! Port of the applicable tests from `tui/test/paste_test.dart`. The
    //! `PasteForwarder.recover` cases (synthetic Ctrl+V + ClipboardManager
    //! buffer) are nocterm artifacts with no crossterm equivalent — crossterm
    //! delivers pastes as `Event::Paste`, never as a synthetic key event.
    use super::*;
    use crate::input::encode::encode_key;
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    #[test]
    fn wraps_text_in_esc_200_and_esc_201_guards() {
        assert_eq!(
            wrap_bracketed_paste("hello"),
            b"\x1b[200~hello\x1b[201~".to_vec()
        );
    }

    #[test]
    fn embedded_newlines_stay_verbatim_inside_the_guards() {
        // A multi-line snippet must arrive as ONE paste — no newline may leak
        // outside the guards where the remote app would treat it as Enter.
        let text = "line one\nline two\r\nline three\n";
        let mut expected = b"\x1b[200~".to_vec();
        expected.extend_from_slice(text.as_bytes());
        expected.extend_from_slice(b"\x1b[201~");
        assert_eq!(wrap_bracketed_paste(text), expected);
    }

    #[test]
    fn empty_text_still_produces_a_well_formed_empty_paste() {
        assert_eq!(wrap_bracketed_paste(""), b"\x1b[200~\x1b[201~".to_vec());
    }

    #[test]
    fn genuine_ctrl_v_is_not_a_paste_encode_key_emits_raw_0x16() {
        // The Dart fall-through rule: a real Ctrl+V keypress continues
        // through normal key encoding and delivers the literal byte.
        let ctrl_v = KeyEvent::new(KeyCode::Char('v'), KeyModifiers::CONTROL);
        assert_eq!(encode_key(&ctrl_v), Some(vec![0x16]));
    }
}
