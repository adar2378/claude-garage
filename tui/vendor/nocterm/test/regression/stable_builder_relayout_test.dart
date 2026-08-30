import 'package:nocterm/nocterm.dart';
import 'package:test/test.dart';

/// Regression tests for stale content with stable (hoisted) builders.
///
/// ListView and LayoutBuilder build their children during layout. Their
/// elements used to gate markNeedsLayout on builder-closure identity, so a
/// hoisted builder reading mutated external state never triggered a layout
/// pass: update() ran, the dirty flags were set, but with value-equal
/// constraints the layout skip kept performLayout from running and the
/// stale child content stayed on screen.
void main() {
  group('stable builders must not pin stale content', () {
    test('ListView with a hoisted itemBuilder re-renders mutated state',
        () async {
      await testNocterm('hoisted itemBuilder', (tester) async {
        await tester.pumpComponent(_HoistedList());
        // Settle the transient dirtiness left by the first layout pass
        // (adoptChild re-marks the viewport while building children), so
        // the next frame genuinely starts from a clean render object.
        await tester.pump();

        expect(tester.terminalState.containsText('OLD-0'), isTrue);

        tester.findState<_HoistedListState>().rename('NEW');
        await tester.pump();

        expect(
          tester.terminalState.containsText('NEW-0'),
          isTrue,
          reason: 'the itemBuilder closure is identical across rebuilds, '
              'but the state it reads changed - the list must re-layout '
              'and rebuild its children',
        );
        expect(tester.terminalState.containsText('OLD-0'), isFalse);
      }, size: const Size(20, 6));
    });

    test('LayoutBuilder with a hoisted builder re-renders mutated state',
        () async {
      await testNocterm('hoisted LayoutBuilder builder', (tester) async {
        await tester.pumpComponent(_HoistedLayoutBuilder());
        // Settle the transient dirtiness left by the first layout pass
        // (adoptChild re-marks the render object while inserting the built
        // child), so the next frame genuinely starts from a clean state.
        await tester.pump();

        expect(tester.terminalState.containsText('value=0'), isTrue);

        tester.findState<_HoistedLayoutBuilderState>().increment();
        await tester.pump();

        expect(
          tester.terminalState.containsText('value=1'),
          isTrue,
          reason: 'the builder closure is identical across rebuilds, but '
              'the state it reads changed - the LayoutBuilder must re-run '
              'its builder at the next layout',
        );
      }, size: const Size(20, 6));
    });
  });
}

class _HoistedList extends StatefulComponent {
  @override
  State<_HoistedList> createState() => _HoistedListState();
}

class _HoistedListState extends State<_HoistedList> {
  String _label = 'OLD';

  // Deliberately hoisted: the SAME closure instance is passed to every
  // ListView the build method creates. A function declaration would not
  // guarantee a stable identity, which is what this test depends on.
  // ignore: prefer_function_declarations_over_variables
  late final Component Function(BuildContext, int) _builder =
      (context, index) => Text('$_label-$index');

  void rename(String label) {
    setState(() => _label = label);
  }

  @override
  Component build(BuildContext context) {
    return ListView.builder(
      itemCount: 2,
      itemExtent: 1,
      itemBuilder: _builder,
    );
  }
}

class _HoistedLayoutBuilder extends StatefulComponent {
  @override
  State<_HoistedLayoutBuilder> createState() => _HoistedLayoutBuilderState();
}

class _HoistedLayoutBuilderState extends State<_HoistedLayoutBuilder> {
  int _value = 0;

  // Deliberately hoisted: identical builder instance across rebuilds.
  // ignore: prefer_function_declarations_over_variables
  late final LayoutBuilderCallback _builder =
      (context, constraints) => Text('value=$_value');

  void increment() {
    setState(() => _value++);
  }

  @override
  Component build(BuildContext context) {
    return LayoutBuilder(builder: _builder);
  }
}
