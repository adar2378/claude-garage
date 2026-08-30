import 'package:nocterm/nocterm.dart';
import 'package:test/test.dart';

/// Coverage for RenderTextField.getImeCursorPosition() (added in PR #83),
/// focused on the multiple-consecutive-newline row tracking that PR #78 and
/// PR #83 both fixed.
///
/// The buggy version checked whether the text *up to* the current offset
/// ended with '\n' (`textSoFar.endsWith('\n')`), which can never see the
/// newline that sits AT the line-split position. With consecutive newlines
/// (empty lines) it under-counts characters and reports the cursor one or
/// more rows too low. The fix checks `_text[charCount] == '\n'` instead.
void main() {
  group('RenderTextField.getImeCursorPosition', () {
    RenderTextField? findTextField() {
      RenderTextField? result;
      void visit(Element element) {
        if (result != null) return;
        if (element is RenderObjectElement &&
            element.renderObject is RenderTextField) {
          result = element.renderObject as RenderTextField;
          return;
        }
        element.visitChildren(visit);
      }

      visit(NoctermTestBinding.instance.rootElement!);
      return result;
    }

    test('tracks the cursor row across consecutive newlines', () async {
      await testNocterm(
        'ime cursor consecutive newlines',
        (tester) async {
          // Lines: ['a', '', 'b', 'c'] — an empty line between 'a' and 'b'.
          final controller = TextEditingController(text: 'a\n\nb\nc');

          await tester.pumpComponent(
            TextField(
              controller: controller,
              width: 20,
              maxLines: 6,
              focused: true,
            ),
          );

          final field = findTextField();
          expect(field, isNotNull, reason: 'no RenderTextField found');

          // offset 0 -> 'a' on line 0
          controller.selection = const TextSelection.collapsed(offset: 0);
          await tester.pump();
          final pos0 = field!.getImeCursorPosition();
          expect(pos0, isNotNull);

          // offset 3 -> 'b' on line 2 (one empty line sits between)
          controller.selection = const TextSelection.collapsed(offset: 3);
          await tester.pump();
          final pos3 = field.getImeCursorPosition();
          expect(pos3, isNotNull);

          // offset 5 -> 'c' on line 3
          controller.selection = const TextSelection.collapsed(offset: 5);
          await tester.pump();
          final pos5 = field.getImeCursorPosition();
          expect(pos5, isNotNull);

          // 'b' is two rows below 'a', not three: the empty line counts once.
          expect(pos3!.dy - pos0!.dy, equals(2),
              reason: "'b' should be two rows below 'a' (line index 2)");
          // 'c' is three rows below 'a'.
          expect(pos5!.dy - pos0.dy, equals(3),
              reason: "'c' should be three rows below 'a' (line index 3)");
          // Both 'a' and 'b' sit at the start of their line (column 0).
          expect(pos3.dx, equals(pos0.dx),
              reason: "'b' starts its line, same column as 'a'");
        },
      );
    });
  });
}
