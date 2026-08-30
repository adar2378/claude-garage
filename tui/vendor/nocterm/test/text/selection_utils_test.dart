import 'package:nocterm/nocterm.dart';
import 'package:nocterm/src/text/selection_utils.dart' as selection_utils;
import 'package:test/test.dart';

void main() {
  group('selection_utils', () {
    test('lineStartOffsets accounts for consecutive newlines', () {
      const text = 'a\n\nb';
      const lines = ['a', '', 'b'];

      final offsets = selection_utils.lineStartOffsets(text, lines);

      expect(offsets, equals([0, 2, 3]));
      expect(Selectable.lineStartOffsets(text, lines), equals(offsets));
    });

    test('getCharacterIndexAtLocalPosition snaps by half-grapheme', () {
      const text = 'abc';
      const lines = ['abc'];

      expect(
        selection_utils.getCharacterIndexAtLocalPosition(
          localPos: const Offset(0.0, 0.0),
          text: text,
          lines: lines,
        ),
        equals(0),
      );

      expect(
        selection_utils.getCharacterIndexAtLocalPosition(
          localPos: const Offset(0.4, 0.0),
          text: text,
          lines: lines,
        ),
        equals(0),
      );

      expect(
        selection_utils.getCharacterIndexAtLocalPosition(
          localPos: const Offset(0.6, 0.0),
          text: text,
          lines: lines,
        ),
        equals(1),
      );

      expect(
        selection_utils.getCharacterIndexAtLocalPosition(
          localPos: const Offset(1.6, 0.0),
          text: text,
          lines: lines,
        ),
        equals(2),
      );
    });

    test('getCharacterIndexAtLocalPosition maps empty lines', () {
      const text = 'a\n\nb';
      const lines = ['a', '', 'b'];

      final index = selection_utils.getCharacterIndexAtLocalPosition(
        localPos: const Offset(0.0, 1.0),
        text: text,
        lines: lines,
      );

      expect(index, equals(2)); // Start of the empty line.
    });
  });
}
