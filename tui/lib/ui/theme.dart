/// Status vocabulary for the wall — glyphs, colors, elapsed formatting.
///
/// The visual contract is the p7 UX mockup's salience ladder (mirrored from
/// `ui/src/lib/status.js` semantics): amber is EXCLUSIVELY needs-input — the
/// only loud state — `done` is green and fades 2 minutes after the
/// transition, `working`/`idle` are deliberately colourless neutrals.
///
/// Everything here is pure (time is injected) so it unit-tests without a
/// terminal.
library;

import 'package:nocterm/nocterm.dart';

/// Glyph set from the mockup: ● needs-input, ◐ working, ✓ done, ○ idle,
/// ⟳ restorable.
const Map<String, String> statusGlyphs = {
  'needs-input': '●',
  'working': '◐',
  'done': '✓',
  'idle': '○',
  'restorable': '⟳',
};

String glyphFor(String status) => statusGlyphs[status] ?? statusGlyphs['idle']!;

/// `done` stops being highlighted this long after the transition
/// (spec tui-wall: "Done fades").
const Duration doneFade = Duration(minutes: 2);

/// True when a `done` session's highlight has expired. Unknown `since` never
/// fades (better to over-highlight than to hide a fresh completion).
bool doneFaded(int? sinceMs, int nowMs) =>
    sinceMs != null && nowMs - sinceMs > doneFade.inMilliseconds;

/// The wall palette. Amber is reserved: nothing but needs-input may use it.
abstract final class GarageColors {
  /// Needs-input — the only hue that shouts.
  static const Color amber = Color.fromRGB(255, 179, 64);

  /// Fresh `done`.
  static const Color green = Colors.green;

  /// Normal foreground.
  static const Color fg = Colors.white;

  /// Working/idle neutral.
  static const Color dim = Colors.grey;

  /// De-emphasized (idle glyphs, non-gridded rail rows, faded done).
  static const Color faint = Colors.brightBlack;
}

/// Status → color per the salience ladder. [sinceMs]/[nowMs] drive the
/// done-fade.
Color statusColor(String status, {int? sinceMs, required int nowMs}) {
  switch (status) {
    case 'needs-input':
      return GarageColors.amber;
    case 'done':
      return doneFaded(sinceMs, nowMs) ? GarageColors.faint : GarageColors.green;
    case 'working':
      return GarageColors.dim;
    default: // idle, restorable
      return GarageColors.faint;
  }
}

/// Elapsed-time formatting, ported from `ui/src/lib/elapsed.js`
/// formatElapsed(): null → "—", under an hour → "M:SS", under a day →
/// "Hh MMm", else "Nd". Clamps negative deltas to 0 (clock skew reads as
/// "just now").
String formatElapsed(int? sinceMs, int nowMs) {
  if (sinceMs == null) return '—';
  final totalSeconds = ((nowMs - sinceMs).clamp(0, 1 << 62)) ~/ 1000;
  if (totalSeconds < 3600) {
    final m = totalSeconds ~/ 60;
    final s = totalSeconds % 60;
    return '$m:${s.toString().padLeft(2, '0')}';
  }
  if (totalSeconds < 86400) {
    final h = totalSeconds ~/ 3600;
    final m = (totalSeconds % 3600) ~/ 60;
    return '${h}h ${m.toString().padLeft(2, '0')}m';
  }
  return '${totalSeconds ~/ 86400}d';
}

/// The tile/rail timer: waiting time for needs-input, working time for
/// working — the two states where "how long" matters. Null for the rest.
String? elapsedFor(String status, int? sinceMs, int nowMs) {
  if (status != 'needs-input' && status != 'working') return null;
  return formatElapsed(sinceMs, nowMs);
}
