/// Bracketed-paste forwarding (tui-key-routing spec: "Paste forwarding").
///
/// The vendored framework collapses real bracketed pastes (and long
/// coalesced printable runs) into a synthetic Ctrl+V key event with the text
/// parked in [ClipboardManager] — see patch 1 in tui/vendor/NOCTERM_VERSION
/// and the paste-recovery finding in spikes/nocterm-wall/SPIKE.md. While
/// engaged, that text must reach the tile's PTY as one bracketed paste
/// (`ESC[200~ … ESC[201~`) so embedded newlines never submit lines in the
/// remote app.
///
/// A genuine Ctrl+V keypress with an *empty* buffer is not a paste: the
/// recovery reports "not handled" (null) and the caller falls through to
/// `encodeKey`, which emits the raw 0x16 byte.
library;

import 'package:nocterm/nocterm.dart';

/// Wrap [text] in bracketed-paste guards, verbatim — embedded newlines and
/// carriage returns are preserved untouched inside the guards.
String wrapBracketedPaste(String text) => '\x1b[200~$text\x1b[201~';

/// Reads the framework's internal paste buffer. Injectable for tests.
typedef ClipboardReader = String? Function();

/// Policy object for the synthetic-Ctrl+V paste recovery flow.
///
/// Pure decision logic — no terminal, no PTY. The engaged key handler calls
/// [recover]; a non-null result is the exact byte string to write to the
/// tile's PTY (and the event is handled). A null result means the event is
/// not a recoverable paste and must continue through normal key encoding.
class PasteForwarder {
  PasteForwarder({ClipboardReader? readClipboard})
      : _read = readClipboard ?? ClipboardManager.paste;

  final ClipboardReader _read;

  /// If [event] is the synthetic Ctrl+V carrying a non-empty paste buffer,
  /// return the bracketed-paste bytes to forward; otherwise null.
  String? recover(KeyboardEvent event) {
    if (!event.matches(LogicalKey.keyV, ctrl: true)) return null;
    final text = _read();
    if (text == null || text.isEmpty) return null;
    return wrapBracketedPaste(text);
  }
}
