import 'package:nocterm/nocterm.dart';
import 'package:test/test.dart';

void main() {
  group('Shift+Enter newline in TextField', () {
    test('Shift+Enter inserts newline in multiline field', () async {
      await testNocterm(
        'shift+enter newline',
        (tester) async {
          final controller = TextEditingController(text: '');
          String? submittedText;

          await tester.pumpComponent(
            TextField(
              controller: controller,
              width: 40,
              maxLines: 5,
              focused: true,
              onSubmitted: (text) => submittedText = text,
              decoration: const InputDecoration(
                border: BoxBorder(),
                contentPadding: EdgeInsets.symmetric(horizontal: 1),
              ),
              showCursor: true,
              cursorBlinkRate: null,
            ),
          );

          // Type some text
          await tester.enterText('Hello');
          await tester.pump();
          expect(controller.text, equals('Hello'));

          // Send Shift+Enter - should insert newline
          await tester.sendKeyEvent(KeyboardEvent(
            logicalKey: LogicalKey.enter,
            character: '\n',
            modifiers: const ModifierKeys(shift: true),
          ));
          await tester.pump();

          expect(controller.text, equals('Hello\n'));
          expect(submittedText, isNull, reason: 'Should not submit');

          // Type more text after newline
          await tester.enterText('World');
          await tester.pump();
          expect(controller.text, equals('Hello\nWorld'));
        },
        debugPrintAfterPump: true,
      );
    });

    test('Ctrl+Enter inserts newline in multiline field', () async {
      await testNocterm(
        'ctrl+enter newline',
        (tester) async {
          final controller = TextEditingController(text: '');
          String? submittedText;

          await tester.pumpComponent(
            TextField(
              controller: controller,
              width: 40,
              maxLines: 5,
              focused: true,
              onSubmitted: (text) => submittedText = text,
              decoration: const InputDecoration(
                border: BoxBorder(),
                contentPadding: EdgeInsets.symmetric(horizontal: 1),
              ),
              showCursor: true,
              cursorBlinkRate: null,
            ),
          );

          await tester.enterText('Line1');
          await tester.pump();

          // Send Ctrl+Enter - should insert newline
          await tester.sendKeyEvent(KeyboardEvent(
            logicalKey: LogicalKey.enter,
            character: '\n',
            modifiers: const ModifierKeys(ctrl: true),
          ));
          await tester.pump();

          expect(controller.text, equals('Line1\n'));
          expect(submittedText, isNull, reason: 'Should not submit');
        },
      );
    });

    test('Alt+Enter inserts newline in multiline field', () async {
      await testNocterm(
        'alt+enter newline',
        (tester) async {
          final controller = TextEditingController(text: '');
          String? submittedText;

          await tester.pumpComponent(
            TextField(
              controller: controller,
              width: 40,
              maxLines: 5,
              focused: true,
              onSubmitted: (text) => submittedText = text,
              decoration: const InputDecoration(
                border: BoxBorder(),
                contentPadding: EdgeInsets.symmetric(horizontal: 1),
              ),
              showCursor: true,
              cursorBlinkRate: null,
            ),
          );

          await tester.enterText('Line1');
          await tester.pump();

          // Send Alt+Enter - should insert newline
          await tester.sendKeyEvent(KeyboardEvent(
            logicalKey: LogicalKey.enter,
            character: '\n',
            modifiers: const ModifierKeys(alt: true),
          ));
          await tester.pump();

          expect(controller.text, equals('Line1\n'));
          expect(submittedText, isNull, reason: 'Should not submit');
        },
      );
    });

    test('Ctrl+J inserts newline in multiline field', () async {
      await testNocterm(
        'ctrl+j newline',
        (tester) async {
          final controller = TextEditingController(text: '');
          String? submittedText;

          await tester.pumpComponent(
            TextField(
              controller: controller,
              width: 40,
              maxLines: 5,
              focused: true,
              onSubmitted: (text) => submittedText = text,
              decoration: const InputDecoration(
                border: BoxBorder(),
                contentPadding: EdgeInsets.symmetric(horizontal: 1),
              ),
              showCursor: true,
              cursorBlinkRate: null,
            ),
          );

          await tester.enterText('Line1');
          await tester.pump();

          // Send Ctrl+J - universal newline fallback
          await tester.sendKeyEvent(KeyboardEvent(
            logicalKey: LogicalKey.keyJ,
            modifiers: const ModifierKeys(ctrl: true),
          ));
          await tester.pump();

          expect(controller.text, equals('Line1\n'));
          expect(submittedText, isNull, reason: 'Should not submit');
        },
      );
    });

    test('plain Enter still submits', () async {
      await testNocterm(
        'plain enter submits',
        (tester) async {
          final controller = TextEditingController(text: '');
          String? submittedText;

          await tester.pumpComponent(
            TextField(
              controller: controller,
              width: 40,
              maxLines: 5,
              focused: true,
              onSubmitted: (text) => submittedText = text,
              decoration: const InputDecoration(
                border: BoxBorder(),
                contentPadding: EdgeInsets.symmetric(horizontal: 1),
              ),
              showCursor: true,
              cursorBlinkRate: null,
            ),
          );

          await tester.enterText('Hello World');
          await tester.pump();

          // Plain Enter should submit, not insert newline
          await tester.sendEnter();
          await tester.pump();

          expect(submittedText, equals('Hello World'));
          // Text should not have a newline
          expect(controller.text, isNot(contains('\n')));
        },
      );
    });

    test('Shift+Enter does not insert newline in single-line field', () async {
      await testNocterm(
        'shift+enter single line',
        (tester) async {
          final controller = TextEditingController(text: '');

          await tester.pumpComponent(
            TextField(
              controller: controller,
              width: 40,
              maxLines: 1,
              focused: true,
              decoration: const InputDecoration(
                border: BoxBorder(),
                contentPadding: EdgeInsets.symmetric(horizontal: 1),
              ),
              showCursor: true,
              cursorBlinkRate: null,
            ),
          );

          await tester.enterText('Hello');
          await tester.pump();

          // Shift+Enter in single-line field should not insert newline
          await tester.sendKeyEvent(KeyboardEvent(
            logicalKey: LogicalKey.enter,
            character: '\n',
            modifiers: const ModifierKeys(shift: true),
          ));
          await tester.pump();

          expect(controller.text, isNot(contains('\n')));
        },
      );
    });

    test('Ctrl+J does not insert newline in single-line field', () async {
      await testNocterm(
        'ctrl+j single line',
        (tester) async {
          final controller = TextEditingController(text: '');

          await tester.pumpComponent(
            TextField(
              controller: controller,
              width: 40,
              maxLines: 1,
              focused: true,
              decoration: const InputDecoration(
                border: BoxBorder(),
                contentPadding: EdgeInsets.symmetric(horizontal: 1),
              ),
              showCursor: true,
              cursorBlinkRate: null,
            ),
          );

          await tester.enterText('Hello');
          await tester.pump();

          // Ctrl+J in single-line field should not insert newline
          await tester.sendKeyEvent(KeyboardEvent(
            logicalKey: LogicalKey.keyJ,
            modifiers: const ModifierKeys(ctrl: true),
          ));
          await tester.pump();

          expect(controller.text, isNot(contains('\n')));
        },
      );
    });

    test('multiple newlines build a multiline text', () async {
      await testNocterm(
        'multiple newlines',
        (tester) async {
          final controller = TextEditingController(text: '');

          await tester.pumpComponent(
            TextField(
              controller: controller,
              width: 40,
              maxLines: 10,
              focused: true,
              decoration: const InputDecoration(
                border: BoxBorder(),
                contentPadding: EdgeInsets.symmetric(horizontal: 1),
              ),
              showCursor: true,
              cursorBlinkRate: null,
            ),
          );

          // Build a multi-line text
          await tester.enterText('Line 1');
          await tester.sendKeyEvent(KeyboardEvent(
            logicalKey: LogicalKey.enter,
            character: '\n',
            modifiers: const ModifierKeys(shift: true),
          ));
          await tester.enterText('Line 2');
          await tester.sendKeyEvent(KeyboardEvent(
            logicalKey: LogicalKey.keyJ,
            modifiers: const ModifierKeys(ctrl: true),
          ));
          await tester.enterText('Line 3');
          await tester.pump();

          expect(controller.text, equals('Line 1\nLine 2\nLine 3'));
        },
        debugPrintAfterPump: true,
      );
    });
  });
}
