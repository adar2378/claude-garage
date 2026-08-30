// Salience visuals as pure functions (tui/lib/ui/theme.dart): amber is
// EXCLUSIVELY needs-input, done fades after 2 minutes, elapsed formatting
// ported from ui/src/lib/elapsed.js (spec tui-wall: "Rail, strip, and
// salience ladder").
import 'package:garage_tui/ui/theme.dart';
import 'package:test/test.dart';

void main() {
  const now = 10 * 60 * 1000; // t = 10 minutes

  group('glyphs', () {
    test('mockup glyph set', () {
      expect(glyphFor('needs-input'), '●');
      expect(glyphFor('working'), '◐');
      expect(glyphFor('done'), '✓');
      expect(glyphFor('idle'), '○');
      expect(glyphFor('restorable'), '⟳');
      expect(glyphFor('unknown'), '○');
    });
  });

  group('statusColor', () {
    test('amber exclusively for needs-input', () {
      expect(statusColor('needs-input', sinceMs: 0, nowMs: now),
          GarageColors.amber);
      for (final status in ['working', 'done', 'idle', 'restorable']) {
        expect(statusColor(status, sinceMs: now, nowMs: now),
            isNot(GarageColors.amber),
            reason: '$status must never be amber');
      }
    });

    test('done is green then fades after 2 minutes', () {
      expect(statusColor('done', sinceMs: now - 119_000, nowMs: now),
          GarageColors.green);
      expect(statusColor('done', sinceMs: now - 121_000, nowMs: now),
          GarageColors.faint);
      // Unknown transition time never fades.
      expect(statusColor('done', sinceMs: null, nowMs: now),
          GarageColors.green);
    });
  });

  group('formatElapsed', () {
    test('elapsed.js shapes', () {
      expect(formatElapsed(null, now), '—');
      expect(formatElapsed(now - 5_000, now), '0:05');
      expect(formatElapsed(now - 65_000, now), '1:05');
      expect(formatElapsed(now - 3_600_000, now), '1h 00m');
      expect(formatElapsed(now - 3_720_000, now), '1h 02m');
      expect(formatElapsed(0, 86_400_000), '1d');
    });

    test('future timestamps clamp to zero', () {
      expect(formatElapsed(now + 60_000, now), '0:00');
    });
  });

  group('elapsedFor', () {
    test('only needs-input (waiting) and working (running) show a timer', () {
      expect(elapsedFor('needs-input', now - 65_000, now), '1:05');
      expect(elapsedFor('working', now - 5_000, now), '0:05');
      expect(elapsedFor('done', now - 5_000, now), isNull);
      expect(elapsedFor('idle', now - 5_000, now), isNull);
      expect(elapsedFor('restorable', null, now), isNull);
    });
  });
}
