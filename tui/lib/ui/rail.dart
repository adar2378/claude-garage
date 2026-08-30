/// The left workspace rail (spec tui-wall: "Rail, strip, and salience
/// ladder"): workspaces in salience order with their sessions — glyph,
/// label, elapsed — amber rows exclusively for needs-input, per-workspace
/// blocked counts, non-gridded sessions dimmed, and a reserved-column `▸`
/// marker on the FOCUSED session's row (p8.3) so the rail always shows
/// which session the wall's focus sits on. The marker column exists on
/// every session row, so rows never shift as focus moves and the
/// one-line-per-row click mapping (railTargetAt) is unchanged.
///
/// Rows are plain text lines, so click routing is one ClickRegion around
/// the row Column (local row index == rendered line) resolved through the
/// pure [railTargetAt] — a workspace header click focuses that workspace,
/// a session row click focuses that session (grid swap-in for overflow).
library;

import 'package:nocterm/nocterm.dart';

import '../state/salience.dart';
import '../state/wall_state.dart';
import 'click_region.dart';
import 'hit_targets.dart';
import 'theme.dart';

class Rail extends StatelessComponent {
  const Rail({
    super.key,
    required this.state,
    required this.nowMs,
    this.width = 28,
    this.onTarget,
  });

  final WallState state;
  final int nowMs;
  final int width;

  /// A rail row was clicked. Null renders a click-inert rail (tests).
  final void Function(RailTarget target)? onTarget;

  @override
  Component build(BuildContext context) {
    final gridded = state.griddedSessionIds.toSet();
    final rows = <Component>[];

    for (var i = 0; i < state.groups.length; i++) {
      final group = state.groups[i];
      final focusedWs = group.name == state.focusedWorkspace;
      rows.add(_workspaceRow(group, i, focusedWs));
      for (final s in group.sessions) {
        rows.add(_sessionRow(
          s,
          gridded: focusedWs && gridded.contains(s.id),
          focused: focusedWs && s.id == state.focusedSessionId,
        ));
      }
    }
    if (rows.isEmpty) {
      rows.add(const Text(' no workspaces',
          style: TextStyle(color: GarageColors.faint)));
    }

    Component column = Column(
      crossAxisAlignment: CrossAxisAlignment.start,
      children: rows,
    );
    final onTarget = this.onTarget;
    if (onTarget != null) {
      // Inside the Container so the border inset never skews row indices:
      // region-local row r IS rendered row r of the Column.
      column = ClickRegion(
        onClick: (col, row) {
          final target = railTargetAt(state.groups, row);
          if (target != null) onTarget(target);
        },
        child: column,
      );
    }

    return Container(
      width: width.toDouble(),
      decoration: const BoxDecoration(
        border: BoxBorder(right: BorderSide(color: GarageColors.faint)),
      ),
      child: column,
    );
  }

  Component _workspaceRow(WorkspaceGroup group, int index, bool focused) {
    final blocked = group.sessions.where((s) => s.needsInput).length;
    return RichText(
      maxLines: 1,
      softWrap: false,
      text: TextSpan(children: [
        TextSpan(
            text: ' ${index + 1} ',
            style: const TextStyle(color: GarageColors.dim)),
        TextSpan(
          text: group.name,
          style: TextStyle(
            color: focused ? GarageColors.fg : GarageColors.dim,
            fontWeight: focused ? FontWeight.bold : null,
          ),
        ),
        if (blocked > 0)
          TextSpan(
              text: '  ● $blocked',
              style: const TextStyle(color: GarageColors.amber)),
      ]),
    );
  }

  Component _sessionRow(WallSession s,
      {required bool gridded, required bool focused}) {
    // Amber row exclusively for needs-input; everything else follows the
    // status ladder, dimmed further when the session has no tile. The
    // FOCUSED session (spec tui-wall "Rail focus marker") carries a `▸`
    // marker plus a bright/bold label — never amber (amber stays exclusive
    // to needs-input, whose label keeps its hue and just gains bold). The
    // marker column is always reserved so rows never shift as focus moves.
    final glyphColor = statusColor(s.status, sinceMs: s.since, nowMs: nowMs);
    final labelColor = s.needsInput
        ? GarageColors.amber
        : focused
            ? GarageColors.fg
            : gridded
                ? GarageColors.dim
                : GarageColors.faint;
    final elapsed = elapsedFor(s.status, s.since, nowMs);
    return RichText(
      maxLines: 1,
      softWrap: false,
      text: TextSpan(children: [
        TextSpan(
            text: focused ? ' ▸ ' : '   ',
            style: const TextStyle(
                color: GarageColors.fg, fontWeight: FontWeight.bold)),
        TextSpan(text: '${glyphFor(s.status)} ',
            style: TextStyle(color: gridded ? glyphColor : GarageColors.faint)),
        TextSpan(
            text: s.label,
            style: TextStyle(
                color: labelColor,
                fontWeight: focused ? FontWeight.bold : null)),
        if (elapsed != null)
          TextSpan(text: ' $elapsed',
              style: TextStyle(
                  color: s.needsInput
                      ? GarageColors.amber
                      : GarageColors.faint)),
      ]),
    );
  }
}
