import 'package:nocterm/nocterm.dart';
import 'package:test/test.dart';

/// Regression test for ParentDataElement's copy-in-place path skipping
/// markNeedsLayout.
///
/// When a Positioned child lives inside an Overlay/Theater, the child's
/// render object carries TheaterParentData (a StackParentData subtype).
/// Applying new Positioned values goes through
/// _copyStackParentDataIfApplicable, which mutates the existing parent
/// data in place and returns early. That early return used to skip the
/// `renderObject.parent?.markNeedsLayout()` call made by the replacement
/// path, so under the value-equality layout skip the theater never re-ran
/// performLayout and the child stayed painted at its old position.
void main() {
  group('ParentDataElement copy-path relayout', () {
    test('moving a Positioned inside an Overlay repositions it', () async {
      await testNocterm('overlay positioned move', (tester) async {
        await tester.pumpComponent(const _OverlayMover());

        final state = tester.findState<_OverlayMoverState>();

        var matches = tester.terminalState.findText('XX');
        expect(matches, hasLength(1));
        expect(matches.single.x, 2,
            reason: 'initial frame must paint XX at left: 2');

        state.moveTo(20);
        await tester.pump();

        matches = tester.terminalState.findText('XX');
        expect(matches, hasLength(1));
        expect(
          matches.single.x,
          20,
          reason: 'after setState the Positioned child must repaint at '
              'left: 20 — the copy-in-place parent-data path must mark '
              'the theater for relayout',
        );
      });
    });
  });
}

class _OverlayMover extends StatefulComponent {
  const _OverlayMover();

  @override
  State<_OverlayMover> createState() => _OverlayMoverState();
}

class _OverlayMoverState extends State<_OverlayMover> {
  int _left = 2;
  late final OverlayEntry _entry = OverlayEntry(
    builder: (context) => Positioned(
      left: _left.toDouble(),
      top: 0,
      child: const Text('XX'),
    ),
  );

  void moveTo(int left) {
    setState(() => _left = left);
    _entry.markNeedsBuild();
  }

  @override
  Component build(BuildContext context) {
    return Overlay(initialEntries: [_entry]);
  }
}
