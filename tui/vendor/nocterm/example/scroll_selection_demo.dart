import 'package:nocterm/nocterm.dart';

void main() async {
  await runApp(const Navigator(home: ScrollSelectionDemo()));
}

class ScrollSelectionDemo extends StatefulComponent {
  const ScrollSelectionDemo({super.key});

  @override
  State<ScrollSelectionDemo> createState() => _ScrollSelectionDemoState();
}

class _ScrollSelectionDemoState extends State<ScrollSelectionDemo> {
  final ScrollController _controller = ScrollController();

  @override
  void dispose() {
    _controller.dispose();
    super.dispose();
  }

  @override
  Component build(BuildContext context) {
    return SelectionArea(
      onSelectionCompleted: ClipboardManager.copy,
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          Container(
            padding: EdgeInsets.symmetric(horizontal: 1, vertical: 1),
            decoration: BoxDecoration(
              border: BoxBorder.all(color: Colors.cyan),
            ),
            child: Column(
              crossAxisAlignment: CrossAxisAlignment.start,
              children: [
                const Text(
                  'SelectionArea + SingleChildScrollView',
                  style: TextStyle(fontWeight: FontWeight.bold),
                ),
                const Text('Click-drag to select. Use mouse wheel to scroll.'),
                const Text('Selection is copied to clipboard on mouse up.'),
              ],
            ),
          ),
          Expanded(
            child: Container(
              decoration: BoxDecoration(
                border: BoxBorder.all(color: Colors.green),
              ),
              child: SingleChildScrollView(
                controller: _controller,
                padding: EdgeInsets.all(1),
                child: Column(
                  crossAxisAlignment: CrossAxisAlignment.start,
                  children: [
                    const Text('Scrollable content:'),
                    const Text(''),
                    for (int i = 0; i < 60; i++)
                      Text(
                          'Line $i: The quick brown fox jumps over the lazy dog.'),
                    const Text(''),
                    const Text(
                      'End of content.',
                      style: TextStyle(color: Colors.magenta),
                    ),
                  ],
                ),
              ),
            ),
          ),
        ],
      ),
    );
  }
}
