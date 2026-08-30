import 'package:nocterm/nocterm.dart';
import 'package:test/test.dart';

/// Regression test for updateChild slot propagation on identity-equal
/// components.
///
/// When updateChild received a component that was identical (==) to the
/// existing child's component (e.g. const widgets reused across rebuilds), it
/// returned early without checking whether the slot had changed. This meant
/// reordering const children inside a Stack had no effect on render-tree paint
/// order — the last-painted (visually "on top") child was frozen.
void main() {
  group('updateChild slot propagation for identity-equal components', () {
    test('reordering const children in Stack changes paint order', () async {
      await testNocterm('const child reorder', (tester) async {
        // Pump with order A (bottom) → B (top).
        // B paints last so its text wins.
        await tester.pumpComponent(const _ReorderableStack(topIsB: true));

        // B is on top — only 'BBB' visible at the overlap position.
        expect(tester.terminalState, containsText('BBB'));

        // Reorder: B (bottom) → A (top).
        // A should now paint last.
        await tester.pumpComponent(const _ReorderableStack(topIsB: false));

        // A is on top — 'AAA' must be visible at the overlap position.
        expect(
          tester.terminalState,
          containsText('AAA'),
          reason: 'After reordering const children, the new last child '
              'should paint on top',
        );
      });
    });

    test('cycling const children through multiple reorders', () async {
      await testNocterm('const child cycling', (tester) async {
        await tester.pumpComponent(const _CyclingStack(activeIndex: 0));
        expect(tester.terminalState, containsText('VIEW_0'));

        await tester.pumpComponent(const _CyclingStack(activeIndex: 1));
        expect(tester.terminalState, containsText('VIEW_1'));

        await tester.pumpComponent(const _CyclingStack(activeIndex: 2));
        expect(tester.terminalState, containsText('VIEW_2'));

        // Cycle back to 0.
        await tester.pumpComponent(const _CyclingStack(activeIndex: 0));
        expect(
          tester.terminalState,
          containsText('VIEW_0'),
          reason: 'Cycling back to original order must work',
        );
      });
    });

    test('stateful reorder preserves child state', () async {
      await testNocterm('stateful reorder', (tester) async {
        await tester.pumpComponent(const _StatefulReorder());

        final state = tester.findState<_StatefulReorderState>();

        // Initial: B on top.
        expect(tester.terminalState, containsText('BBB'));

        // Flip: A on top.
        state.toggle();
        await tester.pump();
        expect(
          tester.terminalState,
          containsText('AAA'),
          reason: 'A should be on top after toggle',
        );

        // Flip back: B on top again.
        state.toggle();
        await tester.pump();
        expect(
          tester.terminalState,
          containsText('BBB'),
          reason: 'B should be on top after toggling back',
        );
      });
    });
  });
}

// ---------------------------------------------------------------------------
// Test helpers
// ---------------------------------------------------------------------------

/// Stack with two const children whose order is controlled by [topIsB].
/// Both children fill the entire area so the top one fully overlaps.
class _ReorderableStack extends StatelessComponent {
  const _ReorderableStack({required this.topIsB});

  final bool topIsB;

  static const _a = _ConstView(key: ValueKey('a'), label: 'AAA');
  static const _b = _ConstView(key: ValueKey('b'), label: 'BBB');

  @override
  Component build(BuildContext context) {
    return Stack(children: topIsB ? const [_a, _b] : const [_b, _a]);
  }
}

/// Stack that cycles three const keyed views, placing the active one last.
class _CyclingStack extends StatelessComponent {
  const _CyclingStack({required this.activeIndex});

  final int activeIndex;

  static const _views = [
    _ConstView(key: ValueKey(0), label: 'VIEW_0'),
    _ConstView(key: ValueKey(1), label: 'VIEW_1'),
    _ConstView(key: ValueKey(2), label: 'VIEW_2'),
  ];

  @override
  Component build(BuildContext context) {
    return Stack(
      children: [
        for (var i = 0; i < _views.length; i++)
          if (i != activeIndex) _views[i],
        _views[activeIndex],
      ],
    );
  }
}

/// Stateful wrapper to toggle child order via setState.
class _StatefulReorder extends StatefulComponent {
  const _StatefulReorder();

  @override
  State<_StatefulReorder> createState() => _StatefulReorderState();
}

class _StatefulReorderState extends State<_StatefulReorder> {
  bool _topIsB = true;

  void toggle() => setState(() => _topIsB = !_topIsB);

  @override
  Component build(BuildContext context) {
    return _ReorderableStack(topIsB: _topIsB);
  }
}

/// A const-constructible full-size view with a centered label.
class _ConstView extends StatelessComponent {
  const _ConstView({required this.label, super.key});

  final String label;

  @override
  Component build(BuildContext context) {
    return Positioned.fill(child: Center(child: Text(label)));
  }
}
