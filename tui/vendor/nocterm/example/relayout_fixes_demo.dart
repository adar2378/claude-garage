import 'package:nocterm/nocterm.dart';

/// Interactive demo for the relayout fixes (post-0.6.0 layout-skip contract).
///
/// Run with the fixes in the working tree:
///   dart run example/relayout_fixes_demo.dart
///
/// To see the broken behavior, stash the fixes and run again:
///   git stash && dart run example/relayout_fixes_demo.dart && git stash pop
///
/// Each panel exercises one fixed bug. With the fixes, every keypress
/// visibly updates its panel. Without them, the panels freeze after the
/// first frame settles (press the key at least twice - transient dirtiness
/// from the very first layout can mask the bug for one frame).
///
///   [o] Overlay:  moves a Positioned box inside an Overlay
///                 (proxy_element copy-path markNeedsLayout)
///   [p] Padding:  cycles EdgeInsets.all(0/2/4) around a Divider
///                 (RenderPadding compare-and-mark setter)
///   [a] Align:    cycles topLeft / center / bottomRight
///                 (RenderPositionedBox compare-and-mark setters)
///   [s] Swap:     reorders two const keyed Row children
///                 (ContainerRenderObjectMixin.move marks layout)
///   [h] Hoisted:  bumps a counter read by a hoisted (identical across
///                 rebuilds) ListView itemBuilder and LayoutBuilder builder
///                 (unconditional markNeedsLayout in update())
void main() {
  runApp(const RelayoutFixesDemo());
}

class RelayoutFixesDemo extends StatefulComponent {
  const RelayoutFixesDemo({super.key});

  @override
  State<RelayoutFixesDemo> createState() => _RelayoutFixesDemoState();
}

class _RelayoutFixesDemoState extends State<RelayoutFixesDemo> {
  // [o] Overlay scenario
  int _overlayLeft = 1;
  late final OverlayEntry _entry = OverlayEntry(
    builder: (context) => Positioned(
      left: _overlayLeft.toDouble(),
      top: 1,
      child: const Text('[BOX]', style: TextStyle(color: Color(0xFF00FFAA))),
    ),
  );

  // [p] Padding scenario
  static const _paddings = [0.0, 2.0, 4.0];
  int _paddingStep = 0;

  // [a] Align scenario
  static const _alignments = [
    Alignment.topLeft,
    Alignment.center,
    Alignment.bottomRight,
  ];
  static const _alignmentNames = ['topLeft', 'center', 'bottomRight'];
  int _alignStep = 0;

  // [s] Swap scenario
  static const _childA = Text('AAA',
      key: ValueKey('a'), style: TextStyle(color: Color(0xFFFF6666)));
  static const _childB = Text('BB',
      key: ValueKey('b'), style: TextStyle(color: Color(0xFF66AAFF)));
  bool _aFirst = true;

  // [h] Hoisted-builder scenario: the SAME closure instances across
  // rebuilds - only the state they read changes.
  int _counter = 0;
  // ignore: prefer_function_declarations_over_variables
  late final Component Function(BuildContext, int) _itemBuilder =
      (context, index) => Text('item $index -> counter=$_counter');
  // ignore: prefer_function_declarations_over_variables
  late final LayoutBuilderCallback _layoutBuilder =
      (context, constraints) => Text('LayoutBuilder -> counter=$_counter');

  bool _handleKeyEvent(KeyboardEvent event) {
    switch (event.logicalKey) {
      case LogicalKey.keyO:
        setState(() => _overlayLeft = (_overlayLeft + 4) % 24);
        _entry.markNeedsBuild();
        return true;
      case LogicalKey.keyP:
        setState(() => _paddingStep = (_paddingStep + 1) % _paddings.length);
        return true;
      case LogicalKey.keyA:
        setState(() => _alignStep = (_alignStep + 1) % _alignments.length);
        return true;
      case LogicalKey.keyS:
        setState(() => _aFirst = !_aFirst);
        return true;
      case LogicalKey.keyH:
        setState(() => _counter++);
        return true;
      case LogicalKey.keyQ:
        shutdownApp();
        return true;
      default:
        return false;
    }
  }

  Component _panel(String title, Component child, {double height = 7}) {
    return SizedBox(
      width: 38,
      height: height,
      child: Container(
        decoration: BoxDecoration(border: BoxBorder.all()),
        child: Column(
          crossAxisAlignment: CrossAxisAlignment.start,
          children: [
            Text(title,
                style: const TextStyle(
                    fontWeight: FontWeight.bold, color: Color(0xFFFFAA00))),
            Expanded(child: child),
          ],
        ),
      ),
    );
  }

  @override
  Component build(BuildContext context) {
    return Focusable(
      focused: true,
      onKeyEvent: _handleKeyEvent,
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          const Text(
            ' relayout fixes demo - press each key repeatedly; q quits ',
            style: TextStyle(fontWeight: FontWeight.bold),
          ),
          Row(children: [
            _panel(
              '[o] Overlay left=$_overlayLeft',
              SizedBox(
                width: 34,
                height: 4,
                child: Overlay(initialEntries: [_entry]),
              ),
            ),
            _panel(
              '[p] Padding all(${_paddings[_paddingStep].toInt()})',
              Padding(
                padding: EdgeInsets.all(_paddings[_paddingStep]),
                // double style (═) so the divider is distinguishable from
                // the single-line (─) panel borders.
                child: const Divider(style: DividerStyle.double),
              ),
            ),
          ]),
          Row(children: [
            _panel(
              '[a] Align ${_alignmentNames[_alignStep]}',
              Align(
                alignment: _alignments[_alignStep],
                child: const Text('@@',
                    style: TextStyle(color: Color(0xFF00FFAA))),
              ),
            ),
            _panel(
              '[s] Swap order=${_aFirst ? "A,B" : "B,A"}',
              Row(
                  children: _aFirst
                      ? const [_childA, _childB]
                      : const [_childB, _childA]),
              height: 7,
            ),
          ]),
          _panel(
            '[h] Hoisted builders counter=$_counter',
            Column(
              crossAxisAlignment: CrossAxisAlignment.start,
              children: [
                SizedBox(
                  height: 3,
                  child: ListView.builder(
                    itemCount: 3,
                    itemExtent: 1,
                    itemBuilder: _itemBuilder,
                  ),
                ),
                LayoutBuilder(builder: _layoutBuilder),
              ],
            ),
            height: 8,
          ),
        ],
      ),
    );
  }
}
