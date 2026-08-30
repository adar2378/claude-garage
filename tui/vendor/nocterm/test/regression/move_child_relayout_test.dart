import 'package:nocterm/nocterm.dart';
import 'package:test/test.dart';

/// Regression test for ContainerRenderObjectMixin.move() not marking layout.
///
/// Reordering identity-equal (const, keyed) children routes through
/// updateChild's slot propagation into moveRenderObjectChild -> move(),
/// which reorders the render child list. Child order determines Flex
/// offsets, so without a markNeedsLayout the parent keeps the offsets
/// computed for the old order (the value-equality layout skip prevents
/// the relayout that used to happen incidentally).
void main() {
  test('reordering keyed const children in a Row swaps their offsets',
      () async {
    await testNocterm('row const reorder', (tester) async {
      await tester.pumpComponent(const _SwappableRow(aFirst: true));

      expect(tester.terminalState.findText('AAA').single.x, 0);
      expect(tester.terminalState.findText('BB').single.x, 3);

      final state = tester.findState<_SwappableRowState>();
      state.swap();
      await tester.pump();

      expect(
        tester.terminalState.findText('BB').single.x,
        0,
        reason: 'after the swap BB is the first Row child and must be '
            'laid out at x=0',
      );
      expect(
        tester.terminalState.findText('AAA').single.x,
        2,
        reason: 'after the swap AAA follows BB and must be laid out at '
            'x=2; stale offsets keep it at x=0',
      );
    }, size: const Size(20, 3));
  });
}

class _SwappableRow extends StatefulComponent {
  const _SwappableRow({required this.aFirst});

  final bool aFirst;

  @override
  State<_SwappableRow> createState() => _SwappableRowState();
}

class _SwappableRowState extends State<_SwappableRow> {
  static const _a = Text('AAA', key: ValueKey('a'));
  static const _b = Text('BB', key: ValueKey('b'));

  late bool _aFirst = component.aFirst;

  void swap() {
    setState(() => _aFirst = !_aFirst);
  }

  @override
  Component build(BuildContext context) {
    return Row(children: _aFirst ? const [_a, _b] : const [_b, _a]);
  }
}
