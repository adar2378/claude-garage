/// The triage queue overlay (spec tui-triage: "Triage queue overlay"):
/// every needs-input session across workspaces, sorted by waiting time
/// (longest first), each row showing the amber glyph, workspace/label,
/// waiting duration, and the daemon's notification message when present.
///
/// The queue model (row ordering, selection wrap) is pure and lives here so
/// it unit-tests without a terminal; the selection index itself is held by
/// the app state (bin/garage_tui.dart), reset each time the overlay opens.
///
/// Click routing: a full-screen barrier ClickRegion closes on any click
/// outside the modal (and stops clicks reaching the wall underneath); a
/// second ClickRegion around the modal box absorbs inside clicks — a queue
/// row click selects and jump-engages it (same landing as Enter), clicks
/// on the border/padding/footer do nothing. Row mapping is the pure
/// [triageRowIndexAt].
library;

import 'package:nocterm/nocterm.dart';

import '../state/wall_state.dart';
import 'click_region.dart';
import 'hit_targets.dart';
import 'theme.dart';

/// Queue rows: needs-input sessions only, longest-waiting first (smallest
/// `since`). Stable for ties, and a null `since` (unknown transition time)
/// sorts after every known one — mirroring `jumpTarget`'s "a null since
/// never beats a known one".
List<WallSession> triageQueueRows(List<WallSession> sessions) {
  final blocked = [
    for (final s in sessions)
      if (s.needsInput) s,
  ];
  final indexed = blocked.asMap().entries.toList()
    ..sort((a, b) {
      final sa = a.value.since;
      final sb = b.value.since;
      if (sa != sb) {
        if (sa == null) return 1;
        if (sb == null) return -1;
        final byWait = sa.compareTo(sb);
        if (byWait != 0) return byWait;
      }
      return a.key.compareTo(b.key); // stable: Dart's List.sort is not
    });
  return [for (final e in indexed) e.value];
}

/// j/k selection movement with wrap-around. Degenerate lengths pin to 0.
int wrapSelection(int current, int delta, int length) {
  if (length <= 0) return 0;
  final next = (current.clamp(0, length - 1) + delta) % length;
  return next < 0 ? next + length : next;
}

/// Truncate [text] to [width] cells with a trailing ellipsis.
String _fit(String text, int width) {
  if (width <= 0) return '';
  if (text.length <= width) return text;
  return width == 1 ? '…' : '${text.substring(0, width - 1)}…';
}

class TriageOverlay extends StatelessComponent {
  const TriageOverlay({
    super.key,
    required this.rows,
    required this.selectedIndex,
    required this.nowMs,
    this.onRowClick,
    this.onDismiss,
  });

  /// Pre-sorted via [triageQueueRows].
  final List<WallSession> rows;
  final int selectedIndex;
  final int nowMs;

  /// A queue row was clicked: select it and jump-engage (Enter's landing).
  final void Function(int index, WallSession row)? onRowClick;

  /// A click landed outside the modal box (matches Esc).
  final void Function()? onDismiss;

  @override
  Component build(BuildContext context) {
    return LayoutBuilder(builder: (context, constraints) {
      // Border (2) + horizontal padding (4) around the text column.
      final boxWidth =
          (constraints.maxWidth.floor() - 8).clamp(24, 78);
      final textWidth = boxWidth - 6;
      final selected =
          rows.isEmpty ? -1 : selectedIndex.clamp(0, rows.length - 1);

      Component modal = Container(
        width: boxWidth.toDouble(),
        decoration: BoxDecoration(
          color: Colors.black,
          border: BoxBorder.all(
              color: GarageColors.dim, style: BoxBorderStyle.rounded),
          title: const BorderTitle(
            text: ' triage queue ',
            style: TextStyle(color: GarageColors.fg),
          ),
        ),
        padding: const EdgeInsets.symmetric(horizontal: 2, vertical: 1),
        child: Column(
          mainAxisSize: MainAxisSize.min,
          crossAxisAlignment: CrossAxisAlignment.start,
          children: [
            if (rows.isEmpty)
              const Text('nothing needs you',
                  style: TextStyle(color: GarageColors.dim))
            else
              for (var i = 0; i < rows.length; i++)
                _row(rows[i], selected: i == selected, width: textWidth),
            const Text(''),
            const Text('j/k move · Enter jump · Esc close',
                style: TextStyle(color: GarageColors.faint)),
          ],
        ),
      );

      final onRowClick = this.onRowClick;
      final onDismiss = this.onDismiss;
      if (onRowClick == null && onDismiss == null) {
        return Center(child: modal); // click-inert (tests)
      }

      // Inner region absorbs every click on the modal box (border and
      // padding included) so the barrier only sees true outside clicks.
      final absorber = ClickAbsorber();
      modal = ClickRegion(
        absorbs: absorber,
        onClick: (col, row) {
          final i = triageRowIndexAt(row, rows.length);
          if (i != null) onRowClick?.call(i, rows[i]);
        },
        child: modal,
      );
      return ClickRegion(
        yieldsTo: absorber,
        onClick: (_, __) => onDismiss?.call(),
        child: Center(child: modal),
      );
    });
  }

  Component _row(WallSession s, {required bool selected, required int width}) {
    final identity = '${s.workspace}/${s.label}';
    final waiting = formatElapsed(s.since, nowMs);
    // '▸ ● ' prefix (4) + identity + 2 spaces + waiting + 2 spaces.
    final head = 4 + identity.length + 2 + waiting.length;
    final messageWidth = width - head - 2;
    final message =
        s.message == null ? null : _fit(s.message!, messageWidth);
    return RichText(
      maxLines: 1,
      softWrap: false,
      text: TextSpan(children: [
        TextSpan(
            text: selected ? '▸ ' : '  ',
            style: const TextStyle(color: GarageColors.fg)),
        const TextSpan(
            text: '● ', style: TextStyle(color: GarageColors.amber)),
        TextSpan(
          text: _fit(identity, width - 4),
          style: TextStyle(
            color: selected ? GarageColors.fg : GarageColors.dim,
            fontWeight: selected ? FontWeight.bold : null,
          ),
        ),
        TextSpan(
            text: '  $waiting',
            style: const TextStyle(color: GarageColors.amber)),
        if (message != null && message.isNotEmpty)
          TextSpan(
              text: '  $message',
              style: const TextStyle(color: GarageColors.faint)),
      ]),
    );
  }
}
