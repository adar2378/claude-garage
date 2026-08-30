// Grid math (tui/lib/ui/grid_layout.dart): ceil(sqrt(n)) columns, row-major
// slots (spec tui-wall: "Grid layout with a six-tile cap"). The cap and LRU
// swap-in are state logic, tested in wall_store_test.dart.
import 'package:garage_tui/ui/grid_layout.dart';
import 'package:test/test.dart';

void main() {
  group('gridColumns', () {
    test('ceil(sqrt(n)) for every wall size', () {
      expect(gridColumns(0), 0);
      expect(gridColumns(1), 1);
      expect(gridColumns(2), 2);
      expect(gridColumns(3), 2);
      expect(gridColumns(4), 2);
      expect(gridColumns(5), 3);
      expect(gridColumns(6), 3);
    });
  });

  group('gridRowCount', () {
    test('rows cover all tiles at the column count', () {
      expect(gridRowCount(0), 0);
      expect(gridRowCount(1), 1);
      expect(gridRowCount(2), 1); // 2 cols × 1 row
      expect(gridRowCount(3), 2); // 2 cols → 2+1
      expect(gridRowCount(4), 2); // 2×2
      expect(gridRowCount(5), 2); // 3 cols → 3+2
      expect(gridRowCount(6), 2); // 3×2
    });
  });

  group('gridSlot', () {
    test('row-major placement', () {
      // 5 tiles → 3 columns: [0 1 2] / [3 4]
      expect(gridSlot(0, 5), (row: 0, col: 0));
      expect(gridSlot(2, 5), (row: 0, col: 2));
      expect(gridSlot(3, 5), (row: 1, col: 0));
      expect(gridSlot(4, 5), (row: 1, col: 1));
      // 4 tiles → 2×2
      expect(gridSlot(2, 4), (row: 1, col: 0));
      expect(gridSlot(3, 4), (row: 1, col: 1));
    });
  });

  group('gridRows', () {
    test('walks every slot exactly once, row-major', () {
      expect(gridRows(0), isEmpty);
      expect(gridRows(1), [
        [0],
      ]);
      expect(gridRows(3), [
        [0, 1],
        [2],
      ]);
      expect(gridRows(6), [
        [0, 1, 2],
        [3, 4, 5],
      ]);
    });

    test('cell rects tile the area exactly — no gaps, no overlap', () {
      const width = 173, height = 51; // deliberately not divisible
      for (var n = 1; n <= 6; n++) {
        var area = 0;
        for (var i = 0; i < n; i++) {
          final r = gridCellRect(i, n, width, height);
          expect(r.width, greaterThan(0));
          expect(r.height, greaterThan(0));
          expect(r.x + r.width, lessThanOrEqualTo(width));
          expect(r.y + r.height, lessThanOrEqualTo(height));
          area += r.width * r.height;
        }
        // Full rows tile the width; a short last row leaves blank cells,
        // so total area is slots/gridcells * cell area.
        final cols = gridColumns(n);
        final rows = gridRowCount(n);
        expect(area, lessThanOrEqualTo(width * height));
        if (n == cols * rows) {
          expect(area, width * height,
              reason: 'a full $cols x $rows grid must cover everything');
        }
      }
      // Adjacent cells share edges exactly (n=6: 3x2).
      final a = gridCellRect(0, 6, width, height);
      final b = gridCellRect(1, 6, width, height);
      final d = gridCellRect(3, 6, width, height);
      expect(b.x, a.x + a.width);
      expect(d.y, a.y + a.height);
    });

    test('rows agree with gridSlot for all n up to the cap', () {
      for (var n = 1; n <= 6; n++) {
        final rows = gridRows(n);
        expect(rows.length, gridRowCount(n));
        for (var r = 0; r < rows.length; r++) {
          for (var c = 0; c < rows[r].length; c++) {
            expect(gridSlot(rows[r][c], n), (row: r, col: c));
          }
        }
      }
    });
  });
}
