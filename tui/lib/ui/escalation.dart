/// Off-screen escalation (spec tui-triage: "Off-screen escalation"):
///  - terminal bell (BEL, SSH-safe) once per session ENTERING needs-input;
///  - OSC 0 terminal title carrying the blocked count — `(N) garage` when
///    N > 0, plain `garage` at zero — re-emitted on every count change;
///  - visibility heartbeat (`POST /api/ui/visibility`) so daemon-side macOS
///    notifications stay suppressed while the TUI runs, same contract as
///    the web UI.
///
/// [EscalationPolicy] is a pure policy class: terminal writes go through an
/// injected sink so it unit-tests without a terminal. The sink receives one
/// string per update; the caller decides how to emit it safely between
/// frames (bin/garage_tui.dart routes it through nocterm's terminal write
/// buffer, which Dart's single-threaded event loop keeps clear of frame
/// paints).
library;

import 'dart:async';
import 'dart:math';

import '../state/wall_state.dart';

class EscalationPolicy {
  EscalationPolicy({required this.write, this.appName = 'garage'});

  /// Raw terminal sink (injected for tests).
  final void Function(String data) write;
  final String appName;

  static const String bel = '\x07';

  Set<String> _blocked = const {};
  int? _titleCount;

  /// Title text for a blocked count (pure; unit-tested directly).
  String titleFor(int blocked) => blocked > 0 ? '($blocked) $appName' : appName;

  /// OSC 0 (icon + window title), BEL-terminated — the most widely supported
  /// terminator, SSH-safe.
  static String oscTitle(String title) => '\x1b]0;$title$bel';

  /// Feed the current session snapshot. Emits at most one write per call:
  /// a single BEL when at least one session newly entered needs-input
  /// (coalesced — a batch of simultaneous transitions rings once), plus the
  /// title sequence whenever the blocked count changed (the very first call
  /// always claims the title, so the terminal shows `garage` from startup).
  /// A session already blocked in the previous snapshot never re-rings; one
  /// that left and re-entered does.
  void update(Iterable<WallSession> sessions) {
    final now = <String>{
      for (final s in sessions)
        if (s.needsInput) s.id,
    };
    final buffer = StringBuffer();
    if (now.difference(_blocked).isNotEmpty) buffer.write(bel);
    if (_titleCount != now.length) {
      buffer.write(oscTitle(titleFor(now.length)));
      _titleCount = now.length;
    }
    _blocked = now;
    if (buffer.isNotEmpty) write(buffer.toString());
  }
}

/// Visibility heartbeat: `visible: true` on start and every [interval],
/// `visible: false` once on [stop] (clean shutdown). Post failures are
/// swallowed — a daemon hiccup must never take the wall down.
class VisibilityHeartbeat {
  VisibilityHeartbeat({
    required this.post,
    String? clientId,
    this.interval = const Duration(seconds: 30),
  }) : clientId = clientId ?? _randomClientId();

  /// `GarageClient.postVisibility`, injected for tests.
  final Future<void> Function(String clientId, bool visible) post;

  /// Random per-process id — the daemon tracks visibility per client.
  final String clientId;
  final Duration interval;

  Timer? _timer;

  void start() {
    unawaited(_post(true));
    _timer ??= Timer.periodic(interval, (_) => unawaited(_post(true)));
  }

  /// Cancel the heartbeat and report invisible. Safe to call more than once.
  Future<void> stop() {
    _timer?.cancel();
    _timer = null;
    return _post(false);
  }

  Future<void> _post(bool visible) async {
    try {
      await post(clientId, visible);
    } on Object {
      // Daemon hiccup — the next beat (or the daemon's stale-client sweep)
      // reconciles.
    }
  }

  static String _randomClientId() {
    final rng = Random();
    final hex =
        List.generate(8, (_) => rng.nextInt(16).toRadixString(16)).join();
    final t = DateTime.now().millisecondsSinceEpoch.toRadixString(36);
    return 'tui-$t-$hex';
  }
}
