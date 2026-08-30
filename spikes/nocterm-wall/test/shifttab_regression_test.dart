// Regression test for the live-app Shift+Tab (ESC[Z) drop — task group 8
// of p8-nocterm-tui.
//
// Root cause: InputParser._parseCSISequence matched the 3-byte CSI finals
// (ESC[Z, ESC[A..D, ESC[H, ESC[F) only when the parser buffer was EXACTLY
// 3 bytes long. In the live app, stdin reads coalesce under load (startup
// frames, streaming PTY tiles, mouse motion traffic), so ESC[Z shared a
// read with other bytes and fell through to the fallbacks: either the
// "unknown CSI" consumption ate it (Z is a valid CSI final byte), or the
// last-byte completeness check returned "need more bytes" and the whole
// read sat stuck until the 100ms staleness clear dropped everything.
//
// These tests replay the exact coalesced reads observed live (via
// `tmux send-keys -H` against a running nocterm app) as directly as the
// failing path allows: one addBytes() per stdin read, then a full
// parseNext() drain — the same loop TerminalBinding._startInputHandling
// runs.
import 'package:nocterm/src/keyboard/input_event.dart';
import 'package:nocterm/src/keyboard/input_parser.dart';
import 'package:nocterm/nocterm.dart' show LogicalKey;
import 'package:test/test.dart';

/// Simulates one stdin read: add the bytes, drain all events (mirrors the
/// parseNext loop in TerminalBinding._startInputHandling).
List<InputEvent> drain(InputParser parser, List<int> bytes) {
  parser.addBytes(bytes);
  final events = <InputEvent>[];
  InputEvent? e;
  while ((e = parser.parseNext()) != null) {
    events.add(e!);
  }
  return events;
}

void expectShiftTab(InputEvent e) {
  expect(e, isA<KeyboardInputEvent>());
  final k = (e as KeyboardInputEvent).event;
  expect(k.logicalKey, LogicalKey.tab);
  expect(k.modifiers.shift, isTrue, reason: 'expected Shift+Tab');
}

void main() {
  group('Shift+Tab (ESC[Z) regression', () {
    test('lone ESC[Z parses (control — always worked)', () {
      final events = drain(InputParser(), '\x1b[Z'.codeUnits);
      expect(events, hasLength(1));
      expectShiftTab(events[0]);
    });

    test('ESC[Z coalesced with a trailing plain Tab in ONE read '
        '(live failure: BOTH keys were dropped)', () {
      final events = drain(InputParser(), '\x1b[Z\x09'.codeUnits);
      expect(events, hasLength(2));
      expectShiftTab(events[0]);
      final tab = (events[1] as KeyboardInputEvent).event;
      expect(tab.logicalKey, LogicalKey.tab);
      expect(tab.modifiers.shift, isFalse);
    });

    test('two ESC[Z in ONE read '
        '(live failure: first one was eaten as unknown CSI)', () {
      final events = drain(InputParser(), '\x1b[Z\x1b[Z'.codeUnits);
      expect(events, hasLength(2));
      expectShiftTab(events[0]);
      expectShiftTab(events[1]);
    });

    test('ESC[Z coalesced with a following SGR mouse sequence '
        '(mouse-motion traffic scenario)', () {
      final events = drain(InputParser(), '\x1b[Z\x1b[<35;10;5M'.codeUnits);
      expect(events.length, greaterThanOrEqualTo(1));
      expectShiftTab(events[0]);
    });

    test('ESC[Z coalesced with a following alt+arrow CSI', () {
      final events = drain(InputParser(), '\x1b[Z\x1b[1;3C'.codeUnits);
      expect(events, hasLength(2));
      expectShiftTab(events[0]);
      final right = (events[1] as KeyboardInputEvent).event;
      expect(right.logicalKey, LogicalKey.arrowRight);
      expect(right.modifiers.alt, isTrue);
    });

    test('ESC[Z after a printable char in the same read', () {
      final events = drain(InputParser(), 'a\x1b[Z'.codeUnits);
      expect(events, hasLength(2));
      final a = (events[0] as KeyboardInputEvent).event;
      expect(a.character, 'a');
      expectShiftTab(events[1]);
    });

    test('coalesced plain arrow also survives (same == 3 bug family)', () {
      final events = drain(InputParser(), '\x1b[A\x09'.codeUnits);
      expect(events, hasLength(2));
      final up = (events[0] as KeyboardInputEvent).event;
      expect(up.logicalKey, LogicalKey.arrowUp);
      final tab = (events[1] as KeyboardInputEvent).event;
      expect(tab.logicalKey, LogicalKey.tab);
    });

    test('split delivery: ESC[Z arriving byte-by-byte still parses '
        'once complete', () {
      final parser = InputParser();
      expect(drain(parser, [0x1b]), hasLength(1),
          reason: 'lone ESC parses as Escape (pre-existing behavior)');
      // Re-send as split CSI: ESC [ then Z
      final p2 = InputParser();
      expect(drain(p2, [0x1b, 0x5b]), isEmpty, reason: 'incomplete CSI waits');
      final events = drain(p2, [0x5a]);
      expect(events, hasLength(1));
      expectShiftTab(events[0]);
    });
  });
}
