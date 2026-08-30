// Regression test for the vendored nocterm debug-key opt-out patch
// (tui/vendor/NOCTERM_VERSION, patch 4).
//
// Upstream hardcodes Ctrl+G as the debug-mode toggle, intercepted in the
// event loop before component dispatch — the app never sees it, which
// collides with garage's Ctrl+G disengage chord. The patch gates the
// intercept on the public static flag `TerminalBinding.debugKeyEnabled`
// (default true = upstream behavior).
//
// The binding cannot be instantiated headless, so — like the batching patch
// (patch 2) — the intercept decision lives in the public top-level
// `debugShouldHandleDebugKey`, which `_handleDebugKeyEvent` delegates to.
// This test drives that function, including against events produced by the
// real InputParser from the raw Ctrl+G byte (0x07).
import 'package:nocterm/src/binding/terminal_binding.dart'
    show TerminalBinding, debugShouldHandleDebugKey;
import 'package:nocterm/src/keyboard/input_event.dart';
import 'package:nocterm/src/keyboard/input_parser.dart';
import 'package:nocterm/src/keyboard/keyboard_event.dart';
import 'package:nocterm/src/keyboard/logical_key.dart';
import 'package:test/test.dart';

/// Parse one stdin chunk exactly like TerminalBinding does.
KeyboardEvent parseSingleKey(List<int> bytes) {
  final parser = InputParser();
  parser.addBytes(bytes);
  final event = parser.parseNext();
  expect(event, isA<KeyboardInputEvent>());
  return (event as KeyboardInputEvent).event;
}

void main() {
  tearDown(() {
    // Never leak flag state between tests (upstream default is true).
    TerminalBinding.debugKeyEnabled = true;
  });

  test('flag defaults to enabled — upstream behavior is unchanged', () {
    expect(TerminalBinding.debugKeyEnabled, isTrue);
    const ctrlG = KeyboardEvent(
      logicalKey: LogicalKey.keyG,
      modifiers: ModifierKeys(ctrl: true),
    );
    expect(debugShouldHandleDebugKey(ctrlG), isTrue,
        reason: 'with the flag on, Ctrl+G is still the debug toggle');
  });

  test('non-Ctrl+G events are never intercepted', () {
    const plainG =
        KeyboardEvent(logicalKey: LogicalKey.keyG, character: 'g');
    const ctrlQ = KeyboardEvent(
      logicalKey: LogicalKey.keyQ,
      modifiers: ModifierKeys(ctrl: true),
    );
    expect(debugShouldHandleDebugKey(plainG), isFalse);
    expect(debugShouldHandleDebugKey(ctrlQ), isFalse);
  });

  test('flag off: Ctrl+G passes through untouched', () {
    TerminalBinding.debugKeyEnabled = false;
    const ctrlG = KeyboardEvent(
      logicalKey: LogicalKey.keyG,
      modifiers: ModifierKeys(ctrl: true),
    );
    expect(debugShouldHandleDebugKey(ctrlG), isFalse,
        reason: 'the app owns Ctrl+G (disengage chord) when opted out');
  });

  test('parser-produced Ctrl+G (raw 0x07) honors the flag both ways', () {
    // 0x07 (BEL) is what a real terminal sends for Ctrl+G.
    final ctrlG = parseSingleKey([0x07]);
    expect(ctrlG.logicalKey, LogicalKey.keyG);
    expect(ctrlG.isControlPressed, isTrue);

    expect(debugShouldHandleDebugKey(ctrlG), isTrue);
    TerminalBinding.debugKeyEnabled = false;
    expect(debugShouldHandleDebugKey(ctrlG), isFalse);
  });
}
