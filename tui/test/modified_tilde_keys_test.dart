// Regression test for the vendored modified-tilde-key patch
// (tui/vendor/NOCTERM_VERSION, patch 7) — task group 7 of p8-nocterm-tui.
//
// Upstream InputParser matched only the UNMODIFIED tilde sequences exactly
// (`ESC[5~`, `ESC[6~`, ...), so xterm's modified encoding
// `ESC[<code>;<mod>~` — Shift+PageUp arrives as `ESC[5;2~` from every
// mainstream terminal, tmux included — fell through to the unknown-CSI
// consumer and was silently dropped before any component could see it. The
// tui-scrollback spec hangs the whole frozen-history view off
// Shift+PageUp/Down while engaged, so the drop was fatal.
import 'package:nocterm/nocterm.dart' show LogicalKey;
import 'package:nocterm/src/keyboard/input_event.dart';
import 'package:nocterm/src/keyboard/input_parser.dart';
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

KeyboardInputEvent single(List<InputEvent> events) {
  expect(events, hasLength(1));
  expect(events.single, isA<KeyboardInputEvent>());
  return events.single as KeyboardInputEvent;
}

void main() {
  test('Shift+PageUp (ESC[5;2~) parses with the shift modifier', () {
    final k = single(drain(InputParser(), '\x1B[5;2~'.codeUnits)).event;
    expect(k.logicalKey, LogicalKey.pageUp);
    expect(k.modifiers.shift, isTrue);
    expect(k.modifiers.ctrl, isFalse);
    expect(k.modifiers.alt, isFalse);
  });

  test('Shift+PageDown (ESC[6;2~) parses with the shift modifier', () {
    final k = single(drain(InputParser(), '\x1B[6;2~'.codeUnits)).event;
    expect(k.logicalKey, LogicalKey.pageDown);
    expect(k.modifiers.shift, isTrue);
  });

  test('Ctrl+PageUp (ESC[5;5~) decodes the ctrl bit', () {
    final k = single(drain(InputParser(), '\x1B[5;5~'.codeUnits)).event;
    expect(k.logicalKey, LogicalKey.pageUp);
    expect(k.modifiers.ctrl, isTrue);
    expect(k.modifiers.shift, isFalse);
  });

  test('Ctrl+Shift+Delete (ESC[3;6~) decodes combined bits', () {
    final k = single(drain(InputParser(), '\x1B[3;6~'.codeUnits)).event;
    expect(k.logicalKey, LogicalKey.delete);
    expect(k.modifiers.ctrl, isTrue);
    expect(k.modifiers.shift, isTrue);
    expect(k.modifiers.alt, isFalse);
  });

  test('vt220 Home/End (ESC[1~ / ESC[4~, the tmux encoding) parse', () {
    final home = single(drain(InputParser(), '\x1B[1~'.codeUnits)).event;
    expect(home.logicalKey, LogicalKey.home);
    expect(home.modifiers.shift, isFalse);
    final end = single(drain(InputParser(), '\x1B[4~'.codeUnits)).event;
    expect(end.logicalKey, LogicalKey.end);
    expect(end.modifiers.shift, isFalse);
  });

  test('multi-digit function keys are untouched (ESC[15~ is still F5)', () {
    final k = single(drain(InputParser(), '\x1B[15~'.codeUnits)).event;
    expect(k.logicalKey, LogicalKey.f5);
  });

  test('unmodified PageUp/PageDown still parse (upstream path intact)', () {
    final up = single(drain(InputParser(), '\x1B[5~'.codeUnits)).event;
    expect(up.logicalKey, LogicalKey.pageUp);
    expect(up.modifiers.shift, isFalse);
    final down = single(drain(InputParser(), '\x1B[6~'.codeUnits)).event;
    expect(down.logicalKey, LogicalKey.pageDown);
  });

  test('coalesced read: Shift+PageUp followed by more bytes in one read',
      () {
    // Under load stdin reads coalesce — the modified sequence must parse
    // by prefix, not exact buffer equality (same failure family as the
    // patch-3 Shift+Tab drop).
    final events = drain(InputParser(), '\x1B[5;2~x'.codeUnits);
    expect(events, hasLength(2));
    final first = (events[0] as KeyboardInputEvent).event;
    expect(first.logicalKey, LogicalKey.pageUp);
    expect(first.modifiers.shift, isTrue);
    final second = (events[1] as KeyboardInputEvent).event;
    expect(second.character, 'x');
  });

  test('two Shift+PageUps coalesced into one read both parse', () {
    final events = drain(InputParser(), '\x1B[5;2~\x1B[5;2~'.codeUnits);
    expect(events, hasLength(2));
    for (final e in events) {
      final k = (e as KeyboardInputEvent).event;
      expect(k.logicalKey, LogicalKey.pageUp);
      expect(k.modifiers.shift, isTrue);
    }
  });
}
