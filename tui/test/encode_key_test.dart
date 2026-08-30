// Table-driven tests for the verbatim key re-encoder
// (tui/lib/input/encode_key.dart) against the byte table in the
// tui-key-routing spec ("Verbatim byte re-encoding"): printable characters;
// Ctrl+letter (0x01–0x1a, including character-null parser events); Alt+char
// (ESC prefix); arrows/Home/End with every modifier combination
// (CSI 1;<mod><final>); Enter, Tab, Shift+Tab (CSI Z), Backspace, Delete,
// Insert, PageUp/PageDown with modifiers; plus the F1–F12 and Ctrl+arrow
// extensions.
import 'package:garage_tui/input/encode_key.dart';
import 'package:nocterm/nocterm.dart';
import 'package:test/test.dart';

const shift = ModifierKeys(shift: true);
const alt = ModifierKeys(alt: true);
const ctrl = ModifierKeys(ctrl: true);
const meta = ModifierKeys(meta: true);
const ctrlShift = ModifierKeys(ctrl: true, shift: true);
const altShift = ModifierKeys(alt: true, shift: true);
const ctrlAlt = ModifierKeys(ctrl: true, alt: true);
const ctrlAltShift = ModifierKeys(ctrl: true, alt: true, shift: true);

class Case {
  const Case(this.desc, this.event, this.expected);
  final String desc;
  final KeyboardEvent event;
  final String? expected;
}

const cases = <Case>[
  // --- Printable characters pass through verbatim.
  Case('plain h', KeyboardEvent(logicalKey: LogicalKey.keyH, character: 'h'),
      'h'),
  Case(
      'shifted H (character already uppercase)',
      KeyboardEvent(
          logicalKey: LogicalKey.keyH, character: 'H', modifiers: shift),
      'H'),
  Case('digit 1',
      KeyboardEvent(logicalKey: LogicalKey.digit1, character: '1'), '1'),
  Case('space',
      KeyboardEvent(logicalKey: LogicalKey.space, character: ' '), ' '),
  Case(
      'multi-byte character é',
      KeyboardEvent(logicalKey: LogicalKey.keyE, character: 'é'),
      'é'),

  // --- Ctrl+letter → 0x01–0x1a.
  Case(
      'Ctrl+A with character',
      KeyboardEvent(
          logicalKey: LogicalKey.keyA, character: 'a', modifiers: ctrl),
      '\x01'),
  Case(
      'Ctrl+C with character',
      KeyboardEvent(
          logicalKey: LogicalKey.keyC, character: 'c', modifiers: ctrl),
      '\x03'),
  Case(
      'Ctrl+Z with uppercase character',
      KeyboardEvent(
          logicalKey: LogicalKey.keyZ, character: 'Z', modifiers: ctrlShift),
      '\x1a'),
  // Parser control-byte events carry only logicalKey (character == null) —
  // see _parseControlChar in the vendored fork; the byte is re-derived.
  Case(
      'Ctrl+C, character null (parsed 0x03 / synthetic SIGINT)',
      KeyboardEvent(
          logicalKey: LogicalKey.keyC, character: null, modifiers: ctrl),
      '\x03'),
  Case(
      'Ctrl+A, character null (parsed 0x01)',
      KeyboardEvent(
          logicalKey: LogicalKey.keyA, character: null, modifiers: ctrl),
      '\x01'),
  Case(
      'Ctrl+V, character null (genuine Ctrl+V, empty paste buffer)',
      KeyboardEvent(
          logicalKey: LogicalKey.keyV, character: null, modifiers: ctrl),
      '\x16'),
  Case(
      'Ctrl+Alt+F, character null → ESC + 0x06',
      KeyboardEvent(
          logicalKey: LogicalKey.keyF, character: null, modifiers: ctrlAlt),
      '\x1b\x06'),

  // --- Alt+character → ESC prefix.
  Case(
      'Alt+f (readline word-forward)',
      KeyboardEvent(
          logicalKey: LogicalKey.keyF, character: 'f', modifiers: alt),
      '\x1bf'),
  Case(
      'Alt+. (readline yank-last-arg)',
      KeyboardEvent(
          logicalKey: LogicalKey.period, character: '.', modifiers: alt),
      '\x1b.'),
  Case(
      'Ctrl+Alt+b with character → ESC + 0x02',
      KeyboardEvent(
          logicalKey: LogicalKey.keyB, character: 'b', modifiers: ctrlAlt),
      '\x1b\x02'),

  // --- Arrows: every modifier combination (CSI 1;<mod><final>).
  Case('Up', KeyboardEvent(logicalKey: LogicalKey.arrowUp), '\x1b[A'),
  Case('Down', KeyboardEvent(logicalKey: LogicalKey.arrowDown), '\x1b[B'),
  Case('Right', KeyboardEvent(logicalKey: LogicalKey.arrowRight), '\x1b[C'),
  Case('Left', KeyboardEvent(logicalKey: LogicalKey.arrowLeft), '\x1b[D'),
  Case(
      'Shift+Up (mod 2)',
      KeyboardEvent(logicalKey: LogicalKey.arrowUp, modifiers: shift),
      '\x1b[1;2A'),
  Case(
      'Alt+Right (mod 3) — the spec word-jump scenario',
      KeyboardEvent(logicalKey: LogicalKey.arrowRight, modifiers: alt),
      '\x1b[1;3C'),
  Case(
      'Alt+Left (mod 3)',
      KeyboardEvent(logicalKey: LogicalKey.arrowLeft, modifiers: alt),
      '\x1b[1;3D'),
  Case(
      'Alt+Shift+Down (mod 4)',
      KeyboardEvent(logicalKey: LogicalKey.arrowDown, modifiers: altShift),
      '\x1b[1;4B'),
  Case(
      'Ctrl+Right (mod 5)',
      KeyboardEvent(logicalKey: LogicalKey.arrowRight, modifiers: ctrl),
      '\x1b[1;5C'),
  Case(
      'Ctrl+Left (mod 5)',
      KeyboardEvent(logicalKey: LogicalKey.arrowLeft, modifiers: ctrl),
      '\x1b[1;5D'),
  Case(
      'Ctrl+Up (mod 5)',
      KeyboardEvent(logicalKey: LogicalKey.arrowUp, modifiers: ctrl),
      '\x1b[1;5A'),
  Case(
      'Ctrl+Down (mod 5)',
      KeyboardEvent(logicalKey: LogicalKey.arrowDown, modifiers: ctrl),
      '\x1b[1;5B'),
  Case(
      'Ctrl+Shift+Up (mod 6)',
      KeyboardEvent(logicalKey: LogicalKey.arrowUp, modifiers: ctrlShift),
      '\x1b[1;6A'),
  Case(
      'Ctrl+Alt+Left (mod 7)',
      KeyboardEvent(logicalKey: LogicalKey.arrowLeft, modifiers: ctrlAlt),
      '\x1b[1;7D'),
  Case(
      'Ctrl+Alt+Shift+Right (mod 8)',
      KeyboardEvent(
          logicalKey: LogicalKey.arrowRight, modifiers: ctrlAltShift),
      '\x1b[1;8C'),
  Case(
      'Meta+Up (mod 9)',
      KeyboardEvent(logicalKey: LogicalKey.arrowUp, modifiers: meta),
      '\x1b[1;9A'),

  // --- Home/End with modifiers.
  Case('Home', KeyboardEvent(logicalKey: LogicalKey.home), '\x1b[H'),
  Case('End', KeyboardEvent(logicalKey: LogicalKey.end), '\x1b[F'),
  Case(
      'Shift+Home (mod 2)',
      KeyboardEvent(logicalKey: LogicalKey.home, modifiers: shift),
      '\x1b[1;2H'),
  Case(
      'Ctrl+End (mod 5)',
      KeyboardEvent(logicalKey: LogicalKey.end, modifiers: ctrl),
      '\x1b[1;5F'),
  Case(
      'Alt+End (mod 3)',
      KeyboardEvent(logicalKey: LogicalKey.end, modifiers: alt),
      '\x1b[1;3F'),

  // --- Enter / Tab / Shift+Tab / Backspace / Escape.
  Case('Enter', KeyboardEvent(logicalKey: LogicalKey.enter), '\r'),
  Case(
      'Alt+Enter',
      KeyboardEvent(logicalKey: LogicalKey.enter, modifiers: alt),
      '\x1b\r'),
  Case('Tab', KeyboardEvent(logicalKey: LogicalKey.tab), '\t'),
  Case(
      'Shift+Tab → CSI Z',
      KeyboardEvent(logicalKey: LogicalKey.tab, modifiers: shift),
      '\x1b[Z'),
  Case('Backspace', KeyboardEvent(logicalKey: LogicalKey.backspace), '\x7f'),
  Case(
      'Alt+Backspace (delete word)',
      KeyboardEvent(logicalKey: LogicalKey.backspace, modifiers: alt),
      '\x1b\x7f'),
  Case('Escape', KeyboardEvent(logicalKey: LogicalKey.escape), '\x1b'),

  // --- Delete / Insert / PageUp / PageDown (tilde family) with modifiers.
  Case('Delete', KeyboardEvent(logicalKey: LogicalKey.delete), '\x1b[3~'),
  Case(
      'Shift+Delete (mod 2)',
      KeyboardEvent(logicalKey: LogicalKey.delete, modifiers: shift),
      '\x1b[3;2~'),
  Case(
      'Ctrl+Delete (mod 5)',
      KeyboardEvent(logicalKey: LogicalKey.delete, modifiers: ctrl),
      '\x1b[3;5~'),
  Case('Insert', KeyboardEvent(logicalKey: LogicalKey.insert), '\x1b[2~'),
  Case(
      'Alt+Insert (mod 3)',
      KeyboardEvent(logicalKey: LogicalKey.insert, modifiers: alt),
      '\x1b[2;3~'),
  Case('PageUp', KeyboardEvent(logicalKey: LogicalKey.pageUp), '\x1b[5~'),
  Case('PageDown', KeyboardEvent(logicalKey: LogicalKey.pageDown), '\x1b[6~'),
  Case(
      'Ctrl+PageUp (mod 5)',
      KeyboardEvent(logicalKey: LogicalKey.pageUp, modifiers: ctrl),
      '\x1b[5;5~'),
  Case(
      'Shift+PageDown (mod 2)',
      KeyboardEvent(logicalKey: LogicalKey.pageDown, modifiers: shift),
      '\x1b[6;2~'),

  // --- F1–F4: SS3 unmodified, CSI 1;<mod><P..S> modified (xterm).
  Case('F1', KeyboardEvent(logicalKey: LogicalKey.f1), '\x1bOP'),
  Case('F2', KeyboardEvent(logicalKey: LogicalKey.f2), '\x1bOQ'),
  Case('F3', KeyboardEvent(logicalKey: LogicalKey.f3), '\x1bOR'),
  Case('F4', KeyboardEvent(logicalKey: LogicalKey.f4), '\x1bOS'),
  Case(
      'Shift+F1 (mod 2)',
      KeyboardEvent(logicalKey: LogicalKey.f1, modifiers: shift),
      '\x1b[1;2P'),
  Case(
      'Ctrl+F2 (mod 5)',
      KeyboardEvent(logicalKey: LogicalKey.f2, modifiers: ctrl),
      '\x1b[1;5Q'),
  Case(
      'Alt+F4 (mod 3)',
      KeyboardEvent(logicalKey: LogicalKey.f4, modifiers: alt),
      '\x1b[1;3S'),

  // --- F5–F12: CSI <n>~ family, CSI <n>;<mod>~ modified.
  Case('F5', KeyboardEvent(logicalKey: LogicalKey.f5), '\x1b[15~'),
  Case('F6', KeyboardEvent(logicalKey: LogicalKey.f6), '\x1b[17~'),
  Case('F7', KeyboardEvent(logicalKey: LogicalKey.f7), '\x1b[18~'),
  Case('F8', KeyboardEvent(logicalKey: LogicalKey.f8), '\x1b[19~'),
  Case('F9', KeyboardEvent(logicalKey: LogicalKey.f9), '\x1b[20~'),
  Case('F10', KeyboardEvent(logicalKey: LogicalKey.f10), '\x1b[21~'),
  Case('F11', KeyboardEvent(logicalKey: LogicalKey.f11), '\x1b[23~'),
  Case('F12', KeyboardEvent(logicalKey: LogicalKey.f12), '\x1b[24~'),
  Case(
      'Shift+F5 (mod 2)',
      KeyboardEvent(logicalKey: LogicalKey.f5, modifiers: shift),
      '\x1b[15;2~'),
  Case(
      'Ctrl+F10 (mod 5)',
      KeyboardEvent(logicalKey: LogicalKey.f10, modifiers: ctrl),
      '\x1b[21;5~'),
  Case(
      'Alt+F12 (mod 3)',
      KeyboardEvent(logicalKey: LogicalKey.f12, modifiers: alt),
      '\x1b[24;3~'),

  // --- Nothing forwardable.
  Case(
      'bare modifier keypress encodes to nothing',
      KeyboardEvent(logicalKey: LogicalKey.controlLeft, modifiers: ctrl),
      null),
];

void main() {
  group('encodeKey', () {
    for (final c in cases) {
      test(c.desc, () {
        expect(encodeKey(c.event), c.expected,
            reason: 'event: ${c.event}');
      });
    }
  });
}
