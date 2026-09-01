//! Verbatim key re-encoder (spec tui-key-routing: "Verbatim byte
//! re-encoding"). Port of `tui/lib/input/encode_key.dart`, from crossterm
//! [`KeyEvent`]s instead of nocterm `KeyboardEvent`s.
//!
//! Re-encodes a parsed key event back into the raw byte sequence a real
//! terminal would have sent, so an engaged tile's PTY receives exactly what
//! the user typed. This is the single choke point for passthrough fidelity —
//! modifiers must survive (Alt+Right stays `ESC[1;3C`, never plain Right).
//!
//! Coverage (the Dart table, translated case-for-case):
//!   - printable characters verbatim (UTF-8, multi-byte included);
//!   - Ctrl+letter → 0x01–0x1A (crossterm always carries the character on
//!     `KeyCode::Char`, so the Dart parser's character-null control events
//!     collapse onto the same event here — nothing is lost);
//!   - Alt+character → ESC prefix (composing with Ctrl);
//!   - arrows/Home/End with every modifier combination (`CSI 1;<mod><final>`,
//!     meta included: mod = 1 + shift(1) + alt(2) + ctrl(4) + meta(8));
//!   - Enter, Tab, Shift+Tab (`CSI Z` — crossterm surfaces it as `BackTab`),
//!     Backspace, Escape, and the Delete/Insert/PageUp/PageDown tilde family
//!     with modifiers (`CSI <n>;<mod>~`);
//!   - Alt+Enter / Alt+Backspace ESC prefixes;
//!   - F1–F4 as SS3 `ESC O P..S` unmodified / `CSI 1;<mod>P..S` modified,
//!     F5–F12 as the `CSI 15~..24~` tilde family (the Dart encoder's xterm
//!     encodings);
//!   - Ctrl+punctuation control bytes (Ctrl+Space → 0x00 etc.), from the
//!     ratatui spike's working encoder — a strict superset of the Dart table.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

/// True when the event carries a meta/super modifier (macOS Cmd). crossterm
/// splits it across SUPER and META depending on the keyboard-enhancement
/// protocol; both map to the xterm meta bit (8).
fn has_meta(m: KeyModifiers) -> bool {
    m.intersects(KeyModifiers::SUPER | KeyModifiers::META)
}

/// Encode `key` as the raw bytes a terminal would send, or `None` when the
/// event carries nothing forwardable (e.g. a bare modifier keypress).
pub fn encode_key(key: &KeyEvent) -> Option<Vec<u8>> {
    let m = key.modifiers;
    let shift = m.contains(KeyModifiers::SHIFT);
    let alt = m.contains(KeyModifiers::ALT);
    let ctrl = m.contains(KeyModifiers::CONTROL);
    let meta = has_meta(m);
    // xterm modifier parameter: 1 + shift(1) + alt(2) + ctrl(4) + meta(8).
    let modn: u8 =
        1 + u8::from(shift) + u8::from(alt) * 2 + u8::from(ctrl) * 4 + u8::from(meta) * 8;

    let csi = |fin: u8| -> Vec<u8> {
        if modn == 1 {
            vec![0x1b, b'[', fin]
        } else {
            let mut out = format!("\x1b[1;{modn}").into_bytes();
            out.push(fin);
            out
        }
    };
    let tilde = |n: u8| -> Vec<u8> {
        if modn == 1 {
            format!("\x1b[{n}~").into_bytes()
        } else {
            format!("\x1b[{n};{modn}~").into_bytes()
        }
    };
    // F1–F4: SS3 when unmodified, CSI 1;<mod><final> when modified (xterm).
    let ss3 = |fin: u8| -> Vec<u8> {
        if modn == 1 {
            vec![0x1b, b'O', fin]
        } else {
            csi(fin)
        }
    };

    let bytes = match key.code {
        KeyCode::Up => csi(b'A'),
        KeyCode::Down => csi(b'B'),
        KeyCode::Right => csi(b'C'),
        KeyCode::Left => csi(b'D'),
        KeyCode::Home => csi(b'H'),
        KeyCode::End => csi(b'F'),
        KeyCode::Enter => {
            if shift || ctrl {
                // p14 (spec tui-key-routing "Enhanced-keyboard
                // passthrough"): Shift+Enter is Claude Code's newline —
                // kitty CSI-u form (ESC[13;<mod>u), which tmux with
                // extended-keys re-encodes for the pane. Legacy \r cannot
                // carry these modifiers at all.
                format!("\x1b[13;{modn}u").into_bytes()
            } else if alt {
                // Option+Enter stays the legacy ESC CR — Claude Code
                // accepts it directly, and it works without any protocol.
                vec![0x1b, b'\r']
            } else {
                vec![b'\r']
            }
        }
        KeyCode::Tab => {
            if shift {
                b"\x1b[Z".to_vec()
            } else if alt {
                vec![0x1b, b'\t']
            } else {
                vec![b'\t']
            }
        }
        // crossterm surfaces Shift+Tab as BackTab (spec: Shift+Tab → CSI Z).
        KeyCode::BackTab => b"\x1b[Z".to_vec(),
        KeyCode::Backspace => {
            let byte = if ctrl { 0x08 } else { 0x7f };
            if alt {
                vec![0x1b, byte]
            } else {
                vec![byte]
            }
        }
        KeyCode::Esc => vec![0x1b],
        KeyCode::PageUp => tilde(5),
        KeyCode::PageDown => tilde(6),
        KeyCode::Delete => tilde(3),
        KeyCode::Insert => tilde(2),
        KeyCode::F(1) => ss3(b'P'),
        KeyCode::F(2) => ss3(b'Q'),
        KeyCode::F(3) => ss3(b'R'),
        KeyCode::F(4) => ss3(b'S'),
        KeyCode::F(5) => tilde(15),
        KeyCode::F(6) => tilde(17),
        KeyCode::F(7) => tilde(18),
        KeyCode::F(8) => tilde(19),
        KeyCode::F(9) => tilde(20),
        KeyCode::F(10) => tilde(21),
        KeyCode::F(11) => tilde(23),
        KeyCode::F(12) => tilde(24),
        KeyCode::Char(c) => {
            let mut out = Vec::with_capacity(5);
            if alt {
                out.push(0x1b);
            }
            if ctrl {
                match c.to_ascii_lowercase() {
                    l @ 'a'..='z' => out.push(l as u8 - b'a' + 1),
                    // Control-byte punctuation (spike encoder; the Dart
                    // original passed these through unmodified — a real
                    // terminal sends the control byte, so fidelity wins).
                    ' ' | '@' => out.push(0x00),
                    '[' => out.push(0x1b),
                    '\\' => out.push(0x1c),
                    ']' => out.push(0x1d),
                    '^' => out.push(0x1e),
                    '_' | '/' => out.push(0x1f),
                    '?' => out.push(0x7f),
                    other => {
                        let mut b = [0u8; 4];
                        out.extend_from_slice(other.encode_utf8(&mut b).as_bytes());
                    }
                }
            } else {
                let mut b = [0u8; 4];
                out.extend_from_slice(c.encode_utf8(&mut b).as_bytes());
            }
            out
        }
        // Bare modifier presses, media keys, locks, F13+ … — nothing a
        // terminal would forward.
        _ => return None,
    };
    Some(bytes)
}

#[cfg(test)]
mod tests {
    //! Port of `tui/test/encode_key_test.dart` — the same 71 table rows, same
    //! inputs (as their crossterm representations), same expected byte
    //! strings.
    //!
    //! Crossterm representation notes (no material gaps):
    //!   - The Dart parser's character-null Ctrl+letter events (raw
    //!     0x01–0x1A) have no distinct crossterm form — crossterm's parser
    //!     always yields `KeyCode::Char(<letter>) + CONTROL`. Those Dart rows
    //!     collapse onto the same event as their with-character twins; kept
    //!     as rows for case-for-case parity.
    //!   - Dart's `Tab + shift` arrives from crossterm as `KeyCode::BackTab`
    //!     (with SHIFT); both forms are asserted.
    //!   - Dart's `meta` modifier maps to crossterm SUPER (and META under the
    //!     kitty protocol; both are covered).
    //!   - The bare-modifier row uses `KeyCode::Modifier` (kitty protocol) —
    //!     the only crossterm form a lone modifier press can take.
    use super::*;
    use crossterm::event::ModifierKeyCode;

    const NONE: KeyModifiers = KeyModifiers::NONE;
    const SHIFT: KeyModifiers = KeyModifiers::SHIFT;
    const ALT: KeyModifiers = KeyModifiers::ALT;
    const CTRL: KeyModifiers = KeyModifiers::CONTROL;
    const META: KeyModifiers = KeyModifiers::SUPER;

    fn key(code: KeyCode, modifiers: KeyModifiers) -> KeyEvent {
        KeyEvent::new(code, modifiers)
    }

    struct Case {
        desc: &'static str,
        event: KeyEvent,
        expected: Option<&'static [u8]>,
    }

    fn cases() -> Vec<Case> {
        let ctrl_shift = CTRL | SHIFT;
        let alt_shift = ALT | SHIFT;
        let ctrl_alt = CTRL | ALT;
        let ctrl_alt_shift = CTRL | ALT | SHIFT;
        let c = |desc, event, expected: &'static [u8]| Case {
            desc,
            event,
            expected: Some(expected),
        };
        vec![
            // --- Printable characters pass through verbatim.
            c("plain h", key(KeyCode::Char('h'), NONE), b"h"),
            c(
                "shifted H (character already uppercase)",
                key(KeyCode::Char('H'), SHIFT),
                b"H",
            ),
            c("digit 1", key(KeyCode::Char('1'), NONE), b"1"),
            c("space", key(KeyCode::Char(' '), NONE), b" "),
            c(
                "multi-byte character \u{e9}",
                key(KeyCode::Char('\u{e9}'), NONE),
                "é".as_bytes(),
            ),
            // --- Ctrl+letter → 0x01–0x1a.
            c(
                "Ctrl+A with character",
                key(KeyCode::Char('a'), CTRL),
                b"\x01",
            ),
            c(
                "Ctrl+C with character",
                key(KeyCode::Char('c'), CTRL),
                b"\x03",
            ),
            c(
                "Ctrl+Z with uppercase character",
                key(KeyCode::Char('Z'), ctrl_shift),
                b"\x1a",
            ),
            // The Dart parser's character-null control events collapse onto
            // the with-character crossterm form (see module notes).
            c(
                "Ctrl+C, character null (parsed 0x03 / synthetic SIGINT)",
                key(KeyCode::Char('c'), CTRL),
                b"\x03",
            ),
            c(
                "Ctrl+A, character null (parsed 0x01)",
                key(KeyCode::Char('a'), CTRL),
                b"\x01",
            ),
            c(
                "Ctrl+V, character null (genuine Ctrl+V, empty paste buffer)",
                key(KeyCode::Char('v'), CTRL),
                b"\x16",
            ),
            c(
                "Ctrl+Alt+F, character null → ESC + 0x06",
                key(KeyCode::Char('f'), ctrl_alt),
                b"\x1b\x06",
            ),
            // --- Alt+character → ESC prefix.
            c(
                "Alt+f (readline word-forward)",
                key(KeyCode::Char('f'), ALT),
                b"\x1bf",
            ),
            c(
                "Alt+. (readline yank-last-arg)",
                key(KeyCode::Char('.'), ALT),
                b"\x1b.",
            ),
            c(
                "Ctrl+Alt+b with character → ESC + 0x02",
                key(KeyCode::Char('b'), ctrl_alt),
                b"\x1b\x02",
            ),
            // --- Arrows: every modifier combination (CSI 1;<mod><final>).
            c("Up", key(KeyCode::Up, NONE), b"\x1b[A"),
            c("Down", key(KeyCode::Down, NONE), b"\x1b[B"),
            c("Right", key(KeyCode::Right, NONE), b"\x1b[C"),
            c("Left", key(KeyCode::Left, NONE), b"\x1b[D"),
            c("Shift+Up (mod 2)", key(KeyCode::Up, SHIFT), b"\x1b[1;2A"),
            c(
                "Alt+Right (mod 3) — the spec word-jump scenario",
                key(KeyCode::Right, ALT),
                b"\x1b[1;3C",
            ),
            c("Alt+Left (mod 3)", key(KeyCode::Left, ALT), b"\x1b[1;3D"),
            c(
                "Alt+Shift+Down (mod 4)",
                key(KeyCode::Down, alt_shift),
                b"\x1b[1;4B",
            ),
            c("Ctrl+Right (mod 5)", key(KeyCode::Right, CTRL), b"\x1b[1;5C"),
            c("Ctrl+Left (mod 5)", key(KeyCode::Left, CTRL), b"\x1b[1;5D"),
            c("Ctrl+Up (mod 5)", key(KeyCode::Up, CTRL), b"\x1b[1;5A"),
            c("Ctrl+Down (mod 5)", key(KeyCode::Down, CTRL), b"\x1b[1;5B"),
            c(
                "Ctrl+Shift+Up (mod 6)",
                key(KeyCode::Up, ctrl_shift),
                b"\x1b[1;6A",
            ),
            c(
                "Ctrl+Alt+Left (mod 7)",
                key(KeyCode::Left, ctrl_alt),
                b"\x1b[1;7D",
            ),
            c(
                "Ctrl+Alt+Shift+Right (mod 8)",
                key(KeyCode::Right, ctrl_alt_shift),
                b"\x1b[1;8C",
            ),
            c("Meta+Up (mod 9)", key(KeyCode::Up, META), b"\x1b[1;9A"),
            // --- Home/End with modifiers.
            c("Home", key(KeyCode::Home, NONE), b"\x1b[H"),
            c("End", key(KeyCode::End, NONE), b"\x1b[F"),
            c(
                "Shift+Home (mod 2)",
                key(KeyCode::Home, SHIFT),
                b"\x1b[1;2H",
            ),
            c("Ctrl+End (mod 5)", key(KeyCode::End, CTRL), b"\x1b[1;5F"),
            c("Alt+End (mod 3)", key(KeyCode::End, ALT), b"\x1b[1;3F"),
            // --- Enter / Tab / Shift+Tab / Backspace / Escape.
            c("Enter", key(KeyCode::Enter, NONE), b"\r"),
            c("Alt+Enter", key(KeyCode::Enter, ALT), b"\x1b\r"),
            c("Tab", key(KeyCode::Tab, NONE), b"\t"),
            // crossterm's native Shift+Tab form is BackTab.
            c("Shift+Tab → CSI Z", key(KeyCode::BackTab, SHIFT), b"\x1b[Z"),
            c("Backspace", key(KeyCode::Backspace, NONE), b"\x7f"),
            c(
                "Alt+Backspace (delete word)",
                key(KeyCode::Backspace, ALT),
                b"\x1b\x7f",
            ),
            c("Escape", key(KeyCode::Esc, NONE), b"\x1b"),
            // --- Delete / Insert / PageUp / PageDown (tilde family).
            c("Delete", key(KeyCode::Delete, NONE), b"\x1b[3~"),
            c(
                "Shift+Delete (mod 2)",
                key(KeyCode::Delete, SHIFT),
                b"\x1b[3;2~",
            ),
            c(
                "Ctrl+Delete (mod 5)",
                key(KeyCode::Delete, CTRL),
                b"\x1b[3;5~",
            ),
            c("Insert", key(KeyCode::Insert, NONE), b"\x1b[2~"),
            c(
                "Alt+Insert (mod 3)",
                key(KeyCode::Insert, ALT),
                b"\x1b[2;3~",
            ),
            c("PageUp", key(KeyCode::PageUp, NONE), b"\x1b[5~"),
            c("PageDown", key(KeyCode::PageDown, NONE), b"\x1b[6~"),
            c(
                "Ctrl+PageUp (mod 5)",
                key(KeyCode::PageUp, CTRL),
                b"\x1b[5;5~",
            ),
            c(
                "Shift+PageDown (mod 2)",
                key(KeyCode::PageDown, SHIFT),
                b"\x1b[6;2~",
            ),
            // --- F1–F4: SS3 unmodified, CSI 1;<mod><P..S> modified (xterm).
            c("F1", key(KeyCode::F(1), NONE), b"\x1bOP"),
            c("F2", key(KeyCode::F(2), NONE), b"\x1bOQ"),
            c("F3", key(KeyCode::F(3), NONE), b"\x1bOR"),
            c("F4", key(KeyCode::F(4), NONE), b"\x1bOS"),
            c("Shift+F1 (mod 2)", key(KeyCode::F(1), SHIFT), b"\x1b[1;2P"),
            c("Ctrl+F2 (mod 5)", key(KeyCode::F(2), CTRL), b"\x1b[1;5Q"),
            c("Alt+F4 (mod 3)", key(KeyCode::F(4), ALT), b"\x1b[1;3S"),
            // --- F5–F12: CSI <n>~ family, CSI <n>;<mod>~ modified.
            c("F5", key(KeyCode::F(5), NONE), b"\x1b[15~"),
            c("F6", key(KeyCode::F(6), NONE), b"\x1b[17~"),
            c("F7", key(KeyCode::F(7), NONE), b"\x1b[18~"),
            c("F8", key(KeyCode::F(8), NONE), b"\x1b[19~"),
            c("F9", key(KeyCode::F(9), NONE), b"\x1b[20~"),
            c("F10", key(KeyCode::F(10), NONE), b"\x1b[21~"),
            c("F11", key(KeyCode::F(11), NONE), b"\x1b[23~"),
            c("F12", key(KeyCode::F(12), NONE), b"\x1b[24~"),
            c("Shift+F5 (mod 2)", key(KeyCode::F(5), SHIFT), b"\x1b[15;2~"),
            c("Ctrl+F10 (mod 5)", key(KeyCode::F(10), CTRL), b"\x1b[21;5~"),
            c("Alt+F12 (mod 3)", key(KeyCode::F(12), ALT), b"\x1b[24;3~"),
            // --- Nothing forwardable.
            Case {
                desc: "bare modifier keypress encodes to nothing",
                event: key(
                    KeyCode::Modifier(ModifierKeyCode::LeftControl),
                    CTRL,
                ),
                expected: None,
            },
        ]
    }

    #[test]
    fn the_dart_table_ports_case_for_case() {
        let cases = cases();
        assert_eq!(cases.len(), 71, "the Dart suite has 71 rows");
        for case in cases {
            assert_eq!(
                encode_key(&case.event),
                case.expected.map(<[u8]>::to_vec),
                "case: {} (event: {:?})",
                case.desc,
                case.event
            );
        }
    }

    // ── crossterm-specific extensions beyond the Dart table ─────────────────

    #[test]
    fn shift_tab_also_encodes_from_the_raw_tab_plus_shift_form() {
        // Some terminals/protocols deliver Shift+Tab as Tab+SHIFT rather than
        // BackTab; both must produce CSI Z.
        assert_eq!(
            encode_key(&key(KeyCode::Tab, SHIFT)),
            Some(b"\x1b[Z".to_vec())
        );
        assert_eq!(
            encode_key(&key(KeyCode::BackTab, NONE)),
            Some(b"\x1b[Z".to_vec())
        );
    }

    #[test]
    fn kitty_protocol_meta_modifier_also_counts_as_meta() {
        assert_eq!(
            encode_key(&key(KeyCode::Up, KeyModifiers::META)),
            Some(b"\x1b[1;9A".to_vec())
        );
    }

    #[test]
    fn ctrl_punctuation_maps_to_control_bytes_spike_extension() {
        assert_eq!(encode_key(&key(KeyCode::Char(' '), CTRL)), Some(vec![0x00]));
        assert_eq!(encode_key(&key(KeyCode::Char('['), CTRL)), Some(vec![0x1b]));
        assert_eq!(encode_key(&key(KeyCode::Char('_'), CTRL)), Some(vec![0x1f]));
        assert_eq!(encode_key(&key(KeyCode::Char('?'), CTRL)), Some(vec![0x7f]));
    }

    #[test]
    fn modified_enter_takes_the_kitty_csi_u_form_p14() {
        // Shift+Enter is Claude Code's newline; legacy \r cannot carry it.
        assert_eq!(
            encode_key(&key(KeyCode::Enter, SHIFT)),
            Some(b"\x1b[13;2u".to_vec())
        );
        assert_eq!(
            encode_key(&key(KeyCode::Enter, CTRL)),
            Some(b"\x1b[13;5u".to_vec())
        );
        assert_eq!(
            encode_key(&key(KeyCode::Enter, CTRL | SHIFT)),
            Some(b"\x1b[13;6u".to_vec())
        );
        // Option+Enter stays the protocol-free legacy form Claude Code
        // accepts directly; plain Enter stays \r.
        assert_eq!(encode_key(&key(KeyCode::Enter, ALT)), Some(b"\x1b\r".to_vec()));
        assert_eq!(encode_key(&key(KeyCode::Enter, NONE)), Some(b"\r".to_vec()));
    }

    #[test]
    fn ctrl_backspace_sends_0x08_spike_extension() {
        assert_eq!(
            encode_key(&key(KeyCode::Backspace, CTRL)),
            Some(vec![0x08])
        );
    }
}
