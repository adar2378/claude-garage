/// PTY controller registry for the tile grid.
///
/// Controllers live here, keyed by session id, created/disposed by
/// [sync]ing against the WallState's gridded set — never inside build()
/// (build must stay a pure function of state; see design.md "State
/// architecture"). Each controller runs `tmux attach -t =<id>` with `TMUX`
/// stripped from the child environment so nested tmux never refuses, and
/// killing the TUI only kills attach clients — tmux owns the sessions.
///
/// Reattach loop (spec tui-wall: "Tile PTY death recovery"): when an attach
/// PTY exits while the daemon still lists the session live, restart after
/// 500 ms, at most 4 attempts (4 × 500 ms = the 2-second reattach budget) —
/// then the tile shows a dead placeholder with the reason. A PTY that stayed
/// up long enough to count as healthy resets the attempt counter, so an
/// inner session recreated hours later gets a fresh budget.
library;

import 'dart:async';
import 'dart:io';

import 'package:nocterm/nocterm.dart' show PtyController;

import 'pty_writer.dart';

/// Pure retry policy for the reattach loop — extracted so the backoff rules
/// unit-test without processes or timers.
class ReattachPolicy {
  ReattachPolicy({
    this.maxAttempts = 4,
    this.retryDelay = const Duration(milliseconds: 500),
    this.healthyUptime = const Duration(seconds: 5),
  });

  final int maxAttempts;
  final Duration retryDelay;

  /// An attach that survived this long counts as healthy: the next exit
  /// starts a fresh attempt budget instead of inheriting old failures.
  final Duration healthyUptime;

  int _attempts = 0;
  int get attempts => _attempts;

  /// Record an exit after [uptime] of running. Returns the delay before the
  /// next restart, or null when the budget is exhausted (dead placeholder).
  Duration? onExit(Duration uptime) {
    if (uptime >= healthyUptime) _attempts = 0;
    if (_attempts >= maxAttempts) return null;
    _attempts++;
    return retryDelay;
  }
}

/// One registry slot: the controller plus its retry bookkeeping.
class _TileEntry {
  _TileEntry(this.controller)
      : writer = PtyWriter(controller),
        startedAt = DateTime.now();

  final PtyController controller;
  final PtyWriter writer;
  final ReattachPolicy policy = ReattachPolicy();
  DateTime startedAt;
  Timer? retryTimer;
  bool started = false;

  /// Pre-attach history captured from tmux (CRLF-normalized), non-null only
  /// when the registry's seedHistory flag is on and capture succeeded.
  String? seedText;

  /// Non-null once the reattach budget is exhausted — the tile renders a
  /// dead placeholder with this text.
  String? deadReason;
}

/// Attach environment: strip TMUX (nested attach must not refuse) and pin
/// TERM (tiles emulate xterm-256color regardless of the host terminal).
Map<String, String> attachEnvironment() =>
    Map<String, String>.of(Platform.environment)
      ..remove('TMUX')
      ..['TERM'] = 'xterm-256color';

class TilePtyRegistry {
  TilePtyRegistry({
    required this.isSessionLive,
    this.onChanged,
    this.attachStagger = const Duration(milliseconds: 50),
    this.seedHistory = false,
    PtyController Function(String sessionId)? createController,
    String? Function(String sessionId)? capturePane,
  })  : _create = createController ?? _defaultCreate,
        _capturePane = capturePane ?? _defaultCapturePane;

  /// Capture-pane history seeding (spec tui-scrollback "History depth",
  /// task 7.3). OFF by default. When on, `tmux capture-pane -p -S -1000` is
  /// run for each session as its registry entry is created — before the
  /// first attach — and the output is exposed via [seedFor] for the tile to
  /// pass to TerminalXterm's initialContent (patch 6), so pre-attach output
  /// is scrollable.
  ///
  /// Seam behavior: the capture includes the pane's CURRENT visible screen,
  /// and `tmux attach` repaints that same screen as its first output — so
  /// the tile's scrollback carries one duplicated screenful at the
  /// attach seam (seeded copy directly above the live repaint). Lines
  /// longer than the tile's 80 columns re-wrap on write.
  final bool seedHistory;

  /// Whether the daemon still lists this session as live — consulted on PTY
  /// exit to pick reattach vs. let-reconciliation-remove-it.
  final bool Function(String sessionId) isSessionLive;

  /// Fires after a lifecycle change a render pass should reflect (attach
  /// died, reattach succeeded, dead placeholder). Never fires from sync()
  /// itself — the caller is already reacting to a state change.
  final void Function()? onChanged;

  /// Initial PTY attaches are staggered this far apart to keep the first
  /// frames cheap (design.md "Rendering budget").
  final Duration attachStagger;

  final PtyController Function(String sessionId) _create;
  final String? Function(String sessionId) _capturePane;
  final Map<String, _TileEntry> _entries = {};

  /// When the next scheduled attach may fire — the stagger clock.
  DateTime _nextAttachAt = DateTime.now();

  static PtyController _defaultCreate(String sessionId) => PtyController(
        command: 'tmux',
        // `=` forces exact-name matching: garage ids contain `/`, which tmux
        // would otherwise treat as a fuzzy prefix pattern.
        arguments: ['attach', '-t', '=$sessionId'],
        environment: attachEnvironment(),
      );

  /// Synchronous by design: the seed must exist before the tile can build
  /// its TerminalXterm (initialContent only applies at initState), and
  /// capture-pane over a local socket is single-digit milliseconds. Any
  /// failure (session vanished mid-sync, tmux missing) degrades to no seed.
  static String? _defaultCapturePane(String sessionId) {
    try {
      final r = Process.runSync(
          'tmux', ['capture-pane', '-p', '-S', '-1000', '-t', '=$sessionId'],
          environment: attachEnvironment());
      if (r.exitCode != 0) return null;
      final out = r.stdout as String;
      if (out.isEmpty) return null;
      // xterm treats a bare \n as linefeed-only; normalize to CRLF so the
      // seeded lines don't staircase.
      return out.replaceAll('\n', '\r\n');
    } on Object {
      return null;
    }
  }

  PtyController? controllerFor(String sessionId) =>
      _entries[sessionId]?.controller;

  /// Pre-attach history for this tile (CRLF-normalized capture-pane
  /// output), or null when seeding is off/failed or the entry is gone.
  /// Stable for the life of the entry, so the tile can hand it to
  /// TerminalXterm.initialContent without re-seeding on rebuilds.
  String? seedFor(String sessionId) => _entries[sessionId]?.seedText;

  /// The flush-safe write path for this tile's PTY (see pty_writer.dart) —
  /// engaged key input MUST go through this, never controller.write.
  PtyWriter? writerFor(String sessionId) => _entries[sessionId]?.writer;

  String? deadReasonFor(String sessionId) => _entries[sessionId]?.deadReason;

  /// True once the controller for [sessionId] exists and is running.
  bool isRunning(String sessionId) =>
      _entries[sessionId]?.controller.isRunning ?? false;

  /// Reconcile the registry against the ids that should have live PTYs
  /// (the gridded, live sessions): create missing controllers (attach
  /// staggered), dispose controllers whose id left the set. Restorable
  /// sessions must not be passed in — they render placeholders, no PTY is
  /// ever spawned for them.
  void sync(Iterable<String> liveGriddedIds) {
    final want = liveGriddedIds.toSet();
    for (final id in _entries.keys.where((id) => !want.contains(id)).toList()) {
      _remove(id);
    }
    for (final id in want) {
      if (_entries.containsKey(id)) continue;
      final entry = _TileEntry(_create(id));
      if (seedHistory) entry.seedText = _capturePane(id);
      _entries[id] = entry;
      _scheduleStart(id, entry);
    }
  }

  /// Stagger clock: each new attach fires >= [attachStagger] after the one
  /// before it, never in the past.
  void _scheduleStart(String id, _TileEntry entry) {
    final now = DateTime.now();
    var at = _nextAttachAt.isAfter(now) ? _nextAttachAt : now;
    _nextAttachAt = at.add(attachStagger);
    entry.retryTimer = Timer(at.difference(now), () => _start(id, entry));
  }

  Future<void> _start(String id, _TileEntry entry) async {
    if (_entries[id] != entry) return; // removed while waiting
    entry.startedAt = DateTime.now();
    try {
      if (entry.started) {
        // restart() = dispose + start with the same configuration.
        await entry.controller.restart();
      } else {
        entry.started = true;
        // 80×24 is only the spawn default: on the controller's transition
        // to running, the tile's TerminalXterm re-pushes its laid-out size
        // (vendored patch 8), so the attach client matches the tile.
        await entry.controller.start(columns: 80, rows: 24);
        entry.controller.addExitCallback((code) => _onExit(id, entry, code));
      }
      onChanged?.call();
    } on Object catch (e) {
      _handleExitOrFailure(id, entry, 'attach failed: $e');
    }
  }

  void _onExit(String id, _TileEntry entry, int code) {
    if (_entries[id] != entry) return; // stale callback after removal
    _handleExitOrFailure(id, entry, 'attach exited (code $code)');
  }

  void _handleExitOrFailure(String id, _TileEntry entry, String what) {
    if (!isSessionLive(id)) {
      // Session gone or restorable — reconciliation (sync from the next
      // sessions refetch) removes the entry; nothing to retry against.
      onChanged?.call();
      return;
    }
    final uptime = DateTime.now().difference(entry.startedAt);
    final delay = entry.policy.onExit(uptime);
    if (delay == null) {
      entry.deadReason =
          '$what — gave up after ${entry.policy.maxAttempts} reattach attempts';
      onChanged?.call();
      return;
    }
    entry.retryTimer?.cancel();
    entry.retryTimer = Timer(delay, () => _start(id, entry));
    onChanged?.call();
  }

  void _remove(String id) {
    final entry = _entries.remove(id);
    if (entry == null) return;
    entry.retryTimer?.cancel();
    entry.writer.dispose();
    // Kill the attach client (never the tmux session) and release the PTY.
    if (entry.controller.isRunning) entry.controller.kill();
    unawaited(entry.controller.dispose());
  }

  /// Tear down every PTY (quit path). tmux sessions survive — only attach
  /// clients die.
  void disposeAll() {
    for (final id in _entries.keys.toList()) {
      _remove(id);
    }
  }
}
