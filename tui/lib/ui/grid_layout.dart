/// Pure grid math (spec tui-wall: "Grid layout with a six-tile cap" —
/// `ceil(sqrt(n))` columns, row-major). The cap and LRU swap-in live in
/// WallState/WallStore; this file only answers "where does slot i go".
library;

import 'dart:math' as math;

/// Number of columns for [n] tiles: `ceil(sqrt(n))`. 0 for an empty grid.
int gridColumns(int n) => n <= 0 ? 0 : math.sqrt(n).ceil();

/// Number of rows for [n] tiles at [gridColumns] columns.
int gridRowCount(int n) => n <= 0 ? 0 : (n + gridColumns(n) - 1) ~/ gridColumns(n);

/// Row-major position of slot [index] in an [n]-tile grid.
({int row, int col}) gridSlot(int index, int n) {
  assert(index >= 0 && index < n);
  final cols = gridColumns(n);
  return (row: index ~/ cols, col: index % cols);
}

/// Slot indices per row, row-major — the shape the render loop walks.
/// The last row may be short; the renderer pads it so columns stay aligned.
List<List<int>> gridRows(int n) {
  final cols = gridColumns(n);
  return [
    for (var start = 0; start < n; start += cols)
      [for (var i = start; i < math.min(start + cols, n); i++) i],
  ];
}

/// Integer cell rect for slot [index] of an [n]-tile grid filling a
/// [width]×[height] area. Edges land on `k*extent/count` boundaries so the
/// rects tile the area exactly (no gaps, no overlap, remainders spread).
///
/// The grid renders as absolutely-positioned tiles in a Stack — NOT nested
/// Row/Column — so each tile keeps its element (and its terminal buffer)
/// across reshapes; only its rect changes.
({int x, int y, int width, int height}) gridCellRect(
    int index, int n, int width, int height) {
  final cols = gridColumns(n);
  final rows = gridRowCount(n);
  final slot = gridSlot(index, n);
  final x0 = (slot.col * width) ~/ cols;
  final x1 = ((slot.col + 1) * width) ~/ cols;
  final y0 = (slot.row * height) ~/ rows;
  final y1 = ((slot.row + 1) * height) ~/ rows;
  return (x: x0, y: y0, width: x1 - x0, height: y1 - y0);
}
