// Frozen-scrollback anchor math (spec tui-scrollback; task group 7).
//
// The core scenario under test is design.md's "Scroll freeze" decision: the
// anchor is ABSOLUTE over the xterm buffer, so the frozen window's top line
// must not move while the buffer grows (new live output) and must stay
// pinned to the same content while the circular buffer trims lines off the
// front (droppedLines growth).
import 'package:garage_tui/ui/scroll_anchor.dart';
import 'package:test/test.dart';

BufferMetrics m({int total = 0, int dropped = 0, int view = 24}) =>
    BufferMetrics(totalLines: total, droppedLines: dropped, viewHeight: view);

void main() {
  group('freeze anchor capture', () {
    test('first scrollUp freezes with anchor = current all-time total', () {
      final a = ScrollAnchor();
      expect(a.frozen, isFalse);
      a.scrollUp(ScrollAnchor.pageLines(24), m(total: 200));
      expect(a.frozen, isTrue);
      expect(a.anchorAllTime, 200);
      expect(a.linesUp, 23); // one page = viewHeight - 1
    });

    test('anchor includes dropped lines (all-time coordinates)', () {
      final a = ScrollAnchor();
      a.scrollUp(10, m(total: 10000, dropped: 500));
      expect(a.anchorAllTime, 10500);
    });

    test('later scrollUps keep the original anchor', () {
      final a = ScrollAnchor();
      a.scrollUp(23, m(total: 200));
      a.scrollUp(23, m(total: 260)); // output arrived in between
      expect(a.anchorAllTime, 200);
      expect(a.linesUp, 46);
    });

    test('no scrollback available: stays live', () {
      final a = ScrollAnchor();
      // Buffer no taller than the viewport — nothing above to show.
      a.scrollUp(23, m(total: 24));
      expect(a.frozen, isFalse);
      expect(a.topLine(m(total: 24)), isNull);
    });
  });

  group('page clamping at buffer start', () {
    test('linesUp clamps so topLine never goes below 0', () {
      final a = ScrollAnchor();
      final metrics = m(total: 100); // 76 lines above the anchor viewport
      a.scrollUp(23, metrics);
      a.scrollUp(23, metrics);
      a.scrollUp(23, metrics);
      a.scrollUp(23, metrics); // would be 92, clamps to 76
      expect(a.linesUp, 76);
      expect(a.topLine(metrics), 0);
      // Further paging up is a no-op, still frozen.
      a.scrollUp(23, metrics);
      expect(a.linesUp, 76);
      expect(a.frozen, isTrue);
    });

    test('clamped-at-start view still pages back down symmetrically', () {
      final a = ScrollAnchor();
      final metrics = m(total: 50); // 26 above
      a.scrollUp(23, metrics);
      a.scrollUp(23, metrics); // clamps at 26
      expect(a.topLine(metrics), 0);
      a.scrollDown(23);
      expect(a.linesUp, 3);
      expect(a.topLine(metrics), 23);
    });
  });

  group('growth compensation (the spec core scenario)', () {
    test('topLine is unchanged while the buffer grows', () {
      final a = ScrollAnchor();
      a.scrollUp(23, m(total: 200));
      a.scrollUp(23, m(total: 200));
      final before = a.topLine(m(total: 200));
      expect(before, 200 - 24 - 46);
      // Live output streams in: total grows, anchor must not move.
      expect(a.topLine(m(total: 350)), before);
      expect(a.topLine(m(total: 5000)), before);
    });

    test('topLine tracks content while the buffer trims (droppedLines)', () {
      final a = ScrollAnchor();
      // Buffer at capacity 10000, 300 already dropped.
      a.scrollUp(23, m(total: 10000, dropped: 300));
      final top = a.topLine(m(total: 10000, dropped: 300))!;
      expect(top, 10300 - 24 - 23 - 300);
      // 40 more lines trimmed off the front: the same CONTENT is now 40
      // slots earlier in the buffer, so the buffer-relative top follows.
      expect(a.topLine(m(total: 10000, dropped: 340)), top - 40);
    });

    test('content trimmed out from under the anchor pins the view at 0', () {
      final a = ScrollAnchor();
      a.scrollUp(23, m(total: 100));
      a.scrollUp(23, m(total: 100)); // top = 100-24-46 = 30
      // Everything up to all-time line 60 has been trimmed away.
      expect(a.topLine(m(total: 100, dropped: 60)), 0);
    });
  });

  group('snap-to-live conditions', () {
    test('paging down past the anchor tail returns to live', () {
      final a = ScrollAnchor();
      a.scrollUp(23, m(total: 200));
      a.scrollDown(23);
      expect(a.frozen, isFalse);
      expect(a.topLine(m(total: 200)), isNull);
    });

    test('landing exactly on the tail returns to live', () {
      final a = ScrollAnchor();
      a.scrollUp(46, m(total: 200));
      a.scrollDown(23);
      expect(a.frozen, isTrue);
      a.scrollDown(23); // exactly 0
      expect(a.frozen, isFalse);
    });

    test('snapLive (typing / End) clears everything immediately', () {
      final a = ScrollAnchor();
      a.scrollUp(46, m(total: 200));
      a.snapLive();
      expect(a.frozen, isFalse);
      expect(a.linesUp, 0);
      expect(a.topLine(m(total: 400)), isNull);
      expect(a.newLines(m(total: 400)), 0);
    });

    test('scrollDown while live is a no-op', () {
      final a = ScrollAnchor();
      a.scrollDown(23);
      expect(a.frozen, isFalse);
    });

    test('wheel granularity: 3-line steps freeze and release too', () {
      final a = ScrollAnchor();
      a.scrollUp(ScrollAnchor.wheelLines, m(total: 200));
      expect(a.frozen, isTrue);
      expect(a.linesUp, 3);
      a.scrollDown(ScrollAnchor.wheelLines);
      expect(a.frozen, isFalse);
    });
  });

  group('+N counter', () {
    test('counts lines arrived since freezing', () {
      final a = ScrollAnchor();
      a.scrollUp(23, m(total: 200));
      expect(a.newLines(m(total: 200)), 0);
      expect(a.newLines(m(total: 265)), 65);
      // Trimming does not eat the count — all-time coordinates.
      expect(a.newLines(m(total: 265, dropped: 100)), 165);
    });

    test('zero while live', () {
      final a = ScrollAnchor();
      expect(a.newLines(m(total: 999)), 0);
    });
  });

  group('page size', () {
    test('viewHeight-1 with a floor of 1', () {
      expect(ScrollAnchor.pageLines(24), 23);
      expect(ScrollAnchor.pageLines(1), 1);
      expect(ScrollAnchor.pageLines(0), 1);
    });
  });
}
