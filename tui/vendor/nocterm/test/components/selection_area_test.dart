import 'package:nocterm/nocterm.dart';
import 'package:nocterm/src/components/selection_state.dart';
import 'package:quiver/strings.dart' hide isEmpty;
import 'package:test/test.dart' hide isEmpty;

void main() {
  group('SelectionArea', () {
    test('selection completion inserts newline when moving to a new row',
        () async {
      await testNocterm(
        'selection completion',
        (tester) async {
          String? lastChanged;
          String? completed;

          await tester.pumpComponent(
            Container(
              width: 20,
              height: 4,
              child: SelectionArea(
                onSelectionChanged: (text) => lastChanged = text,
                onSelectionCompleted: (text) => completed = text,
                child: Column(
                  crossAxisAlignment: CrossAxisAlignment.start,
                  children: const [
                    Text('Hello'),
                    Text('World'),
                  ],
                ),
              ),
            ),
          );

          // Drag from inside "Hello" to inside "World".
          await tester.press(1, 0);
          await tester.sendMouseEvent(const MouseEvent(
            button: MouseButton.left,
            x: 2,
            y: 0,
            pressed: true,
            isMotion: true,
          ));
          await tester.sendMouseEvent(const MouseEvent(
            button: MouseButton.left,
            x: 3,
            y: 1,
            pressed: true,
            isMotion: true,
          ));
          await tester.release(3, 1);

          expect(lastChanged, equals('ello\nWor'));
          expect(completed, equals('ello\nWor'));
        },
      );
    });

    test('clears selection when text content changes', () async {
      await testNocterm(
        'clears selection on text change',
        (tester) async {
          String? lastChanged;

          await tester.pumpComponent(
            Container(
              width: 20,
              height: 4,
              child: SelectionArea(
                onSelectionChanged: (text) => lastChanged = text,
                child: const Text('Hello'),
              ),
            ),
          );

          // Select part of "Hello"
          await tester.press(1, 0);
          await tester.sendMouseEvent(const MouseEvent(
            button: MouseButton.left,
            x: 4,
            y: 0,
            pressed: true,
            isMotion: true,
          ));
          await tester.release(4, 0);

          expect(lastChanged, equals('ell'));

          // Change text content — selection should be cleared by the
          // RenderText.text setter calling clearSelection().
          await tester.pumpComponent(
            Container(
              width: 20,
              height: 4,
              child: SelectionArea(
                onSelectionChanged: (text) => lastChanged = text,
                child: const Text('Goodbye'),
              ),
            ),
          );

          // Press and immediately release at origin to trigger a new
          // selection pass that sees the cleared state.
          await tester.press(0, 0);
          await tester.release(0, 0);

          expect(lastChanged, equals(''));
        },
      );
    });

    test('starts selection when pressing on whitespace and dragging into text',
        () async {
      await testNocterm(
        'drag from whitespace',
        (tester) async {
          String? lastChanged;

          await tester.pumpComponent(
            Container(
              width: 20,
              height: 4,
              child: SelectionArea(
                onSelectionChanged: (text) => lastChanged = text,
                child: Column(
                  crossAxisAlignment: CrossAxisAlignment.start,
                  children: const [
                    SizedBox(height: 1), // row 0: non-selectable whitespace
                    Text('Hello'), // row 1: selectable text
                  ],
                ),
              ),
            ),
          );

          // Press at (1, 0) — whitespace, no selectable hit.
          await tester.press(1, 0);

          // Drag into "Hello" at row 1 — lazy anchor sets here.
          await tester.sendMouseEvent(const MouseEvent(
            button: MouseButton.left,
            x: 1,
            y: 1,
            pressed: true,
            isMotion: true,
          ));

          // Extend selection within "Hello".
          await tester.sendMouseEvent(const MouseEvent(
            button: MouseButton.left,
            x: 4,
            y: 1,
            pressed: true,
            isMotion: true,
          ));

          await tester.release(4, 1);

          // Selection should cover part of "Hello".
          expect(lastChanged, isNotBlank);
          expect(lastChanged!.length, greaterThan(0));
        },
      );
    });

    test('selection crosses a non-selectable gap between widgets', () async {
      await testNocterm(
        'cross boundary selection',
        (tester) async {
          String? completed;

          await tester.pumpComponent(
            Container(
              width: 20,
              height: 5,
              child: SelectionArea(
                onSelectionCompleted: (text) => completed = text,
                child: Column(
                  crossAxisAlignment: CrossAxisAlignment.start,
                  children: const [
                    Text('Top'), // row 0
                    SizedBox(height: 1), // row 1: gap
                    Text('Bottom'), // row 2
                  ],
                ),
              ),
            ),
          );

          // Press inside "Top" at row 0, drag across gap into "Bottom".
          await tester.press(0, 0);
          await tester.sendMouseEvent(const MouseEvent(
            button: MouseButton.left,
            x: 3,
            y: 2,
            pressed: true,
            isMotion: true,
          ));
          await tester.release(3, 2);

          // Should include text from both "Top" and "Bottom".
          expect(completed, isNotBlank);
          expect(completed, contains('Top'));
          expect(completed, contains('Bot'));
        },
      );
    });

    test('selection updates on mouse release position', () async {
      await testNocterm(
        'selection release update',
        (tester) async {
          String? completed;

          await tester.pumpComponent(
            Container(
              width: 10,
              height: 2,
              child: SelectionArea(
                onSelectionCompleted: (text) => completed = text,
                child: const Text('Hello'),
              ),
            ),
          );

          // Press near "e", drag slightly, then release farther right
          // without an intermediate move event.
          await tester.press(1, 0);
          await tester.sendMouseEvent(const MouseEvent(
            button: MouseButton.left,
            x: 2,
            y: 0,
            pressed: true,
            isMotion: true,
          ));
          await tester.release(5, 0);

          expect(completed, equals('ello'));
        },
      );
    });

    test('drag below last line clamps selection to end', () async {
      await testNocterm(
        'drag below last line',
        (tester) async {
          String? completed;

          await tester.pumpComponent(
            Container(
              width: 20,
              height: 2,
              child: SelectionArea(
                onSelectionCompleted: (text) => completed = text,
                child: Column(
                  crossAxisAlignment: CrossAxisAlignment.start,
                  children: const [
                    Text('First'),
                    Text('Second'),
                  ],
                ),
              ),
            ),
          );

          await tester.press(0, 0);
          await tester.sendMouseEvent(const MouseEvent(
            button: MouseButton.left,
            x: 10,
            y: 5,
            pressed: true,
            isMotion: true,
          ));
          await tester.release(10, 5);

          expect(completed, equals('First\nSecond'));
        },
      );
    });

    test('reanchors when anchored selectable is replaced', () async {
      await testNocterm(
        'reanchor on replace',
        (tester) async {
          String? lastChanged;

          await tester.pumpComponent(
            Container(
              width: 20,
              height: 3,
              child: _SelectionRebuildHarness(
                onSelectionChanged: (text) => lastChanged = text,
              ),
            ),
          );

          // Start selection on the first line.
          await tester.press(1, 0);
          await tester.sendMouseEvent(const MouseEvent(
            button: MouseButton.left,
            x: 3,
            y: 0,
            pressed: true,
            isMotion: true,
          ));

          // Rebuild with a different key for the first line (anchor removed).
          final state = _SelectionRebuildHarness.lastState!;
          state.swapFirstKey();
          await tester.pump();

          // Move to the second line to trigger reanchor logic.
          await tester.sendMouseEvent(const MouseEvent(
            button: MouseButton.left,
            x: 2,
            y: 1,
            pressed: true,
            isMotion: true,
          ));
          await tester.release(2, 1);

          expect(lastChanged, isNotBlank);
        },
      );
    });

    test('selectionColor setter updates cached selectables', () async {
      await testNocterm(
        'selectionColor setter',
        (tester) async {
          RenderSelectionArea? renderObject;
          String? lastChanged;

          await tester.pumpComponent(
            Container(
              width: 20,
              height: 2,
              child: _RenderSelectionAreaHarness(
                onCreated: (ro) => renderObject = ro,
                onSelectionChanged: (text) => lastChanged = text,
                selectionColor: Colors.red,
                child: const Text('Hello'),
              ),
            ),
          );

          await tester.press(1, 0);
          await tester.sendMouseEvent(const MouseEvent(
            button: MouseButton.left,
            x: 4,
            y: 0,
            pressed: true,
            isMotion: true,
          ));
          await tester.release(4, 0);

          expect(lastChanged, isNotBlank);

          final current = renderObject!.selectionColor;
          expect(current, equals(Colors.red));

          renderObject!.selectionColor = Colors.green;
          expect(renderObject!.selectionColor, equals(Colors.green));
        },
      );
    });

    test('onEnter clears left button pressed state', () async {
      await testNocterm(
        'onEnter clears left button pressed',
        (tester) async {
          String? lastChanged;

          await tester.pumpComponent(
            Container(
              width: 10,
              height: 2,
              child: _RenderSelectionAreaHarness(
                onSelectionChanged: (text) => lastChanged = text,
                child: const Text('Hello'),
              ),
            ),
          );

          // Move outside first to establish a prior position.
          await tester.sendMouseEvent(const MouseEvent(
            button: MouseButton.left,
            x: 20,
            y: 5,
            pressed: false,
            isMotion: true,
          ));

          // Enter without left button pressed.
          await tester.sendMouseEvent(const MouseEvent(
            button: MouseButton.left,
            x: 1,
            y: 0,
            pressed: false,
            isMotion: true,
          ));

          expect(lastChanged ?? '', equals(''));
        },
      );
    });

    test('onHover with left button down triggers selection start', () async {
      await testNocterm(
        'hover left button down',
        (tester) async {
          String? lastChanged;

          await tester.pumpComponent(
            Container(
              width: 10,
              height: 2,
              child: _RenderSelectionAreaHarness(
                onSelectionChanged: (text) => lastChanged = text,
                child: const Text('Hello'),
              ),
            ),
          );

          // Hover with left button reported as down.
          await tester.sendMouseEvent(const MouseEvent(
            button: MouseButton.left,
            x: 1,
            y: 0,
            pressed: false,
            isMotion: true,
            buttons: {MouseButton.left},
          ));

          // Drag to update selection.
          await tester.sendMouseEvent(const MouseEvent(
            button: MouseButton.left,
            x: 3,
            y: 0,
            pressed: true,
            isMotion: true,
          ));
          await tester.release(3, 0);

          expect(lastChanged, isNotNull);
        },
      );
    });

    test('onExit finalizes selection when leaving region', () async {
      await testNocterm(
        'exit finalizes selection',
        (tester) async {
          String? completed;

          await tester.pumpComponent(
            Container(
              width: 10,
              height: 2,
              child: _RenderSelectionAreaHarness(
                onSelectionCompleted: (text) => completed = text,
                child: const Text('Hello'),
              ),
            ),
          );

          await tester.sendMouseEvent(const MouseEvent(
            button: MouseButton.left,
            x: 1,
            y: 0,
            pressed: false,
            isMotion: true,
          ));

          await tester.press(1, 0);
          await tester.sendMouseEvent(const MouseEvent(
            button: MouseButton.left,
            x: 3,
            y: 0,
            pressed: true,
            isMotion: true,
          ));

          // Exit the region with the button released.
          await tester.sendMouseEvent(const MouseEvent(
            button: MouseButton.left,
            x: 20,
            y: 5,
            pressed: false,
            isMotion: false,
          ));

          expect(completed, isNotNull);
        },
      );
    });

    test('anchor context removed clears selection', () async {
      await testNocterm(
        'anchor context removed',
        (tester) async {
          String? lastChanged;

          await tester.pumpComponent(
            Container(
              width: 20,
              height: 3,
              child: _SelectionAreaSwapHarness(
                onSelectionChanged: (text) => lastChanged = text,
              ),
            ),
          );

          await tester.press(0, 0);
          await tester.sendMouseEvent(const MouseEvent(
            button: MouseButton.left,
            x: 4,
            y: 1,
            pressed: true,
            isMotion: true,
          ));

          // Swap the child while keeping the same SelectionArea render object.
          final state = _SelectionAreaSwapHarness.lastState!;
          state.swapToColumn();
          await tester.pump();

          // Move to trigger selection update with stale anchor context.
          await tester.sendMouseEvent(const MouseEvent(
            button: MouseButton.left,
            x: 2,
            y: 0,
            pressed: true,
            isMotion: true,
          ));
          await tester.release(2, 0);

          expect(lastChanged, equals(''));
        },
      );
    });

    test('onExit handles left button release when invoked directly', () async {
      await testNocterm(
        'onExit direct',
        (tester) async {
          RenderSelectionArea? renderObject;
          String? completed;

          await tester.pumpComponent(
            Container(
              width: 10,
              height: 2,
              child: _RenderSelectionAreaHarness(
                onCreated: (ro) => renderObject = ro,
                onSelectionCompleted: (text) => completed = text,
                child: const Text('Hello'),
              ),
            ),
          );

          final annotation = renderObject!.annotation!;
          annotation.onEnter?.call(const MouseEvent(
            button: MouseButton.left,
            x: 1,
            y: 0,
            pressed: true,
          ));
          annotation.onExit?.call(const MouseEvent(
            button: MouseButton.left,
            x: 20,
            y: 5,
            pressed: false,
          ));

          expect(completed, isNotNull);
        },
      );
    });

    test('dispose cleans drag state when unmounted mid-drag', () async {
      await testNocterm(
        'dispose mid-drag cleanup',
        (tester) async {
          await tester.pumpComponent(
            Container(
              width: 10,
              height: 2,
              child: const _SelectionAreaUnmountHarness(),
            ),
          );

          await tester.press(1, 0);
          expect(SelectionDragState.isActive, isTrue);

          final state = _SelectionAreaUnmountHarness.lastState!;
          state.unmountSelectionArea();
          await tester.pump();

          expect(SelectionDragState.isActive, isFalse);
        },
      );
    });

    test('ListView items with multiple selectables use positional sort',
        () async {
      await testNocterm(
        'listview positional sort',
        (tester) async {
          String? completed;

          await tester.pumpComponent(
            Container(
              width: 30,
              height: 3,
              child: SelectionArea(
                onSelectionCompleted: (text) => completed = text,
                child: ListView.builder(
                  itemCount: 2,
                  itemBuilder: (context, index) => Row(
                    children: [
                      Text('A$index'),
                      Text('B$index'),
                    ],
                  ),
                ),
              ),
            ),
          );

          await tester.press(0, 0);
          await tester.sendMouseEvent(const MouseEvent(
            button: MouseButton.left,
            x: 4,
            y: 1,
            pressed: true,
            isMotion: true,
          ));
          await tester.release(4, 1);

          expect(completed, isNotBlank);
        },
      );
    });

    test('ListView returns to lazy mode after selection drag ends', () async {
      await testNocterm(
        'listview lazy restore',
        (tester) async {
          await tester.pumpComponent(
            Container(
              width: 20,
              height: 3,
              child: SelectionArea(
                child: ListView.builder(
                  itemCount: 200,
                  lazy: true,
                  itemBuilder: (context, index) => Text('Item$index'),
                ),
              ),
            ),
          );

          RenderListViewport? viewport;
          void findViewport(Element element) {
            if (element is RenderObjectElement &&
                element.renderObject is RenderListViewport) {
              viewport = element.renderObject as RenderListViewport;
              return;
            }
            element.visitChildren(findViewport);
          }

          findViewport(NoctermTestBinding.instance.rootElement!);
          expect(viewport, isNotNull);

          final initialCount = tester.findAllComponents<Text>().length;

          SelectionDragState.begin();
          SelectionDragState.updateRange(viewport!, 0, 120);
          await tester.pump();

          final duringDragCount = tester.findAllComponents<Text>().length;
          expect(duringDragCount, greaterThan(initialCount));

          SelectionDragState.end();
          await tester.pump();

          final afterEndCount = tester.findAllComponents<Text>().length;
          expect(afterEndCount, lessThanOrEqualTo(initialCount));
        },
      );
    });

    test('nearest selectable uses left-side distance', () async {
      await testNocterm(
        'nearest selectable left',
        (tester) async {
          String? lastChanged;

          await tester.pumpComponent(
            Container(
              width: 10,
              height: 2,
              child: SelectionArea(
                onSelectionChanged: (text) => lastChanged = text,
                child: const Padding(
                  padding: EdgeInsets.only(left: 3),
                  child: Text('Hello'),
                ),
              ),
            ),
          );

          await tester.press(4, 0);
          await tester.sendMouseEvent(const MouseEvent(
            button: MouseButton.left,
            x: 1,
            y: 0,
            pressed: true,
            isMotion: true,
          ));
          await tester.release(1, 0);

          expect(lastChanged, isNotNull);
        },
      );
    });

    test('multiline selection inserts newlines', () async {
      await testNocterm(
        'multiline newlines',
        (tester) async {
          String? completed;

          await tester.pumpComponent(
            Container(
              width: 10,
              height: 3,
              child: SelectionArea(
                onSelectionCompleted: (text) => completed = text,
                child: const Text('AA\nBB'),
              ),
            ),
          );

          await tester.press(0, 0);
          await tester.sendMouseEvent(const MouseEvent(
            button: MouseButton.left,
            x: 3,
            y: 1,
            pressed: true,
            isMotion: true,
          ));
          await tester.release(3, 1);

          expect(completed, equals('AA\nBB'));
        },
      );
    });

    test('multiline selection inserts spaces for same-row widgets', () async {
      await testNocterm(
        'multiline spaces',
        (tester) async {
          RenderSelectionArea? renderObject;

          await tester.pumpComponent(
            Container(
              width: 20,
              height: 3,
              child: _RenderSelectionAreaHarness(
                onCreated: (ro) => renderObject = ro,
                child: const Text('AA\nBB'),
              ),
            ),
          );

          // Seed the buffer with a multi-line selection and append another
          // multi-line selectable on the same row to force a space separator.
          final appended = renderObject!.debugAppendSelectedText(
            initial: 'AA\nBB',
            text: 'CC\nDD',
            lines: const ['CC', 'DD'],
            start: 0,
            end: 5,
            topRow: 0,
            height: 2,
            lastBottomRow: 1,
          );

          expect(appended, equals('AA\nBB CC\nDD'));
        },
      );
    });

    test('excludes offscreen text from selection output', () async {
      await testNocterm(
        'offscreen exclusion',
        (tester) async {
          String? completed;

          await tester.pumpComponent(
            Container(
              width: 20,
              height: 4,
              child: SelectionArea(
                onSelectionCompleted: (text) => completed = text,
                child: Stack(
                  children: [
                    Positioned(
                      left: 0,
                      top: 0,
                      child: const Text('Visible'),
                    ),
                    Positioned(
                      left: 0,
                      top: -1,
                      child: const Text('Hidden'),
                    ),
                  ],
                ),
              ),
            ),
          );

          // Select everything by dragging across the visible area
          await tester.press(0, 0);
          await tester.sendMouseEvent(const MouseEvent(
            button: MouseButton.left,
            x: 10,
            y: 0,
            pressed: true,
            isMotion: true,
          ));
          await tester.release(10, 0);

          // "Hidden" is at top=-1 which is above the SelectionArea's
          // bounds (starts at row 0), so it should be excluded.
          expect(completed, isNotBlank);
          expect(completed, contains('Visible'));
          expect(completed, isNot(contains('Hidden')));
        },
      );
    });

    // --- New coverage tests ---

    test('backward selection across three widgets', () async {
      await testNocterm(
        'backward selection',
        (tester) async {
          String? completed;

          await tester.pumpComponent(
            Container(
              width: 20,
              height: 5,
              child: SelectionArea(
                onSelectionCompleted: (text) => completed = text,
                child: Column(
                  crossAxisAlignment: CrossAxisAlignment.start,
                  children: const [
                    Text('Alpha'),
                    Text('Bravo'),
                    Text('Charlie'),
                  ],
                ),
              ),
            ),
          );

          // Press on row 2 (Charlie), drag backward to row 0 (Alpha).
          await tester.press(3, 2);
          await tester.sendMouseEvent(const MouseEvent(
            button: MouseButton.left,
            x: 1,
            y: 0,
            pressed: true,
            isMotion: true,
          ));
          await tester.release(1, 0);

          // Backward: anchor=Charlie[3], focus=Alpha[1].
          // Alpha: forward=false → range [focus.offset, len) = [1, 5) = "lpha"
          // Bravo: intermediate → fully selected = "Bravo"
          // Charlie: forward=false → range [0, anchor.offset) = [0, 3) = "Cha"
          expect(completed, isNotBlank);
          expect(completed, contains('lpha'));
          expect(completed, contains('Bravo'));
          expect(completed, contains('Cha'));
        },
      );
    });

    test('horizontal flex does not expand selection to sibling row items',
        () async {
      await testNocterm(
        'row group expansion',
        (tester) async {
          String? completed;

          await tester.pumpComponent(
            Container(
              width: 20,
              height: 4,
              child: SelectionArea(
                onSelectionCompleted: (text) => completed = text,
                child: Column(
                  crossAxisAlignment: CrossAxisAlignment.start,
                  children: [
                    Row(children: const [Text('AA'), Text(' BB')]),
                    Row(children: const [Text('CC'), Text(' DD')]),
                  ],
                ),
              ),
            ),
          );

          // Press on first row's first text, drag to second row.
          await tester.press(1, 0);
          await tester.sendMouseEvent(const MouseEvent(
            button: MouseButton.left,
            x: 1,
            y: 1,
            pressed: true,
            isMotion: true,
          ));
          await tester.release(1, 1);

          // Selection should include only the contiguous range between anchor
          // and focus, without pulling in sibling row items (e.g. " DD").
          expect(completed, isNotBlank);
          expect(completed, contains('A'));
          expect(completed, contains('BB'));
          expect(completed, contains('C'));
          expect(completed, isNot(contains('DD')));
        },
      );
    });

    test('selectionColor same-value early return and actual update', () async {
      await testNocterm(
        'selection color update',
        (tester) async {
          String? lastChanged;

          // Pump with red selection color.
          await tester.pumpComponent(
            Container(
              width: 20,
              height: 4,
              child: SelectionArea(
                selectionColor: Colors.red,
                onSelectionChanged: (text) => lastChanged = text,
                child: const Text('Hello'),
              ),
            ),
          );

          // Select some text to populate _cachedSelectables.
          await tester.press(0, 0);
          await tester.sendMouseEvent(const MouseEvent(
            button: MouseButton.left,
            x: 3,
            y: 0,
            pressed: true,
            isMotion: true,
          ));
          await tester.release(3, 0);

          expect(lastChanged, isNotBlank);

          // Re-pump with same color (early return at line 117).
          await tester.pumpComponent(
            Container(
              width: 20,
              height: 4,
              child: SelectionArea(
                selectionColor: Colors.red,
                onSelectionChanged: (text) => lastChanged = text,
                child: const Text('Hello'),
              ),
            ),
          );

          // Re-pump with different color (lines 120-123).
          await tester.pumpComponent(
            Container(
              width: 20,
              height: 4,
              child: SelectionArea(
                selectionColor: Colors.green,
                onSelectionChanged: (text) => lastChanged = text,
                child: const Text('Hello'),
              ),
            ),
          );

          // No crash expected.
        },
      );
    });

    test('null onSelectionCompleted callback', () async {
      await testNocterm(
        'null completed callback',
        (tester) async {
          String? lastChanged;

          await tester.pumpComponent(
            Container(
              width: 20,
              height: 4,
              child: SelectionArea(
                onSelectionChanged: (text) => lastChanged = text,
                // onSelectionCompleted intentionally omitted (null).
                child: const Text('Hello'),
              ),
            ),
          );

          // Complete a full selection drag.
          await tester.press(1, 0);
          await tester.sendMouseEvent(const MouseEvent(
            button: MouseButton.left,
            x: 4,
            y: 0,
            pressed: true,
            isMotion: true,
          ));
          await tester.release(4, 0);

          // Should not crash despite null onSelectionCompleted.
          expect(lastChanged, isNotBlank);
          expect(lastChanged, equals('ell'));
        },
      );
    });

    test('anchor below and focus above selectable', () async {
      await testNocterm(
        'anchor below focus above',
        (tester) async {
          String? completed;

          await tester.pumpComponent(
            Container(
              width: 20,
              height: 5,
              child: SelectionArea(
                onSelectionCompleted: (text) => completed = text,
                child: Column(
                  crossAxisAlignment: CrossAxisAlignment.start,
                  children: const [
                    SizedBox(height: 1), // row 0
                    Text('Middle'), // row 1
                    SizedBox(height: 1), // row 2
                  ],
                ),
              ),
            ),
          );

          // Press below text at row 2 (no selectable hit → null anchor).
          await tester.press(2, 2);

          // Drag into text row 1 → late anchor set.
          // _pressPosition.dy(2) >= bottom(2) → anchorIndex = length.
          await tester.sendMouseEvent(const MouseEvent(
            button: MouseButton.left,
            x: 2,
            y: 1,
            pressed: true,
            isMotion: true,
          ));

          // Drag above text to row 0.
          // globalPos.dy(0) < top(1) → focusIndex = 0.
          await tester.sendMouseEvent(const MouseEvent(
            button: MouseButton.left,
            x: 2,
            y: 0,
            pressed: true,
            isMotion: true,
          ));

          await tester.release(2, 0);

          // Full text should be selected (anchor=length, focus=0).
          expect(completed, isNotBlank);
          expect(completed, equals('Middle'));
        },
      );
    });

    test('anchor above and focus below selectable', () async {
      await testNocterm(
        'anchor above focus below',
        (tester) async {
          String? completed;

          await tester.pumpComponent(
            Container(
              width: 20,
              height: 5,
              child: SelectionArea(
                onSelectionCompleted: (text) => completed = text,
                child: Column(
                  crossAxisAlignment: CrossAxisAlignment.start,
                  children: const [
                    SizedBox(height: 1), // row 0
                    Text('Middle'), // row 1
                    SizedBox(height: 1), // row 2
                  ],
                ),
              ),
            ),
          );

          // Press above text at row 0 (no selectable hit → null anchor).
          await tester.press(2, 0);

          // Drag into text row 1 → late anchor set.
          // _pressPosition.dy(0) < top(1) → anchorIndex = 0.
          await tester.sendMouseEvent(const MouseEvent(
            button: MouseButton.left,
            x: 2,
            y: 1,
            pressed: true,
            isMotion: true,
          ));

          // Drag below text to row 2.
          // globalPos.dy(2) >= bottom(2) → focusIndex = length.
          await tester.sendMouseEvent(const MouseEvent(
            button: MouseButton.left,
            x: 2,
            y: 2,
            pressed: true,
            isMotion: true,
          ));

          await tester.release(2, 2);

          // Full text should be selected (anchor=0, focus=length).
          expect(completed, isNotBlank);
          expect(completed, equals('Middle'));
        },
      );
    });

    test('empty selectables during pointer move', () async {
      await testNocterm(
        'empty selectables',
        (tester) async {
          String? lastChanged;

          await tester.pumpComponent(
            Container(
              width: 20,
              height: 4,
              child: SelectionArea(
                onSelectionChanged: (text) => lastChanged = text,
                child: const SizedBox(height: 2),
              ),
            ),
          );

          // Press and drag over non-selectable content.
          await tester.press(1, 0);
          await tester.sendMouseEvent(const MouseEvent(
            button: MouseButton.left,
            x: 5,
            y: 1,
            pressed: true,
            isMotion: true,
          ));
          await tester.release(5, 1);

          // No crash; selection should be empty.
          expect(lastChanged, equals(''));
        },
      );
    });

    test('wheel scroll during active drag', () async {
      await testNocterm(
        'wheel during drag',
        (tester) async {
          String? lastChanged;

          await tester.pumpComponent(
            Container(
              width: 20,
              height: 3,
              child: SelectionArea(
                onSelectionChanged: (text) => lastChanged = text,
                child: SingleChildScrollView(
                  child: Column(
                    crossAxisAlignment: CrossAxisAlignment.start,
                    children: const [
                      Text('Line0'),
                      Text('Line1'),
                      Text('Line2'),
                      Text('Line3'),
                      Text('Line4'),
                    ],
                  ),
                ),
              ),
            ),
          );

          // Start selection on the first visible item.
          await tester.press(0, 0);
          await tester.sendMouseEvent(const MouseEvent(
            button: MouseButton.left,
            x: 3,
            y: 0,
            pressed: true,
            isMotion: true,
          ));

          // Send a wheel-down event while primary button is down.
          await tester.sendMouseEvent(const MouseEvent(
            button: MouseButton.wheelDown,
            x: 3,
            y: 0,
            pressed: false,
          ));

          // Pump to let post-frame callback run.
          await tester.pump();

          await tester.release(3, 1);

          // Should not crash and selection should have been maintained.
          expect(lastChanged, isNotBlank);
        },
      );
    });

    test('SingleChildScrollView viewport clipping', () async {
      await testNocterm(
        'viewport clipping',
        (tester) async {
          String? completed;

          await tester.pumpComponent(
            Container(
              width: 20,
              height: 2,
              child: SelectionArea(
                onSelectionCompleted: (text) => completed = text,
                child: SingleChildScrollView(
                  child: Column(
                    crossAxisAlignment: CrossAxisAlignment.start,
                    children: const [
                      Text('Visible1'),
                      Text('Visible2'),
                      Text('Hidden3'),
                    ],
                  ),
                ),
              ),
            ),
          );

          // Select across the visible viewport (height=2, so rows 0-1).
          await tester.press(0, 0);
          await tester.sendMouseEvent(const MouseEvent(
            button: MouseButton.left,
            x: 8,
            y: 1,
            pressed: true,
            isMotion: true,
          ));
          await tester.release(8, 1);

          // Hidden3 is outside the viewport, should be excluded.
          expect(completed, isNotBlank);
          expect(completed, contains('Visible1'));
          expect(completed, contains('Visible2'));
          expect(completed, isNot(contains('Hidden3')));
        },
      );
    });

    test('ListView context selection', () async {
      await testNocterm(
        'listview selection',
        (tester) async {
          String? completed;

          await tester.pumpComponent(
            Container(
              width: 20,
              height: 3,
              child: SelectionArea(
                onSelectionCompleted: (text) => completed = text,
                child: ListView.builder(
                  itemCount: 5,
                  itemBuilder: (context, index) => Text('Item$index'),
                ),
              ),
            ),
          );

          // Select across visible items in the ListView.
          await tester.press(0, 0);
          await tester.sendMouseEvent(const MouseEvent(
            button: MouseButton.left,
            x: 5,
            y: 2,
            pressed: true,
            isMotion: true,
          ));
          await tester.release(5, 2);

          expect(completed, isNotBlank);
          expect(completed, contains('Item0'));
          expect(completed, contains('Item1'));
          expect(completed, contains('Item2'));
        },
      );
    });

    test('multiline wrapped text selection', () async {
      await testNocterm(
        'multiline wrapped text',
        (tester) async {
          String? completed;

          await tester.pumpComponent(
            Container(
              width: 6,
              height: 5,
              child: SelectionArea(
                onSelectionCompleted: (text) => completed = text,
                child: Column(
                  crossAxisAlignment: CrossAxisAlignment.start,
                  children: const [
                    // "Hello " (6 chars) wraps into:
                    // Line 0: "Hello " or "Hello"
                    // Line 1: "World" (if text wraps)
                    Text('Hello World'),
                    Text('End'),
                  ],
                ),
              ),
            ),
          );

          // Select everything across the wrapped text and "End".
          await tester.press(0, 0);
          await tester.sendMouseEvent(const MouseEvent(
            button: MouseButton.left,
            x: 3,
            y: 2, // "End" is on the row after wrapped text
            pressed: true,
            isMotion: true,
          ));
          await tester.release(3, 2);

          // Should include text from wrapped lines with proper newlines.
          expect(completed, isNotBlank);
          expect(completed, contains('Hello'));
          expect(completed, contains('World'));
          expect(completed, contains('End'));
        },
      );
    });

    test('mouse exit during drag triggers selection completed', () async {
      await testNocterm(
        'mouse exit during drag',
        (tester) async {
          String? completed;

          await tester.pumpComponent(
            Container(
              width: 20,
              height: 4,
              child: SelectionArea(
                onSelectionCompleted: (text) => completed = text,
                child: const Text('Hello World'),
              ),
            ),
          );

          // Press inside.
          await tester.press(1, 0);

          // Drag within to build selection.
          await tester.sendMouseEvent(const MouseEvent(
            button: MouseButton.left,
            x: 5,
            y: 0,
            pressed: true,
            isMotion: true,
          ));

          // Mouse exits region with button released (simulates leaving the
          // window). The exit event is dispatched by the mouse tracker when
          // the cursor leaves the annotation's region.
          // We simulate by releasing at the same position — release triggers
          // onHover with leftDown=false which calls _handlePointerUp.
          await tester.release(5, 0);

          expect(completed, isNotBlank);
          expect(completed!.length, greaterThan(0));
        },
      );
    });

    test('onHover button state transitions', () async {
      await testNocterm(
        'hover button transitions',
        (tester) async {
          String? lastChanged;
          String? completed;

          await tester.pumpComponent(
            Container(
              width: 20,
              height: 4,
              child: SelectionArea(
                onSelectionChanged: (text) => lastChanged = text,
                onSelectionCompleted: (text) => completed = text,
                child: const Text('Hello World'),
              ),
            ),
          );

          // Press at (1, 0) — triggers onEnter then onHover.
          await tester.press(1, 0);

          // Motion while pressed — triggers onHover dragging branch.
          await tester.sendMouseEvent(const MouseEvent(
            button: MouseButton.left,
            x: 5,
            y: 0,
            pressed: true,
            isMotion: true,
          ));

          expect(lastChanged, isNotBlank);

          // Release at (5, 0) — triggers onHover release path.
          await tester.release(5, 0);

          expect(completed, isNotBlank);
          expect(completed, equals('ello'));
        },
      );
    });

    test('context entries become empty after anchor set', () async {
      await testNocterm(
        'empty context after anchor',
        (tester) async {
          String? lastChanged;

          // First: pump with selectable text.
          await tester.pumpComponent(
            Container(
              width: 20,
              height: 4,
              child: SelectionArea(
                onSelectionChanged: (text) => lastChanged = text,
                child: const Text('Hello'),
              ),
            ),
          );

          // Press to set anchor on the text.
          await tester.press(1, 0);

          // Now replace the child tree so there are no selectables.
          // The SelectionArea is rebuilt but the drag state (_isDragging)
          // persists in the render object.
          await tester.pumpComponent(
            Container(
              width: 20,
              height: 4,
              child: SelectionArea(
                onSelectionChanged: (text) => lastChanged = text,
                child: const SizedBox(height: 2),
              ),
            ),
          );

          // Send a motion event — _handlePointerMove runs, collectSelectables
          // returns empty, hits the isEmpty return at line 270.
          await tester.sendMouseEvent(const MouseEvent(
            button: MouseButton.left,
            x: 5,
            y: 0,
            pressed: true,
            isMotion: true,
          ));

          await tester.release(5, 0);

          // Should not crash; selection cleared.
          expect(lastChanged, equals(''));
        },
      );
    });

    // --- Edge auto-scroll during selection drag (PR #81) ---
    //
    // Container height does not constrain the scroll viewport in the test
    // harness (it fills the full 24-row terminal), so these use 40 rows of
    // content to guarantee a scrollable extent and target the real viewport
    // edge at row 23.

    test(
        'drag to the bottom edge auto-scrolls the viewport; mid-viewport '
        'motion does not', () async {
      await testNocterm(
        'edge auto-scroll bottom',
        (tester) async {
          final controller = ScrollController();

          await tester.pumpComponent(
            SelectionArea(
              child: SingleChildScrollView(
                controller: controller,
                child: Column(
                  crossAxisAlignment: CrossAxisAlignment.start,
                  children: [
                    for (int i = 0; i < 40; i++)
                      Text('Line${i.toString().padLeft(2, '0')}'),
                  ],
                ),
              ),
            ),
          );

          // 40 rows of content in the 24-row viewport leaves room to scroll.
          expect(controller.maxScrollExtent, greaterThan(0));
          expect(controller.offset, equals(0));

          // Begin a drag in the neutral middle of the viewport.
          await tester.press(0, 10);

          // Motion that stays in the neutral middle must NOT auto-scroll.
          await tester.sendMouseEvent(const MouseEvent(
            button: MouseButton.left,
            x: 4,
            y: 10,
            pressed: true,
            isMotion: true,
          ));
          expect(controller.offset, equals(0),
              reason: 'no auto-scroll while the pointer is away from an edge');

          // Motion to the bottom edge row fires edge auto-scroll. The first
          // tick runs synchronously, advancing the viewport downward.
          await tester.sendMouseEvent(const MouseEvent(
            button: MouseButton.left,
            x: 4,
            y: 23,
            pressed: true,
            isMotion: true,
          ));
          expect(controller.offset, greaterThan(0),
              reason: 'dragging to the bottom edge should auto-scroll down');

          await tester.release(4, 23);
        },
      );
    });

    test('edge auto-scroll stops once the drag is released', () async {
      await testNocterm(
        'edge auto-scroll stops on release',
        (tester) async {
          final controller = ScrollController();

          await tester.pumpComponent(
            SelectionArea(
              child: SingleChildScrollView(
                controller: controller,
                child: Column(
                  crossAxisAlignment: CrossAxisAlignment.start,
                  children: [
                    for (int i = 0; i < 40; i++)
                      Text('Row${i.toString().padLeft(2, '0')}'),
                  ],
                ),
              ),
            ),
          );

          await tester.press(0, 10);
          await tester.sendMouseEvent(const MouseEvent(
            button: MouseButton.left,
            x: 4,
            y: 23,
            pressed: true,
            isMotion: true,
          ));
          expect(controller.offset, greaterThan(0));

          // Releasing ends the drag and must cancel the auto-scroll loop.
          await tester.release(4, 23);
          final offsetAtRelease = controller.offset;

          // Further frames must not keep scrolling now that the drag is over.
          await tester.pump();
          await tester.pump();
          await tester.pump();

          expect(controller.offset, equals(offsetAtRelease),
              reason: 'auto-scroll must not run after the drag ends');
        },
      );
    });
  });
}

class _SelectionRebuildHarness extends StatefulComponent {
  const _SelectionRebuildHarness({required this.onSelectionChanged});

  final ValueChanged<String> onSelectionChanged;

  static _SelectionRebuildHarnessState? lastState;

  @override
  State<_SelectionRebuildHarness> createState() =>
      _SelectionRebuildHarnessState();
}

class _SelectionRebuildHarnessState extends State<_SelectionRebuildHarness> {
  bool _useAltKey = false;

  @override
  void initState() {
    super.initState();
    _SelectionRebuildHarness.lastState = this;
  }

  void swapFirstKey() {
    setState(() {
      _useAltKey = true;
    });
  }

  @override
  Component build(BuildContext context) {
    return SelectionArea(
      key: const ValueKey('selection-area'),
      onSelectionChanged: component.onSelectionChanged,
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          Text('Alpha', key: ValueKey(_useAltKey ? 'a2' : 'a')),
          const Text('Bravo', key: ValueKey('b')),
        ],
      ),
    );
  }
}

class _SelectionAreaSwapHarness extends StatefulComponent {
  const _SelectionAreaSwapHarness({required this.onSelectionChanged});

  final ValueChanged<String> onSelectionChanged;

  static _SelectionAreaSwapHarnessState? lastState;

  @override
  State<_SelectionAreaSwapHarness> createState() =>
      _SelectionAreaSwapHarnessState();
}

class _SelectionAreaSwapHarnessState extends State<_SelectionAreaSwapHarness> {
  bool _useListView = true;

  @override
  void initState() {
    super.initState();
    _SelectionAreaSwapHarness.lastState = this;
  }

  void swapToColumn() {
    setState(() {
      _useListView = false;
    });
  }

  @override
  Component build(BuildContext context) {
    return SelectionArea(
      onSelectionChanged: component.onSelectionChanged,
      child: _useListView
          ? ListView.builder(
              itemCount: 3,
              itemBuilder: (context, index) => Text('Item$index'),
            )
          : Column(
              crossAxisAlignment: CrossAxisAlignment.start,
              children: const [
                Text('New1'),
                Text('New2'),
              ],
            ),
    );
  }
}

class _SelectionAreaUnmountHarness extends StatefulComponent {
  const _SelectionAreaUnmountHarness();

  static _SelectionAreaUnmountHarnessState? lastState;

  @override
  State<_SelectionAreaUnmountHarness> createState() =>
      _SelectionAreaUnmountHarnessState();
}

class _SelectionAreaUnmountHarnessState
    extends State<_SelectionAreaUnmountHarness> {
  bool _mountedSelectionArea = true;

  @override
  void initState() {
    super.initState();
    _SelectionAreaUnmountHarness.lastState = this;
  }

  void unmountSelectionArea() {
    setState(() {
      _mountedSelectionArea = false;
    });
  }

  @override
  Component build(BuildContext context) {
    if (!_mountedSelectionArea) {
      return const SizedBox(height: 2);
    }

    return const SelectionArea(
      child: Text('Hello'),
    );
  }
}

class _RenderSelectionAreaHarness extends SingleChildRenderObjectComponent {
  const _RenderSelectionAreaHarness({
    required super.child,
    this.onCreated,
    this.onSelectionChanged,
    this.onSelectionCompleted,
    this.selectionColor,
  });

  final void Function(RenderSelectionArea)? onCreated;
  final ValueChanged<String>? onSelectionChanged;
  final ValueChanged<String>? onSelectionCompleted;
  final Color? selectionColor;

  @override
  RenderObject createRenderObject(BuildContext context) {
    final renderObject = RenderSelectionArea(
      selectionColor: selectionColor ?? Colors.blue,
      onSelectionChanged: onSelectionChanged,
      onSelectionCompleted: onSelectionCompleted,
    );
    onCreated?.call(renderObject);
    return renderObject;
  }

  @override
  void updateRenderObject(
      BuildContext context, covariant RenderSelectionArea renderObject) {
    if (selectionColor != null) {
      renderObject.selectionColor = selectionColor!;
    }
    renderObject
      ..onSelectionChanged = onSelectionChanged
      ..onSelectionCompleted = onSelectionCompleted;
  }
}
