// Tests for bracketed-paste forwarding (tui/lib/input/paste.dart) —
// tui-key-routing spec "Paste forwarding".
//
// The vendored framework surfaces real bracketed pastes (and long coalesced
// printable runs) as a synthetic Ctrl+V key event with the text parked in
// ClipboardManager. PasteForwarder recovers that text and produces the
// bracketed-paste bytes for the engaged tile's PTY; a genuine Ctrl+V with an
// empty buffer must fall through to encodeKey (raw 0x16) instead.
import 'package:garage_tui/input/encode_key.dart';
import 'package:garage_tui/input/paste.dart';
import 'package:nocterm/nocterm.dart';
import 'package:test/test.dart';

const ctrlV = KeyboardEvent(
  logicalKey: LogicalKey.keyV,
  modifiers: ModifierKeys(ctrl: true),
);

void main() {
  group('wrapBracketedPaste', () {
    test('wraps text in ESC[200~ … ESC[201~', () {
      expect(wrapBracketedPaste('hello'), '\x1b[200~hello\x1b[201~');
    });

    test('embedded newlines stay verbatim inside the guards', () {
      // A multi-line snippet must arrive as ONE paste — no newline may leak
      // outside the guards where the remote app would treat it as Enter.
      const text = 'line one\nline two\r\nline three\n';
      expect(wrapBracketedPaste(text), '\x1b[200~$text\x1b[201~');
    });

    test('empty text still produces a well-formed (empty) paste', () {
      expect(wrapBracketedPaste(''), '\x1b[200~\x1b[201~');
    });
  });

  group('PasteForwarder.recover', () {
    test('synthetic Ctrl+V with buffered text forwards a bracketed paste',
        () {
      final forwarder = PasteForwarder(readClipboard: () => 'pasted\ntext');
      expect(forwarder.recover(ctrlV), '\x1b[200~pasted\ntext\x1b[201~');
    });

    test('non-Ctrl+V events are never treated as pastes', () {
      final forwarder = PasteForwarder(readClipboard: () => 'pasted');
      const plainV =
          KeyboardEvent(logicalKey: LogicalKey.keyV, character: 'v');
      const ctrlC = KeyboardEvent(
        logicalKey: LogicalKey.keyC,
        modifiers: ModifierKeys(ctrl: true),
      );
      expect(forwarder.recover(plainV), isNull);
      expect(forwarder.recover(ctrlC), isNull);
    });

    test('empty buffer falls through — and encodeKey emits raw 0x16', () {
      // Genuine Ctrl+V keypress, nothing buffered: not a paste. The caller
      // continues to encodeKey, which must deliver the literal Ctrl+V byte.
      final emptyForwarder = PasteForwarder(readClipboard: () => '');
      final nullForwarder = PasteForwarder(readClipboard: () => null);
      expect(emptyForwarder.recover(ctrlV), isNull);
      expect(nullForwarder.recover(ctrlV), isNull);
      expect(encodeKey(ctrlV), '\x16');
    });

    test('default reader recovers from the real ClipboardManager buffer',
        () {
      // ClipboardManager.copy is what the framework calls when it converts
      // a paste into the synthetic Ctrl+V (headless-safe: the OSC 52 side
      // effect is wrapped in try/catch upstream).
      ClipboardManager.copy('from the framework buffer');
      // Reset via copy('') — ClipboardManager.clear() writes a raw OSC 52
      // sequence straight to stdout, which pollutes headless test output.
      addTearDown(() => ClipboardManager.copy(''));
      final forwarder = PasteForwarder();
      expect(forwarder.recover(ctrlV),
          '\x1b[200~from the framework buffer\x1b[201~');
    });
  });
}
