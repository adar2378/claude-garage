import 'package:nocterm/nocterm.dart';
import 'package:test/test.dart';

/// Regression tests for render objects whose layout-relevant properties are
/// updated via updateRenderObject.
///
/// Under the value-equality layout skip (RenderObject.layout returns early
/// when `!_needsLayout && constraints == _constraints`), every layout-relevant
/// property mutation must call markNeedsLayout. RenderPadding and
/// RenderPositionedBox used bare public fields, so changing Padding.padding
/// or Align.alignment between frames silently kept the old layout.
void main() {
  group('render object property changes mark layout dirty', () {
    test('Padding change re-layouts the child', () async {
      await testNocterm('padding relayout', (tester) async {
        // The Divider fills whatever width it is given, so the number of
        // painted line characters reveals the child's laid-out width.
        // RenderPadding.paint re-derives the child *offset* from the live
        // padding field, which masks the bug for position - the child's
        // stale constraints/size are the observable failure.
        await tester.pumpComponent(const _PaddedDivider(padding: 1));

        expect(_dividerWidth(tester), 18,
            reason: 'padding 1 in a 20-wide box leaves 18 columns');

        final state = tester.findState<_PaddedDividerState>();
        state.setPadding(5);
        await tester.pump();

        expect(
          _dividerWidth(tester),
          10,
          reason: 'after padding 1 -> 5 the child must re-layout to '
              '10 columns; a stale layout keeps 18',
        );
      }, size: const Size(20, 7));
    });

    test('Align change repositions the child', () async {
      await testNocterm('align relayout', (tester) async {
        await tester.pumpComponent(const _AlignedLabel(toBottomRight: false));

        var match = tester.terminalState.findText('AB').single;
        expect((match.x, match.y), (0, 0));

        final state = tester.findState<_AlignedLabelState>();
        state.toggle();
        await tester.pump();

        match = tester.terminalState.findText('AB').single;
        expect(
          (match.x, match.y),
          (18, 9),
          reason: 'after alignment topLeft -> bottomRight the child must '
              'move to the bottom-right corner; the offset is computed in '
              'performLayout, so a skipped layout freezes it at (0, 0)',
        );
      }, size: const Size(20, 10));
    });
  });
}

int _dividerWidth(NoctermTester tester) {
  final text = tester.terminalState.getText();
  return RegExp(r'─+')
      .allMatches(text)
      .fold(0, (max, m) => m.end - m.start > max ? m.end - m.start : max);
}

class _PaddedDivider extends StatefulComponent {
  const _PaddedDivider({required this.padding});

  final double padding;

  @override
  State<_PaddedDivider> createState() => _PaddedDividerState();
}

class _PaddedDividerState extends State<_PaddedDivider> {
  late double _padding = component.padding;

  void setPadding(double value) {
    setState(() => _padding = value);
  }

  @override
  Component build(BuildContext context) {
    return Padding(
      padding: EdgeInsets.all(_padding),
      child: const Divider(),
    );
  }
}

class _AlignedLabel extends StatefulComponent {
  const _AlignedLabel({required this.toBottomRight});

  final bool toBottomRight;

  @override
  State<_AlignedLabel> createState() => _AlignedLabelState();
}

class _AlignedLabelState extends State<_AlignedLabel> {
  late bool _bottomRight = component.toBottomRight;

  void toggle() {
    setState(() => _bottomRight = !_bottomRight);
  }

  @override
  Component build(BuildContext context) {
    return Align(
      alignment: _bottomRight ? Alignment.bottomRight : Alignment.topLeft,
      child: const Text('AB'),
    );
  }
}
