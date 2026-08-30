/// Ordered, flush-safe writes to a tile's PTY.
///
/// The vendored PtyHandler.write() does `stdin.write(data); stdin.flush()`.
/// Dart's IOSink throws a synchronous StateError ("StreamSink is bound to a
/// stream") from add/flush while a previous flush() future is still pending —
/// so the SECOND key event in a single stdin chunk (fast typing, tmux
/// send-keys "ls" Enter) threw mid-dispatch and silently dropped itself and
/// every event after it in that chunk. All engaged-tile writes therefore go
/// through this queue: data is buffered in order and re-tried until the sink
/// accepts it.
library;

import 'dart:async';

import 'package:nocterm/nocterm.dart' show PtyController;

class PtyWriter {
  /// Production shape: bound to a [PtyController].
  PtyWriter(PtyController controller)
      : _isRunning = (() => controller.isRunning),
        _rawWrite = controller.write;

  /// Test shape: raw sink functions injected.
  PtyWriter.raw({
    required bool Function() isRunning,
    required void Function(String data) rawWrite,
  })  : _isRunning = isRunning,
        _rawWrite = rawWrite;

  final bool Function() _isRunning;
  final void Function(String data) _rawWrite;
  final List<String> _pending = [];
  Timer? _retry;

  /// Interval between drain retries while the stdin sink is mid-flush.
  /// A flush to a pipe completes within one event-loop turn, so the first
  /// retry almost always succeeds.
  static const Duration retryDelay = Duration(milliseconds: 1);

  /// True while data waits for a retry (test observability).
  bool get hasPending => _pending.isNotEmpty;

  /// Queue [data] for the PTY, preserving order across retries.
  void write(String data) {
    _pending.add(data);
    _drain();
  }

  void _drain() {
    if (_pending.isEmpty) return;
    if (!_isRunning()) {
      // Dead/restarting PTY: drop rather than replay a stale burst into the
      // reattached session later.
      _pending.clear();
      return;
    }
    final data = _pending.join();
    _pending
      ..clear()
      ..add(data);
    try {
      _rawWrite(data);
      _pending.clear();
    } on StateError {
      // stdin still flushing an earlier write — keep the joined data queued
      // and retry shortly (order preserved; new writes append behind it).
      _retry ??= Timer(retryDelay, () {
        _retry = null;
        _drain();
      });
    }
  }

  void dispose() {
    _retry?.cancel();
    _retry = null;
    _pending.clear();
  }
}
