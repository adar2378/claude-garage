import 'package:nocterm/nocterm.dart';
import 'package:test/test.dart';

void main() {
  group('TextField Mouse Interaction', () {
    group('click to position cursor', () {
      test('click positions cursor at correct character', () async {
        await testNocterm(
          'click positions cursor',
          (tester) async {
            final controller = TextEditingController(text: 'Hello World');

            await tester.pumpComponent(
              Container(
                width: 30,
                height: 1,
                child: TextField(
                  controller: controller,
                  focused: true,
                  maxLines: 1,
                  showCursor: true,
                  cursorBlinkRate: null,
                ),
              ),
            );

            // Cursor should start at end of text (offset 11)
            expect(controller.selection.extentOffset, 11);

            // Click at x=0 (first character 'H') should position cursor at 0
            await tester.press(0, 0);
            await tester.release(0, 0);

            expect(controller.selection.isCollapsed, isTrue);
            expect(controller.selection.extentOffset, 0);

            // Click at x=5 (character 'W' in "Hello World")
            await tester.press(5, 0);
            await tester.release(5, 0);

            expect(controller.selection.isCollapsed, isTrue);
            expect(controller.selection.extentOffset, 5);
          },
        );
      });

      test('click at end of text positions cursor at text length', () async {
        await testNocterm(
          'click at end',
          (tester) async {
            final controller = TextEditingController(text: 'Hello');

            await tester.pumpComponent(
              Container(
                width: 30,
                height: 1,
                child: TextField(
                  controller: controller,
                  focused: true,
                  maxLines: 1,
                  showCursor: true,
                  cursorBlinkRate: null,
                ),
              ),
            );

            // Click well past the text - should clamp to text length
            await tester.press(20, 0);
            await tester.release(20, 0);

            expect(controller.selection.isCollapsed, isTrue);
            expect(controller.selection.extentOffset, 5);
          },
        );
      });
    });

    group('click on empty field', () {
      test('click on empty field keeps cursor at 0', () async {
        await testNocterm(
          'click on empty field',
          (tester) async {
            final controller = TextEditingController(text: '');

            await tester.pumpComponent(
              Container(
                width: 30,
                height: 1,
                child: TextField(
                  controller: controller,
                  focused: true,
                  maxLines: 1,
                  showCursor: true,
                  cursorBlinkRate: null,
                ),
              ),
            );

            // Click anywhere on empty field
            await tester.press(5, 0);
            await tester.release(5, 0);

            expect(controller.selection.isCollapsed, isTrue);
            expect(controller.selection.extentOffset, 0);
          },
        );
      });
    });

    group('drag to select', () {
      test('drag selects text range', () async {
        await testNocterm(
          'drag selects text',
          (tester) async {
            final controller = TextEditingController(text: 'Hello World');

            await tester.pumpComponent(
              Container(
                width: 30,
                height: 1,
                child: TextField(
                  controller: controller,
                  focused: true,
                  maxLines: 1,
                  showCursor: true,
                  cursorBlinkRate: null,
                ),
              ),
            );

            // Press at position 0, drag to position 5
            await tester.press(0, 0);
            await tester.sendMouseEvent(const MouseEvent(
              button: MouseButton.left,
              x: 5,
              y: 0,
              pressed: true,
              isMotion: true,
            ));
            await tester.release(5, 0);

            // Selection should span from 0 to 5 ("Hello")
            expect(controller.selection.isCollapsed, isFalse);
            expect(controller.selection.baseOffset, 0);
            expect(controller.selection.extentOffset, 5);
          },
        );
      });

      test('backward drag selects text in reverse', () async {
        await testNocterm(
          'backward drag',
          (tester) async {
            final controller = TextEditingController(text: 'Hello World');

            await tester.pumpComponent(
              Container(
                width: 30,
                height: 1,
                child: TextField(
                  controller: controller,
                  focused: true,
                  maxLines: 1,
                  showCursor: true,
                  cursorBlinkRate: null,
                ),
              ),
            );

            // Press at position 5, drag back to position 0
            await tester.press(5, 0);
            await tester.sendMouseEvent(const MouseEvent(
              button: MouseButton.left,
              x: 0,
              y: 0,
              pressed: true,
              isMotion: true,
            ));
            await tester.release(0, 0);

            // Selection should have base at 5 and extent at 0
            expect(controller.selection.isCollapsed, isFalse);
            expect(controller.selection.baseOffset, 5);
            expect(controller.selection.extentOffset, 0);
          },
        );
      });
    });

    group('double-click to select word', () {
      test('double-click selects word', () async {
        await testNocterm(
          'double-click selects word',
          (tester) async {
            final controller = TextEditingController(text: 'Hello World');

            await tester.pumpComponent(
              Container(
                width: 30,
                height: 1,
                child: TextField(
                  controller: controller,
                  focused: true,
                  maxLines: 1,
                  showCursor: true,
                  cursorBlinkRate: null,
                ),
              ),
            );

            // First click at position 1 (inside "Hello")
            await tester.press(1, 0);
            await tester.release(1, 0);

            // Second click at same position (double-click)
            await tester.press(1, 0);
            await tester.release(1, 0);

            // "Hello" should be selected (base=0, extent=5)
            expect(controller.selection.isCollapsed, isFalse);
            expect(controller.selection.baseOffset, 0);
            expect(controller.selection.extentOffset, 5);
          },
        );
      });

      test('double-click selects second word', () async {
        await testNocterm(
          'double-click second word',
          (tester) async {
            final controller = TextEditingController(text: 'Hello World');

            await tester.pumpComponent(
              Container(
                width: 30,
                height: 1,
                child: TextField(
                  controller: controller,
                  focused: true,
                  maxLines: 1,
                  showCursor: true,
                  cursorBlinkRate: null,
                ),
              ),
            );

            // First click at position 7 (inside "World")
            await tester.press(7, 0);
            await tester.release(7, 0);

            // Second click at same position (double-click)
            await tester.press(7, 0);
            await tester.release(7, 0);

            // "World" should be selected (base=6, extent=11)
            expect(controller.selection.isCollapsed, isFalse);
            expect(controller.selection.baseOffset, 6);
            expect(controller.selection.extentOffset, 11);
          },
        );
      });
    });

    group('click triggers focus', () {
      test('click on unfocused field triggers onFocusChange', () async {
        await testNocterm(
          'click triggers focus',
          (tester) async {
            final controller = TextEditingController(text: 'Hello');
            bool? focusChanged;

            await tester.pumpComponent(
              Container(
                width: 30,
                height: 1,
                child: TextField(
                  controller: controller,
                  focused: false,
                  onFocusChange: (focused) {
                    focusChanged = focused;
                  },
                  maxLines: 1,
                  showCursor: true,
                  cursorBlinkRate: null,
                ),
              ),
            );

            // Click on the unfocused field
            await tester.press(2, 0);
            await tester.release(2, 0);

            // onFocusChange should have been called with true
            expect(focusChanged, isTrue);
          },
        );
      });
    });

    group('multi-line click positioning', () {
      test('click on different lines positions cursor correctly', () async {
        await testNocterm(
          'multiline click',
          (tester) async {
            final controller =
                TextEditingController(text: 'Line 1\nLine 2\nLine 3');

            await tester.pumpComponent(
              Container(
                width: 30,
                height: 5,
                child: TextField(
                  controller: controller,
                  focused: true,
                  maxLines: 5,
                  showCursor: true,
                  cursorBlinkRate: null,
                ),
              ),
            );

            // Click on start of first line
            await tester.press(0, 0);
            await tester.release(0, 0);

            expect(controller.selection.isCollapsed, isTrue);
            expect(controller.selection.extentOffset, 0);

            // Click on second line (y=1), at x=0
            // Line 1 is "Line 1" (6 chars) + newline = offset 7
            await tester.press(0, 1);
            await tester.release(0, 1);

            expect(controller.selection.isCollapsed, isTrue);
            expect(controller.selection.extentOffset, 7);

            // Click on third line (y=2), at x=0
            // "Line 1\n" (7) + "Line 2\n" (7) = offset 14
            await tester.press(0, 2);
            await tester.release(0, 2);

            expect(controller.selection.isCollapsed, isTrue);
            expect(controller.selection.extentOffset, 14);
          },
        );
      });

      test('click at specific position within a line', () async {
        await testNocterm(
          'multiline click position within line',
          (tester) async {
            final controller = TextEditingController(text: 'AAA\nBBBBB\nCC');

            await tester.pumpComponent(
              Container(
                width: 30,
                height: 5,
                child: TextField(
                  controller: controller,
                  focused: true,
                  maxLines: 5,
                  showCursor: true,
                  cursorBlinkRate: null,
                ),
              ),
            );

            // Click on second line at x=3 -> should be offset 4+3=7
            // "AAA\n" = 4 chars, then "BBB" = 3 chars within line 2
            await tester.press(3, 1);
            await tester.release(3, 1);

            expect(controller.selection.isCollapsed, isTrue);
            expect(controller.selection.extentOffset, 7);
          },
        );
      });
    });

    group('click after keyboard selection clears selection', () {
      test('click collapses keyboard selection', () async {
        await testNocterm(
          'click clears selection',
          (tester) async {
            final controller = TextEditingController(text: 'Hello World');

            await tester.pumpComponent(
              Container(
                width: 30,
                height: 1,
                child: TextField(
                  controller: controller,
                  focused: true,
                  maxLines: 1,
                  showCursor: true,
                  cursorBlinkRate: null,
                ),
              ),
            );

            // Select text via keyboard (Ctrl+A)
            await tester.sendKeyEvent(KeyboardEvent(
              logicalKey: LogicalKey.keyA,
              modifiers: ModifierKeys(ctrl: true),
            ));

            // Verify selection is active
            expect(controller.selection.isCollapsed, isFalse);
            expect(controller.selection.baseOffset, 0);
            expect(controller.selection.extentOffset, 11);

            // Click somewhere to collapse the selection
            await tester.press(3, 0);
            await tester.release(3, 0);

            // Selection should now be collapsed at the click position
            expect(controller.selection.isCollapsed, isTrue);
            expect(controller.selection.extentOffset, 3);
          },
        );
      });

      test('click collapses shift-arrow selection', () async {
        await testNocterm(
          'click clears shift-arrow selection',
          (tester) async {
            final controller = TextEditingController(text: 'Hello World');

            await tester.pumpComponent(
              Container(
                width: 30,
                height: 1,
                child: TextField(
                  controller: controller,
                  focused: true,
                  maxLines: 1,
                  showCursor: true,
                  cursorBlinkRate: null,
                ),
              ),
            );

            // Move cursor to position 0
            await tester.sendKey(LogicalKey.home);

            // Select with Shift+Right three times
            for (int i = 0; i < 3; i++) {
              await tester.sendKeyEvent(KeyboardEvent(
                logicalKey: LogicalKey.arrowRight,
                modifiers: ModifierKeys(shift: true),
              ));
            }

            // Verify selection
            expect(controller.selection.isCollapsed, isFalse);
            expect(controller.selection.baseOffset, 0);
            expect(controller.selection.extentOffset, 3);

            // Click at position 7 to collapse
            await tester.press(7, 0);
            await tester.release(7, 0);

            expect(controller.selection.isCollapsed, isTrue);
            expect(controller.selection.extentOffset, 7);
          },
        );
      });
    });

    group('drag across lines in multi-line field', () {
      test('drag select across multiple lines', () async {
        await testNocterm(
          'multi-line drag select',
          (tester) async {
            final controller =
                TextEditingController(text: 'Line 1\nLine 2\nLine 3');

            await tester.pumpComponent(
              Container(
                width: 30,
                height: 5,
                child: TextField(
                  controller: controller,
                  focused: true,
                  maxLines: 5,
                  showCursor: true,
                  cursorBlinkRate: null,
                ),
              ),
            );

            // Press on first line at x=2, drag to second line at x=4
            await tester.press(2, 0);
            await tester.sendMouseEvent(const MouseEvent(
              button: MouseButton.left,
              x: 4,
              y: 1,
              pressed: true,
              isMotion: true,
            ));
            await tester.release(4, 1);

            // Selection should span from position 2 on line 1 to position 4
            // on line 2.
            // Line 1 offset 2 = "ne 1\nLine" -> base=2
            // Line 2 at x=4 = offset 7+4 = 11
            expect(controller.selection.isCollapsed, isFalse);
            expect(controller.selection.baseOffset, 2);
            expect(controller.selection.extentOffset, 11);
          },
        );
      });
    });

    group('edge cases', () {
      test('mouse up clears drag anchor', () async {
        await testNocterm(
          'mouse up clears drag',
          (tester) async {
            final controller = TextEditingController(text: 'Hello World Test');

            await tester.pumpComponent(
              Container(
                width: 30,
                height: 1,
                child: TextField(
                  controller: controller,
                  focused: true,
                  maxLines: 1,
                  showCursor: true,
                  cursorBlinkRate: null,
                ),
              ),
            );

            // Start a drag from position 0 to 3
            await tester.press(0, 0);
            await tester.sendMouseEvent(const MouseEvent(
              button: MouseButton.left,
              x: 3,
              y: 0,
              pressed: true,
              isMotion: true,
            ));
            await tester.release(3, 0);

            // Verify we have a selection
            expect(controller.selection.isCollapsed, isFalse);

            // Click far away from previous click to avoid double-click detection
            // (double-click requires offset difference <= 1)
            await tester.press(10, 0);
            await tester.release(10, 0);

            // Should be collapsed at the new position (not extending old drag)
            expect(controller.selection.isCollapsed, isTrue);
            expect(controller.selection.extentOffset, 10);
          },
        );
      });

      test('wheel events are ignored during mouse interaction', () async {
        await testNocterm(
          'wheel events ignored',
          (tester) async {
            final controller = TextEditingController(text: 'Hello World');

            await tester.pumpComponent(
              Container(
                width: 30,
                height: 1,
                child: TextField(
                  controller: controller,
                  focused: true,
                  maxLines: 1,
                  showCursor: true,
                  cursorBlinkRate: null,
                ),
              ),
            );

            // Position cursor
            await tester.press(3, 0);
            await tester.release(3, 0);
            final cursorAfterClick = controller.selection.extentOffset;

            // Send a wheel event — should not affect cursor
            await tester.sendMouseEvent(const MouseEvent(
              button: MouseButton.wheelDown,
              x: 3,
              y: 0,
              pressed: false,
            ));

            expect(controller.selection.extentOffset, cursorAfterClick);
          },
        );
      });
    });

    group('with decoration', () {
      test('click accounts for border and padding', () async {
        await testNocterm(
          'click with decoration',
          (tester) async {
            final controller = TextEditingController(text: 'Hello');

            await tester.pumpComponent(
              TextField(
                controller: controller,
                focused: true,
                maxLines: 1,
                width: 30,
                showCursor: true,
                cursorBlinkRate: null,
                decoration: const InputDecoration(
                  border: BoxBorder(),
                  contentPadding: EdgeInsets.symmetric(horizontal: 1),
                ),
              ),
            );

            // With border (1 col) + padding (1 col) = 2 col offset to text
            // The RenderTextField is laid out inside the padding/border container.
            // Click at global x=4 should be a few chars into "Hello"
            await tester.press(4, 1);
            await tester.release(4, 1);

            final offset1 = controller.selection.extentOffset;
            expect(controller.selection.isCollapsed, isTrue);

            // Click further right → higher offset
            await tester.press(6, 1);
            await tester.release(6, 1);

            final offset2 = controller.selection.extentOffset;
            expect(controller.selection.isCollapsed, isTrue);
            expect(offset2, greaterThan(offset1));
          },
        );
      });
    });
  });
}
