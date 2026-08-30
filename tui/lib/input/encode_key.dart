/// Verbatim key re-encoder (tui-key-routing spec: "Verbatim byte
/// re-encoding").
///
/// Re-encodes a parsed [KeyboardEvent] back into the raw byte sequence a
/// real terminal would have sent, so an engaged tile's PTY receives exactly
/// what the user typed. This is the single choke point for passthrough
/// fidelity: the framework's built-in key translation drops modifiers
/// (Alt+Right becomes plain Right — the bug the spike reproduced), so it is
/// never used.
///
/// Ported verbatim in spirit from the p8 spike
/// (`spikes/nocterm-wall/bin/wall.dart`), extended with:
///   - F1–F12 (xterm encodings: SS3 `ESC O P..S` for unmodified F1–F4,
///     `CSI 1;<mod>P..S` when modified; the `CSI 15~..24~` tilde family for
///     F5–F12, `CSI <n>;<mod>~` when modified);
///   - Ctrl+arrows and every other arrow/Home/End modifier combination via
///     the shared `CSI 1;<mod><final>` form (Ctrl is mod 5, e.g. `ESC[1;5C`);
///   - Ctrl+letter events whose `character` is null: the input parser
///     surfaces raw control bytes 0x01–0x1A as logicalKey-only events with
///     no character (see `_parseControlChar` in the vendored fork), so the
///     control byte is re-derived from the letter key's id.
library;

import 'package:nocterm/nocterm.dart';

/// Encode [e] as the raw bytes a terminal would send, or null when the event
/// carries nothing forwardable (e.g. a bare modifier keypress).
String? encodeKey(KeyboardEvent e) {
  final m = e.modifiers;
  // xterm modifier parameter: 1 + shift(1) + alt(2) + ctrl(4) + meta(8).
  final mod = 1 +
      (m.shift ? 1 : 0) +
      (m.alt ? 2 : 0) +
      (m.ctrl ? 4 : 0) +
      (m.meta ? 8 : 0);
  String csi(String fin) =>
      m.hasAnyModifier ? '\x1b[1;$mod$fin' : '\x1b[$fin';
  String tilde(int n) => m.hasAnyModifier ? '\x1b[$n;$mod~' : '\x1b[$n~';
  // F1–F4: SS3 when unmodified, CSI 1;<mod><final> when modified (xterm).
  String ss3(String fin) => m.hasAnyModifier ? '\x1b[1;$mod$fin' : '\x1bO$fin';

  switch (e.logicalKey) {
    case LogicalKey.arrowUp:
      return csi('A');
    case LogicalKey.arrowDown:
      return csi('B');
    case LogicalKey.arrowRight:
      return csi('C');
    case LogicalKey.arrowLeft:
      return csi('D');
    case LogicalKey.home:
      return csi('H');
    case LogicalKey.end:
      return csi('F');
    case LogicalKey.enter:
      return m.alt ? '\x1b\r' : '\r';
    case LogicalKey.tab:
      return m.shift ? '\x1b[Z' : '\t';
    case LogicalKey.backspace:
      return m.alt ? '\x1b\x7f' : '\x7f';
    case LogicalKey.escape:
      return '\x1b';
    case LogicalKey.pageUp:
      return tilde(5);
    case LogicalKey.pageDown:
      return tilde(6);
    case LogicalKey.delete:
      return tilde(3);
    case LogicalKey.insert:
      return tilde(2);
    case LogicalKey.f1:
      return ss3('P');
    case LogicalKey.f2:
      return ss3('Q');
    case LogicalKey.f3:
      return ss3('R');
    case LogicalKey.f4:
      return ss3('S');
    case LogicalKey.f5:
      return tilde(15);
    case LogicalKey.f6:
      return tilde(17);
    case LogicalKey.f7:
      return tilde(18);
    case LogicalKey.f8:
      return tilde(19);
    case LogicalKey.f9:
      return tilde(20);
    case LogicalKey.f10:
      return tilde(21);
    case LogicalKey.f11:
      return tilde(23);
    case LogicalKey.f12:
      return tilde(24);
    default:
      break;
  }

  final ch = e.character;
  if (ch == null || ch.isEmpty) {
    // Raw control bytes (0x01–0x1A) parse to logicalKey-only events with a
    // null character; letter-key ids are the lowercase ASCII codes, so the
    // control byte is keyId - 0x60.
    final id = e.logicalKey.keyId;
    if (m.ctrl && id >= 0x61 && id <= 0x7a) {
      final s = String.fromCharCode(id - 0x60);
      return m.alt ? '\x1b$s' : s;
    }
    return null;
  }
  var s = ch;
  if (m.ctrl && ch.length == 1) {
    final c = ch.toLowerCase().codeUnitAt(0);
    if (c >= 0x61 && c <= 0x7a) s = String.fromCharCode(c - 0x60);
  }
  if (m.alt) s = '\x1b$s';
  return s;
}
