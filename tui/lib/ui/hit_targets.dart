/// Pure click hit-mapping: region-local cell coordinates → semantic target
/// (tile index, rail row, triage queue row). Everything here is pure so it
/// unit-tests without a terminal — the ClickRegions in garage_tui.dart /
/// rail.dart / triage_overlay.dart are thin adapters over these.
library;

import '../state/salience.dart';
import 'grid_layout.dart';

/// Tile slot index containing the grid-local point ([col],[row]), or null
/// when the point falls in the dead space after the last short row. Walks
/// the same [gridCellRect] rects the renderer positions tiles with, so a
/// click can never disagree with the paint.
int? tileIndexAt(int col, int row, int n, int width, int height) {
  if (n <= 0 || width <= 0 || height <= 0) return null;
  if (col < 0 || row < 0 || col >= width || row >= height) return null;
  for (var i = 0; i < n; i++) {
    final r = gridCellRect(i, n, width, height);
    if (col >= r.x && col < r.x + r.width && row >= r.y && row < r.y + r.height) {
      return i;
    }
  }
  return null;
}

/// A rail row resolved from a click.
sealed class RailTarget {
  const RailTarget();
}

/// A workspace header row — [index] is the position in the salience-ordered
/// groups list (the same index the `1`–`9` bindings use).
class RailWorkspaceTarget extends RailTarget {
  const RailWorkspaceTarget(this.index);

  final int index;
}

/// A session row.
class RailSessionTarget extends RailTarget {
  const RailSessionTarget(this.id);

  final String id;
}

/// Maps a rail-content row (0-based, local to the rail's Column — the rail
/// renders one line per row: each group's header followed by its sessions)
/// to its target. Null for rows past the last group (or an empty rail).
RailTarget? railTargetAt(List<WorkspaceGroup> groups, int row) {
  if (row < 0) return null;
  var line = 0;
  for (var g = 0; g < groups.length; g++) {
    if (row == line) return RailWorkspaceTarget(g);
    line++;
    final sessions = groups[g].sessions;
    if (row < line + sessions.length) {
      return RailSessionTarget(sessions[row - line].id);
    }
    line += sessions.length;
  }
  return null;
}

/// Rows the triage modal's ClickRegion skips before the first queue row:
/// the Container's border (1) plus its vertical padding (1). The region
/// wraps the whole modal box so a border/padding click is still absorbed
/// (not treated as outside-the-modal), hence the offset.
const int triageModalRowOffset = 2;

/// Maps a modal-local row to a queue row index, or null for the border,
/// padding, blank line, or footer. [rowCount] is the number of queue rows.
int? triageRowIndexAt(int localRow, int rowCount) {
  final i = localRow - triageModalRowOffset;
  if (i < 0 || i >= rowCount) return null;
  return i;
}
