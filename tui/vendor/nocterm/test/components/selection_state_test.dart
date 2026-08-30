import 'package:nocterm/src/components/selection_state.dart';
import 'package:test/test.dart';

void main() {
  group('SelectionDragState', () {
    test('rangeFor returns the last updated range', () {
      final context = Object();

      SelectionDragState.begin();
      SelectionDragState.updateRange(context, 2, 5);

      final range = SelectionDragState.rangeFor(context);

      expect(range, isNotNull);
      expect(range!.minIndex, equals(2));
      expect(range.maxIndex, equals(5));

      SelectionDragState.end();
    });
  });
}
