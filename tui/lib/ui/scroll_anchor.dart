/// Frozen-scrollback state for one tile (spec tui-scrollback).
///
/// Design.md "Scroll freeze": our own render path over the xterm buffer —
/// on freeze, record an ABSOLUTE anchor (total lines at freeze time) and
/// slice the buffer from it, so new live output never moves the visible
/// content. TerminalXterm's relative `scrollOffset` is deliberately unused.
///
/// Coordinates are "all-time" line numbers: `droppedLines` (lines trimmed
/// off the front of the xterm circular buffer once it hits maxLines) plus
/// the current buffer length. Anchoring in all-time coordinates keeps the
/// frozen window pinned to the same CONTENT even while the buffer trims —
/// the buffer-relative top line is recomputed per frame as
/// `anchor - viewHeight - linesUp - droppedLines`.
///
/// Pure math, no framework imports — unit-tested in
/// test/scroll_anchor_test.dart.
library;

import 'dart:math' as math;

/// Snapshot of the xterm buffer geometry, reported by the vendored
/// TerminalXterm each build (patch 6 in tui/vendor/NOCTERM_VERSION).
class BufferMetrics {
  const BufferMetrics({
    required this.totalLines,
    required this.droppedLines,
    required this.viewHeight,
  });

  /// Current buffer length (`terminal.buffer.lines.length`).
  final int totalLines;

  /// Lines trimmed off the buffer front since terminal creation
  /// (monotonic; 0 until the circular buffer overflows).
  final int droppedLines;

  /// Rows in the terminal viewport.
  final int viewHeight;

  /// All-time line count: every line ever completed, trimmed or not.
  int get allTimeTotal => droppedLines + totalLines;

  static const BufferMetrics zero =
      BufferMetrics(totalLines: 0, droppedLines: 0, viewHeight: 24);
}

/// Per-tile scroll state machine: {live | frozen(anchor, linesUp)}.
///
/// The anchor is the all-time total at freeze time — the bottom of the
/// frozen coordinate system. `linesUp` is how far above that anchor tail
/// the view has been scrolled; it reaches 0 → back to live.
class ScrollAnchor {
  /// Lines scrolled per mouse-wheel notch.
  static const int wheelLines = 3;

  /// Lines scrolled per Shift+PageUp/PageDown press (one-line overlap).
  static int pageLines(int viewHeight) => math.max(1, viewHeight - 1);

  bool _frozen = false;
  int _anchorAllTime = 0;
  int _linesUp = 0;

  bool get frozen => _frozen;

  /// All-time anchor captured at freeze; meaningless while live.
  int get anchorAllTime => _anchorAllTime;

  /// Distance scrolled above the anchor tail; 0 while live.
  int get linesUp => _linesUp;

  /// Scroll up by [lines]; freezes first if live, capturing the anchor at
  /// the current all-time total. Clamped at the buffer start — if there is
  /// no scrollback at all the tile simply stays live.
  void scrollUp(int lines, BufferMetrics m) {
    if (!_frozen) {
      _anchorAllTime = m.allTimeTotal;
      _linesUp = 0;
      _frozen = true;
    }
    _linesUp = math.min(_linesUp + lines, _maxLinesUp(m));
    if (_linesUp <= 0) {
      // Nothing above the viewport: freezing would be a no-op view, so
      // stay live (also covers scrollUp(0)).
      snapLive();
    }
  }

  /// Scroll down by [lines]; reaching or passing the anchor tail returns
  /// the tile to live-follow. No-op while live.
  void scrollDown(int lines) {
    if (!_frozen) return;
    _linesUp -= lines;
    if (_linesUp <= 0) snapLive();
  }

  /// Immediate return to live-follow (typing, End, paging past the tail).
  void snapLive() {
    _frozen = false;
    _linesUp = 0;
    _anchorAllTime = 0;
  }

  /// Buffer-relative index of the first visible line while frozen, or null
  /// while live (= follow the tail). Clamped to the current buffer start:
  /// content trimmed out from under the anchor pins the view at line 0.
  int? topLine(BufferMetrics m) {
    if (!_frozen) return null;
    return math.max(
        0, _anchorAllTime - m.viewHeight - _linesUp - m.droppedLines);
  }

  /// Lines arrived since freezing — the "+N lines" affordance count.
  int newLines(BufferMetrics m) =>
      _frozen ? math.max(0, m.allTimeTotal - _anchorAllTime) : 0;

  /// How far above the anchor tail the view can go: everything between the
  /// buffer start (all-time index droppedLines) and the anchor viewport.
  int _maxLinesUp(BufferMetrics m) =>
      math.max(0, _anchorAllTime - m.viewHeight - m.droppedLines);
}
