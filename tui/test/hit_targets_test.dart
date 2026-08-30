// Pure click hit-mapping (post-review fix: clicks were dead everywhere) —
// point→tile index via gridCellRect, point→rail row, triage modal rows.
import 'package:garage_tui/state/salience.dart';
import 'package:garage_tui/state/wall_state.dart';
import 'package:garage_tui/ui/grid_layout.dart';
import 'package:garage_tui/ui/hit_targets.dart';
import 'package:test/test.dart';

WallSession session(String id, {String status = 'idle'}) => WallSession(
      id: id,
      workspace: 'ws',
      label: id,
      status: status,
    );

WorkspaceGroup railGroup(String name, List<WallSession> sessions) =>
    WorkspaceGroup(name: name, registered: true, sessions: sessions);

void main() {
  group('tileIndexAt', () {
    test('single tile: every in-bounds point maps to slot 0', () {
      expect(tileIndexAt(0, 0, 1, 80, 24), 0);
      expect(tileIndexAt(79, 23, 1, 80, 24), 0);
      expect(tileIndexAt(40, 12, 1, 80, 24), 0);
    });

    test('out of bounds and degenerate inputs map to null', () {
      expect(tileIndexAt(-1, 0, 1, 80, 24), isNull);
      expect(tileIndexAt(0, -1, 1, 80, 24), isNull);
      expect(tileIndexAt(80, 0, 1, 80, 24), isNull);
      expect(tileIndexAt(0, 24, 1, 80, 24), isNull);
      expect(tileIndexAt(0, 0, 0, 80, 24), isNull);
      expect(tileIndexAt(0, 0, 3, 0, 24), isNull);
    });

    test('agrees with gridCellRect for every cell of every point (n=1..6)',
        () {
      // The invariant that matters: a click at (col,row) resolves to the
      // slot whose painted rect contains it — for every point of a small
      // area and every grid population the wall can show.
      const width = 17, height = 11; // odd sizes exercise remainder spread
      for (var n = 1; n <= 6; n++) {
        for (var row = 0; row < height; row++) {
          for (var col = 0; col < width; col++) {
            final hit = tileIndexAt(col, row, n, width, height);
            int? expected;
            for (var i = 0; i < n; i++) {
              final r = gridCellRect(i, n, width, height);
              if (col >= r.x &&
                  col < r.x + r.width &&
                  row >= r.y &&
                  row < r.y + r.height) {
                expected = i;
                break;
              }
            }
            expect(hit, expected,
                reason: 'n=$n point=($col,$row)');
          }
        }
      }
    });

    test('4 tiles in 8x4: quadrant corners land on the right slots', () {
      // cols=2, rows=2; cells are 4x2.
      expect(tileIndexAt(0, 0, 4, 8, 4), 0);
      expect(tileIndexAt(3, 1, 4, 8, 4), 0);
      expect(tileIndexAt(4, 0, 4, 8, 4), 1);
      expect(tileIndexAt(0, 2, 4, 8, 4), 2);
      expect(tileIndexAt(7, 3, 4, 8, 4), 3);
    });

    test('short last row: dead space maps to null, not a phantom tile', () {
      // n=3 → cols=2, rows=2; slot 2 is alone on the bottom row.
      const width = 8, height = 4;
      expect(tileIndexAt(1, 3, 3, width, height), 2);
      // Bottom-right quadrant has no slot.
      expect(tileIndexAt(7, 3, 3, width, height), isNull);
    });
  });

  group('railTargetAt', () {
    final groups = [
      railGroup('alpha', [session('a1'), session('a2')]),
      railGroup('beta', [session('b1')]),
    ];

    test('header rows map to workspace targets with group index', () {
      expect(railTargetAt(groups, 0),
          isA<RailWorkspaceTarget>().having((t) => t.index, 'index', 0));
      expect(railTargetAt(groups, 3),
          isA<RailWorkspaceTarget>().having((t) => t.index, 'index', 1));
    });

    test('session rows map to session ids', () {
      expect(railTargetAt(groups, 1),
          isA<RailSessionTarget>().having((t) => t.id, 'id', 'a1'));
      expect(railTargetAt(groups, 2),
          isA<RailSessionTarget>().having((t) => t.id, 'id', 'a2'));
      expect(railTargetAt(groups, 4),
          isA<RailSessionTarget>().having((t) => t.id, 'id', 'b1'));
    });

    test('rows past the last group (and negatives) map to null', () {
      expect(railTargetAt(groups, 5), isNull);
      expect(railTargetAt(groups, 99), isNull);
      expect(railTargetAt(groups, -1), isNull);
      expect(railTargetAt(const [], 0), isNull);
    });

    test('a session-less group is a single header row', () {
      final gs = [railGroup('empty', []), railGroup('full', [session('x')])];
      expect(railTargetAt(gs, 0),
          isA<RailWorkspaceTarget>().having((t) => t.index, 'index', 0));
      expect(railTargetAt(gs, 1),
          isA<RailWorkspaceTarget>().having((t) => t.index, 'index', 1));
      expect(railTargetAt(gs, 2),
          isA<RailSessionTarget>().having((t) => t.id, 'id', 'x'));
    });
  });

  group('triageRowIndexAt', () {
    test('rows sit below the border+padding offset', () {
      expect(triageModalRowOffset, 2);
      expect(triageRowIndexAt(2, 3), 0);
      expect(triageRowIndexAt(4, 3), 2);
    });

    test('border, padding, blank line and footer map to null', () {
      expect(triageRowIndexAt(0, 3), isNull); // top border
      expect(triageRowIndexAt(1, 3), isNull); // vertical padding
      expect(triageRowIndexAt(5, 3), isNull); // blank line after rows
      expect(triageRowIndexAt(6, 3), isNull); // footer
      expect(triageRowIndexAt(2, 0), isNull); // empty queue
      expect(triageRowIndexAt(-1, 3), isNull);
    });
  });
}
