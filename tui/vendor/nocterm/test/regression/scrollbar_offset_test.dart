import 'package:test/test.dart';
import 'package:nocterm/nocterm.dart';

void main() {
  test('scrollbar placement uses local offset', () async {
    await testNocterm(
      'scrollbar offset placement',
      (tester) async {
        final controller = ScrollController();

        const leftWidth = 20;
        const dividerWidth = 1;
        const rightWidth = 20;
        const height = 8;

        await tester.pumpComponent(
          Row(
            children: [
              Container(
                width: leftWidth.toDouble(),
                height: height.toDouble(),
                child: const Text('Left'),
              ),
              Container(
                width: dividerWidth.toDouble(),
                height: height.toDouble(),
                child: const Text('|'),
              ),
              Container(
                width: rightWidth.toDouble(),
                height: height.toDouble(),
                child: Scrollbar(
                  controller: controller,
                  thumbVisibility: true,
                  thickness: 1,
                  child: SingleChildScrollView(
                    controller: controller,
                    child: Column(
                      children: List.generate(
                        30,
                        (index) => Text('Item $index'),
                      ),
                    ),
                  ),
                ),
              ),
            ],
          ),
        );

        expect(controller.maxScrollExtent, greaterThan(0));

        final expectedX = leftWidth + dividerWidth + rightWidth - 1;
        bool foundScrollbar = false;
        final columnChars = StringBuffer();
        for (int y = 0; y < height; y++) {
          final cell = tester.terminalState.getCellAt(expectedX, y);
          final char = cell?.char ?? ' ';
          columnChars.write(char);
          if (char == '│' || char == '█' || char == '▲' || char == '▼') {
            foundScrollbar = true;
          }
        }

        expect(
          foundScrollbar,
          isTrue,
          reason: 'No scrollbar glyphs at x=$expectedX. Column="$columnChars".',
        );
      },
      size: const Size(80, 16),
    );
  });
}
