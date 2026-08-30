import 'package:nocterm/nocterm.dart';
import 'package:nocterm/src/components/render_paragraph.dart';
import 'package:quiver/strings.dart' hide isEmpty, isNotEmpty;
import 'package:test/test.dart' hide isEmpty, isNotEmpty;

void main() {
  group('RenderParagraph Selection', () {
    test('single RichText selection', () async {
      await testNocterm(
        'single rich text selection',
        (tester) async {
          String? completed;

          await tester.pumpComponent(
            Container(
              width: 30,
              height: 4,
              child: SelectionArea(
                onSelectionCompleted: (text) => completed = text,
                child: RichText(
                  text: const TextSpan(
                    children: [
                      TextSpan(text: 'Hello '),
                      TextSpan(
                        text: 'World',
                        style: TextStyle(fontWeight: FontWeight.bold),
                      ),
                    ],
                  ),
                ),
              ),
            ),
          );

          // Select from start to end of "Hello World"
          await tester.press(0, 0);
          await tester.sendMouseEvent(const MouseEvent(
            button: MouseButton.left,
            x: 11,
            y: 0,
            pressed: true,
            isMotion: true,
          ));
          await tester.release(11, 0);

          expect(completed, equals('Hello World'));
        },
      );
    });

    test('partial selection within styled spans', () async {
      await testNocterm(
        'partial styled selection',
        (tester) async {
          String? completed;

          await tester.pumpComponent(
            Container(
              width: 30,
              height: 4,
              child: SelectionArea(
                onSelectionCompleted: (text) => completed = text,
                child: RichText(
                  text: const TextSpan(
                    children: [
                      TextSpan(
                        text: 'Red',
                        style: TextStyle(color: Colors.red),
                      ),
                      TextSpan(
                        text: 'Green',
                        style: TextStyle(color: Colors.green),
                      ),
                      TextSpan(
                        text: 'Blue',
                        style: TextStyle(color: Colors.blue),
                      ),
                    ],
                  ),
                ),
              ),
            ),
          );

          // Select "dGree" which spans across Red and Green
          await tester.press(2, 0);
          await tester.sendMouseEvent(const MouseEvent(
            button: MouseButton.left,
            x: 7,
            y: 0,
            pressed: true,
            isMotion: true,
          ));
          await tester.release(7, 0);

          expect(completed, equals('dGree'));
        },
      );
    });

    test('multi-widget selection with RichText and Text', () async {
      await testNocterm(
        'multi-widget selection',
        (tester) async {
          String? completed;

          await tester.pumpComponent(
            Container(
              width: 30,
              height: 4,
              child: SelectionArea(
                onSelectionCompleted: (text) => completed = text,
                child: Column(
                  crossAxisAlignment: CrossAxisAlignment.start,
                  children: [
                    const Text('Plain'),
                    RichText(
                      text: const TextSpan(
                        children: [
                          TextSpan(text: 'Styled '),
                          TextSpan(
                            text: 'Text',
                            style: TextStyle(fontWeight: FontWeight.bold),
                          ),
                        ],
                      ),
                    ),
                  ],
                ),
              ),
            ),
          );

          // Select from "Plain" to "Styled Text"
          await tester.press(0, 0);
          await tester.sendMouseEvent(const MouseEvent(
            button: MouseButton.left,
            x: 11,
            y: 1,
            pressed: true,
            isMotion: true,
          ));
          await tester.release(11, 1);

          expect(completed, isNotBlank);
          expect(completed, contains('Plain'));
          expect(completed, contains('Styled Text'));
        },
      );
    });

    test('wrapped RichText selection', () async {
      await testNocterm(
        'wrapped rich text selection',
        (tester) async {
          String? completed;

          await tester.pumpComponent(
            Container(
              width: 6,
              height: 5,
              child: SelectionArea(
                onSelectionCompleted: (text) => completed = text,
                child: RichText(
                  text: const TextSpan(
                    children: [
                      TextSpan(text: 'Hello '),
                      TextSpan(
                        text: 'World',
                        style: TextStyle(fontWeight: FontWeight.bold),
                      ),
                    ],
                  ),
                ),
              ),
            ),
          );

          // Select all text by dragging from start to end (across wrapped lines)
          await tester.press(0, 0);
          await tester.sendMouseEvent(const MouseEvent(
            button: MouseButton.left,
            x: 6,
            y: 2,
            pressed: true,
            isMotion: true,
          ));
          await tester.release(6, 2);

          expect(completed, isNotBlank);
          // With width 6, text wraps so "Hello " on line 0, "World" on line 1
          expect(completed, contains('Hello'));
        },
      );
    });

    test('selection with multiline content', () async {
      await testNocterm(
        'multiline content selection',
        (tester) async {
          String? completed;

          await tester.pumpComponent(
            Container(
              width: 30,
              height: 4,
              child: SelectionArea(
                onSelectionCompleted: (text) => completed = text,
                child: RichText(
                  text: const TextSpan(
                    children: [
                      TextSpan(text: 'Line1\n'),
                      TextSpan(
                        text: 'Line2',
                        style: TextStyle(color: Colors.red),
                      ),
                    ],
                  ),
                ),
              ),
            ),
          );

          // Select both lines
          await tester.press(0, 0);
          await tester.sendMouseEvent(const MouseEvent(
            button: MouseButton.left,
            x: 5,
            y: 1,
            pressed: true,
            isMotion: true,
          ));
          await tester.release(5, 1);

          expect(completed, isNotBlank);
          expect(completed, contains('Line1'));
          expect(completed, contains('Line2'));
        },
      );
    });

    test('backward selection in RichText', () async {
      await testNocterm(
        'backward selection',
        (tester) async {
          String? completed;

          await tester.pumpComponent(
            Container(
              width: 30,
              height: 4,
              child: SelectionArea(
                onSelectionCompleted: (text) => completed = text,
                child: RichText(
                  text: const TextSpan(
                    children: [
                      TextSpan(text: 'Hello '),
                      TextSpan(
                        text: 'World',
                        style: TextStyle(fontWeight: FontWeight.bold),
                      ),
                    ],
                  ),
                ),
              ),
            ),
          );

          // Select backward from end to beginning
          await tester.press(11, 0);
          await tester.sendMouseEvent(const MouseEvent(
            button: MouseButton.left,
            x: 0,
            y: 0,
            pressed: true,
            isMotion: true,
          ));
          await tester.release(0, 0);

          expect(completed, equals('Hello World'));
        },
      );
    });

    test('selection clears when RichText content changes', () async {
      await testNocterm(
        'selection clears on change',
        (tester) async {
          String? lastChanged;

          await tester.pumpComponent(
            Container(
              width: 30,
              height: 4,
              child: SelectionArea(
                onSelectionChanged: (text) => lastChanged = text,
                child: RichText(
                  text: const TextSpan(text: 'Original'),
                ),
              ),
            ),
          );

          // Make a selection
          await tester.press(0, 0);
          await tester.sendMouseEvent(const MouseEvent(
            button: MouseButton.left,
            x: 5,
            y: 0,
            pressed: true,
            isMotion: true,
          ));
          await tester.release(5, 0);

          expect(lastChanged, isNotBlank);

          // Change the content
          await tester.pumpComponent(
            Container(
              width: 30,
              height: 4,
              child: SelectionArea(
                onSelectionChanged: (text) => lastChanged = text,
                child: RichText(
                  text: const TextSpan(text: 'Changed'),
                ),
              ),
            ),
          );

          // Make a new empty selection to trigger change callback
          await tester.press(0, 0);
          await tester.release(0, 0);

          expect(lastChanged, equals(''));
        },
      );
    });

    test('multiple RichText widgets in selection', () async {
      await testNocterm(
        'multiple rich text selection',
        (tester) async {
          String? completed;

          await tester.pumpComponent(
            Container(
              width: 30,
              height: 5,
              child: SelectionArea(
                onSelectionCompleted: (text) => completed = text,
                child: Column(
                  crossAxisAlignment: CrossAxisAlignment.start,
                  children: [
                    RichText(
                      text: const TextSpan(
                        children: [
                          TextSpan(text: 'First '),
                          TextSpan(
                            text: 'Line',
                            style: TextStyle(color: Colors.red),
                          ),
                        ],
                      ),
                    ),
                    RichText(
                      text: const TextSpan(
                        children: [
                          TextSpan(text: 'Second '),
                          TextSpan(
                            text: 'Line',
                            style: TextStyle(color: Colors.blue),
                          ),
                        ],
                      ),
                    ),
                  ],
                ),
              ),
            ),
          );

          // Select across both RichText widgets
          await tester.press(0, 0);
          await tester.sendMouseEvent(const MouseEvent(
            button: MouseButton.left,
            x: 11,
            y: 1,
            pressed: true,
            isMotion: true,
          ));
          await tester.release(11, 1);

          expect(completed, isNotBlank);
          expect(completed, contains('First Line'));
          expect(completed, contains('Second Line'));
        },
      );
    });

    test('RichText selectableText returns plain text', () async {
      await testNocterm(
        'selectableText getter',
        (tester) async {
          await tester.pumpComponent(
            RichText(
              text: const TextSpan(
                children: [
                  TextSpan(text: 'Hello '),
                  TextSpan(
                    text: 'World',
                    style: TextStyle(fontWeight: FontWeight.bold),
                  ),
                ],
              ),
            ),
          );

          RenderParagraph? renderParagraph;
          void findRenderParagraph(Element element) {
            if (element is RenderObjectElement &&
                element.renderObject is RenderParagraph) {
              renderParagraph = element.renderObject as RenderParagraph;
              return;
            }
            element.visitChildren(findRenderParagraph);
          }

          findRenderParagraph(NoctermTestBinding.instance.rootElement!);
          expect(renderParagraph, isNotNull);
          expect(renderParagraph!.selectableText, equals('Hello World'));
        },
      );
    });

    test('RichText selectableLayout returns layout result', () async {
      await testNocterm(
        'selectableLayout getter',
        (tester) async {
          await tester.pumpComponent(
            Container(
              width: 20,
              child: RichText(
                text: const TextSpan(
                  text: 'Hello World',
                ),
              ),
            ),
          );

          RenderParagraph? renderParagraph;
          void findRenderParagraph(Element element) {
            if (element is RenderObjectElement &&
                element.renderObject is RenderParagraph) {
              renderParagraph = element.renderObject as RenderParagraph;
              return;
            }
            element.visitChildren(findRenderParagraph);
          }

          findRenderParagraph(NoctermTestBinding.instance.rootElement!);
          expect(renderParagraph, isNotNull);
          expect(renderParagraph!.selectableLayout, isNotNull);
          expect(
              renderParagraph!.selectableLayout!.lines.length, greaterThan(0));
        },
      );
    });
  });
}
