// Regression test for the vendored nocterm input-batching patch
// (tui/vendor/NOCTERM_VERSION, patch 1; test hook = patch 2).
//
// Upstream nocterm collapses a multi-event stdin chunk of "printable" chars
// into one synthetic PasteInputEvent — and counted Enter's `\n` as printable,
// so coalesced keystrokes (event-loop lag, heavy tile output) silently became
// pastes that swallowed Enter. The patch: `\n`/`\r` are never printable, and
// runs shorter than 4 chars stay individual key events.
//
// This drives the real pipeline the terminal binding runs on every stdin
// read: InputParser (addBytes + parseNext loop) → debugBatchCharacterEvents
// (the vendored batching logic, exposed for tests).
import 'package:nocterm/src/binding/terminal_binding.dart'
    show debugBatchCharacterEvents;
import 'package:nocterm/src/keyboard/input_event.dart';
import 'package:nocterm/src/keyboard/input_parser.dart';
import 'package:nocterm/src/keyboard/logical_key.dart';
import 'package:test/test.dart';

/// Parse one stdin chunk exactly like TerminalBinding does, then run the
/// batching step on the parsed events.
List<InputEvent> parseAndBatch(List<int> bytes) {
  final parser = InputParser();
  parser.addBytes(bytes);
  final events = <InputEvent>[];
  InputEvent? event;
  while ((event = parser.parseNext()) != null) {
    events.add(event!);
  }
  return debugBatchCharacterEvents(events);
}

void main() {
  test(
      'coalesced chunk [\\n, h, Ctrl+A] stays three key events — '
      'Enter is never batched into a paste', () {
    // One stdin read carrying Enter (0x0a), 'h' (0x68), Ctrl+A (0x01):
    // the exact shape produced by event-loop lag during heavy output.
    final batched = parseAndBatch([0x0a, 0x68, 0x01]);

    expect(batched, hasLength(3));
    expect(batched.whereType<PasteInputEvent>(), isEmpty,
        reason: 'a short coalesced run must never become a synthetic paste');
    expect(batched, everyElement(isA<KeyboardInputEvent>()));

    final keys = batched.cast<KeyboardInputEvent>().toList();

    // Enter, unmodified.
    expect(keys[0].event.logicalKey, LogicalKey.enter);
    expect(keys[0].event.isControlPressed, isFalse);

    // Plain 'h'.
    expect(keys[1].event.logicalKey, LogicalKey.keyH);
    expect(keys[1].event.character, 'h');
    expect(keys[1].event.isControlPressed, isFalse);

    // Ctrl+A.
    expect(keys[2].event.logicalKey, LogicalKey.keyA);
    expect(keys[2].event.isControlPressed, isTrue);
  });

  test('a 6-printable-char chunk still batches into one PasteInputEvent', () {
    // Long printable runs in one read really are pastes (e.g. Warp drag-drop
    // without bracketed paste) — the patch must not break that.
    final batched = parseAndBatch('hello!'.codeUnits);

    expect(batched, hasLength(1));
    expect(batched.single, isA<PasteInputEvent>());
    expect((batched.single as PasteInputEvent).text, 'hello!');
  });
}
