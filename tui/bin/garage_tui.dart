// claude-garage pit wall — TUI entry point.
//
// The full loop (p8 task 5.4): bootstrap → GarageClient fetches → WallStore →
// SSE events → render. The store is the only mutable slot; PTY controllers
// live in TilePtyRegistry, reconciled against the state — never in build().
import 'dart:async';
import 'dart:convert';
import 'dart:io' show Directory, File, FileMode, Platform, stdout;

import 'package:garage_tui/api/client.dart';
import 'package:garage_tui/api/sse.dart';
import 'package:garage_tui/bootstrap.dart';
import 'package:garage_tui/input/paste.dart';
import 'package:garage_tui/state/armed_close.dart';
import 'package:garage_tui/state/salience.dart' show restorableSessionIds;
import 'package:garage_tui/state/store.dart';
import 'package:garage_tui/state/wall_state.dart';
import 'package:garage_tui/state/workspace_form.dart';
import 'package:garage_tui/state/workspace_remove.dart';
import 'package:garage_tui/ui/click_region.dart';
import 'package:garage_tui/ui/escalation.dart';
import 'package:garage_tui/ui/grid_layout.dart';
import 'package:garage_tui/ui/help_overlay.dart';
import 'package:garage_tui/ui/hit_targets.dart';
import 'package:garage_tui/ui/rail.dart';
import 'package:garage_tui/ui/strip.dart';
import 'package:garage_tui/ui/theme.dart';
import 'package:garage_tui/ui/tile.dart';
import 'package:garage_tui/ui/tile_registry.dart';
import 'package:garage_tui/ui/triage_overlay.dart';
import 'package:garage_tui/ui/workspace_add_overlay.dart';
import 'package:nocterm/nocterm.dart';

Future<void> main() async {
  // Health-gate before touching the terminal: an actionable error beats a
  // blank alt-screen when the daemon is down.
  await requireDaemon();

  // Ctrl+G is the disengage chord (mockup contract), not the framework's
  // debug toggle — flip the vendored patch-4 flag before any input arrives.
  TerminalBinding.debugKeyEnabled = false;

  // Startup ordering per design.md "Rendering budget": daemon health check →
  // fetches → first frame → staggered PTY attaches.
  final client = GarageClient();
  final store = WallStore();
  try {
    store.workspacesFetched(await client.fetchWorkspaces());
    store.sessionsFetched(await client.fetchSessions());
  } on Object catch (e) {
    client.close();
    // Same shape as requireDaemon's failure path: fail before the alt-screen.
    // (The daemon was healthy a moment ago; this is a race, not a config error.)
    // ignore: avoid_print
    print('claude-garage daemon fetch failed: $e');
    return;
  }

  disableFlowControl();
  try {
    await runApp(GarageTuiApp(client: client, store: store));
  } finally {
    // Backstop for paths where runApp actually returns; the normal quit path
    // is shutdownTui(), because nocterm exits the process without unwinding.
    restoreFlowControl();
  }
}

class GarageTuiApp extends StatefulComponent {
  const GarageTuiApp({super.key, required this.client, required this.store});

  final GarageClient client;
  final WallStore store;

  @override
  State<GarageTuiApp> createState() => _GarageTuiAppState();
}

class _GarageTuiAppState extends State<GarageTuiApp> {
  WallStore get store => component.store;
  GarageClient get client => component.client;

  late final TilePtyRegistry registry;
  late final SseClient sse;
  final PasteForwarder pasteForwarder = PasteForwarder();

  late final EscalationPolicy escalation;
  late final VisibilityHeartbeat heartbeat;

  Timer? _ticker;
  Timer? _noticeTimer;
  bool _refetching = false;
  bool _spawning = false;
  String? _notice;

  /// Selection index into the triage queue rows; reset on every open.
  int _queueSelection = 0;

  /// The `x` double-press close arming (pure state machine, 3s window).
  final ArmedClose _armedClose = ArmedClose();

  /// The `X` double-press workspace-remove arming (same generic machine,
  /// keyed by workspace name — spec tui-key-routing "p8.3 workspace
  /// removal").
  final ArmedAction _armedRemove = ArmedAction();

  /// Sessions with a restore call in flight — their placeholders render
  /// "restoring…" (optimistic; the refetch after the response settles it).
  final Set<String> _restoringIds = {};

  /// Add-workspace overlay state (the overlay renders these; the logic
  /// lives here so path validation stays a pure-function concern).
  String? _workspaceAddError;
  bool _workspaceAddBusy = false;

  @override
  void initState() {
    super.initState();
    // fps15 cap (spike gate 1): the default 30fps saturates the single Dart
    // isolate under tile load and stdin reads starve. Must be set here —
    // SchedulerBinding.instance is null before runApp.
    SchedulerBinding.instance.targetFrameDuration = FrameRate.fps15;

    registry = TilePtyRegistry(
      isSessionLive: (id) => store.state.sessionById(id)?.live ?? false,
      onChanged: _onRegistryChanged,
    );

    // Off-screen escalation (spec tui-triage): bell + OSC title through the
    // terminal's own write buffer — event callbacks never run mid-frame on
    // the single isolate, so the sequences cannot interleave a frame paint.
    escalation = EscalationPolicy(write: _writeRawToTerminal);
    heartbeat = VisibilityHeartbeat(post: client.postVisibility)..start();

    store.onChange = (state) {
      _syncRegistry(state);
      escalation.update(state.sessions);
      setState(() {});
    };
    _syncRegistry(store.state); // initial gridded set → staggered attaches
    // Claim the title now; sessions already blocked at launch ring once.
    escalation.update(store.state.sessions);

    sse = SseClient(onEvent: _onSseEvent, onPollFallback: _refetchSessions);
    sse.start();

    // Elapsed timers and the done-fade must advance without input; 10s keeps
    // the "refresh ≤ 30s" contract with margin at negligible render cost.
    _ticker = Timer.periodic(const Duration(seconds: 10), (_) {
      setState(() {});
    });
  }

  @override
  void dispose() {
    _ticker?.cancel();
    _noticeTimer?.cancel();
    unawaited(heartbeat.stop());
    sse.stop();
    registry.disposeAll();
    super.dispose();
  }

  /// Raw escape-sequence sink for [escalation]: nocterm's terminal write
  /// buffer when the binding is up (write + flush is atomic per event-loop
  /// turn), bare stdout as the fallback.
  void _writeRawToTerminal(String data) {
    try {
      TerminalBinding.instance.terminal
        ..write(data)
        ..flush();
    } on Object {
      stdout.write(data);
    }
  }

  // ── Wiring ────────────────────────────────────────────────────────────

  /// PTYs exist exactly for the gridded, live sessions. Runs on every store
  /// change (and registry lifecycle events) — never from build().
  void _syncRegistry(WallState state) {
    registry.sync([
      for (final id in state.griddedSessionIds)
        if (state.sessionById(id)?.live ?? false) id,
    ]);
    // An engaged tile whose PTY died past the reattach budget would black-
    // hole keys (no focused Focusable renders for a dead placeholder) — drop
    // back to garage so the keyboard always has an owner.
    if (state.layer == KeyLayer.engaged) {
      final id = state.focusedSessionId;
      if (id != null && registry.deadReasonFor(id) != null) {
        store.disengage();
      }
    }
  }

  void _onRegistryChanged() {
    _syncRegistry(store.state);
    setState(() {});
  }

  void _onSseEvent(SseEvent event) {
    switch (event.event) {
      case 'status':
        try {
          final data = (jsonDecode(event.data) as Map).cast<String, Object?>();
          final status = data['status'] as String;
          store.statusChanged(
            data['id'] as String,
            status,
            data['since'] is num ? (data['since'] as num).toInt() : null,
          );
          // The status event carries no notification text; a needs-input
          // transition's `message` only exists on the listing, so pull it —
          // the triage queue must show the daemon's question (spec
          // tui-triage "Question text shown").
          if (status == 'needs-input') unawaited(_refetchSessions());
        } on Object {
          // Malformed event — the next sessions refetch reconciles.
        }
      case 'sessions':
        _refetchSessions();
    }
  }

  /// Trailing-coalesced: a trigger arriving while a refetch is in flight
  /// queues exactly one re-run instead of being dropped. Dropping it was a
  /// real staleness bug (p8.1): an SSE `sessions` event that landed during
  /// another refetch was lost, and — the session set now being stable — no
  /// further event ever came, so the wall stayed stale indefinitely (a
  /// killed session kept rendering as a dead attach instead of its
  /// restorable placeholder).
  bool _refetchQueued = false;

  Future<void> _refetchSessions() async {
    if (_refetching) {
      _refetchQueued = true;
      return;
    }
    _refetching = true;
    try {
      do {
        _refetchQueued = false;
        // Workspaces too: registrations change rarely, but a stale dir would
        // mis-derive worktree flags after an edit. Both fetches run after
        // the queued trigger, so the re-run always sees post-trigger state.
        store.workspacesFetched(await client.fetchWorkspaces());
        store.sessionsFetched(await client.fetchSessions());
      } while (_refetchQueued);
    } on Object {
      // Daemon hiccup — SSE reconnect/poll fallback retries shortly.
    } finally {
      _refetching = false;
      _refetchQueued = false;
    }
  }

  // ── Garage/overlay key layers ─────────────────────────────────────────

  // E2E latency instrumentation (task 10.2): when GARAGE_TUI_KEYLOG names a
  // file, append "<epoch-us> <key>" as each garage-layer key is handled.
  static final String? _keylogPath = Platform.environment['GARAGE_TUI_KEYLOG'];
  void _logKeyHandled(KeyboardEvent event) {
    final path = _keylogPath;
    if (path == null) return;
    File(path).writeAsStringSync(
        '${DateTime.now().microsecondsSinceEpoch} ${event.character ?? event.logicalKey}\n',
        mode: FileMode.append);
  }

  bool _onGarageKey(KeyboardEvent event) {
    _logKeyHandled(event);
    final state = store.state;
    if (state.layer == KeyLayer.overlay) {
      if (state.overlay == OverlayKind.workspaceAdd) {
        // The overlay's TextField owns the keys (the root Focusable yields
        // while it is open) — anything that still lands here is consumed.
        return true;
      }
      if (state.overlay == OverlayKind.triageQueue) {
        _onQueueKey(event);
      } else if (event.character == '?' ||
          event.character == 'q' ||
          event.logicalKey == LogicalKey.escape) {
        // Help legend: ? toggles, Esc/q also close.
        store.closeOverlay();
      }
      return true; // overlay layer consumes everything
    }

    final key = event.logicalKey == LogicalKey.enter ? '\r' : event.character;
    if (key == null || key.isEmpty) return true;
    // p8.4: while an `X` remove arm is active, `K` confirms the removal
    // WITH session kill — checked BEFORE the disarm pass (K would otherwise
    // disarm the very arm it confirms). Outside a live arm `K` falls
    // through as an ordinary unbound key (typing hint).
    if (key == 'K' && _killRemovePressed()) return true;
    // Any key but `x` disarms a pending close (spec tui-key-routing "p8.1
    // session lifecycle bindings": "any other key disarms"); same for `X`
    // and a pending workspace removal (p8.3) — the two arms are
    // independent, so an `X` disarms a pending `x` and vice versa. `K`
    // only survives the remove arm via the confirm branch above.
    if (key != 'x') _disarmClose();
    if (key != 'X') _disarmRemove();
    final command = garageCommandFor(key);
    if (command == null) {
      // Unbound garage keys are consumed (never reach an agent, spec
      // tui-key-routing), but silence read as a dead wall — show where the
      // keys actually go (post-review fix; never amber).
      if (_isPrintable(key)) _showTypingHint();
      return true;
    }
    if (command is OpenQueueCommand) _queueSelection = 0;
    if (command is WorkspaceAddCommand) {
      // Fresh overlay state on every open.
      _workspaceAddError = null;
      _workspaceAddBusy = false;
    }
    if (store.dispatch(command)) return true;

    // Commands the store declined: effects (spawn/restore/close/quit), the
    // empty-jump notice, and Enter on a restorable tile (restore effect).
    switch (command) {
      case SpawnCommand(:final worktree):
        unawaited(_spawn(worktree: worktree));
      case QuitCommand():
        _quit();
      case JumpCommand():
        // Spec tui-triage: no-op with a brief strip notice — never amber.
        _showNotice('no session needs you', ttl: const Duration(seconds: 2));
      case EngageCommand():
        // Engage declined: the focused tile is a restorable placeholder →
        // Enter restores it (spec tui-key-routing "p8.1").
        final s = store.state.sessionById(store.state.focusedSessionId);
        if (s != null && !s.live) unawaited(_restore(s.id));
      case RestoreAllCommand():
        unawaited(_restoreAll());
      case CloseCommand():
        _closePressed();
      case WorkspaceRemoveCommand():
        _removePressed();
      default:
        break;
    }
    return true; // garage layer consumes everything
  }

  /// Triage queue keys (spec tui-triage): j/k move with wrap, Enter closes
  /// and jump-and-engages the selected session, Esc closes.
  void _onQueueKey(KeyboardEvent event) {
    if (event.logicalKey == LogicalKey.escape) {
      store.closeOverlay();
      return;
    }
    final rows = triageQueueRows(store.state.sessions);
    if (event.character == 'j' || event.logicalKey == LogicalKey.arrowDown) {
      setState(() =>
          _queueSelection = wrapSelection(_queueSelection, 1, rows.length));
      return;
    }
    if (event.character == 'k' || event.logicalKey == LogicalKey.arrowUp) {
      setState(() =>
          _queueSelection = wrapSelection(_queueSelection, -1, rows.length));
      return;
    }
    if (event.logicalKey == LogicalKey.enter) {
      store.closeOverlay();
      if (rows.isEmpty) return;
      store.jumpToSession(rows[_queueSelection.clamp(0, rows.length - 1)].id);
    }
  }

  // ── Mouse click routing (spec tui-key-routing "or clicking a tile";
  // spec tui-triage "or clicking the strip badge") ──────────────────────
  //
  // Coarse ClickRegions per surface (grid / rail / badge / overlay) with
  // pure hit-mapping (hit_targets.dart); every action composes existing
  // store transitions — clicks introduce no new state semantics.

  /// Grid click: focus AND engage the tile (same landing as focus+Enter).
  /// While engaged, a click on a different tile migrates the engagement;
  /// a click on the engaged tile itself does nothing extra (and is NOT
  /// forwarded to the PTY — only wheel is forwarded, in tile.dart). A click
  /// on a restorable placeholder restores it (Enter's landing, spec
  /// tui-key-routing "p8.1"). While a tile is maximized it covers the whole
  /// grid, so every grid click targets it.
  void _onGridClick(int col, int row, int width, int height) {
    final state = store.state;
    if (state.layer == KeyLayer.overlay) return; // barrier covers this; belt+braces
    final ids = state.griddedSessionIds;
    final String id;
    if (state.maximizedSessionId != null) {
      id = state.maximizedSessionId!;
    } else {
      final index = tileIndexAt(col, row, ids.length, width, height);
      if (index == null) return;
      id = ids[index];
    }
    final session = state.sessionById(id);
    if (session != null && !session.live) {
      if (state.layer == KeyLayer.engaged) store.disengage();
      store.focusSession(id);
      unawaited(_restore(id));
      return;
    }
    if (state.layer == KeyLayer.engaged) {
      if (state.focusedSessionId == id) return;
      // Explicit disengage → focus → engage: engagement must never
      // transfer silently (store contract), so the migration is spelled out.
      store.disengage();
    }
    store.focusSession(id);
    store.engage();
  }

  /// Rail click: header focuses the workspace, session row focuses the
  /// session (grid swap-in for overflow) — never engages. A click while
  /// engaged drops back to garage first (engagement never transfers).
  void _onRailTarget(RailTarget target) {
    final state = store.state;
    if (state.layer == KeyLayer.overlay) return;
    if (state.layer == KeyLayer.engaged) store.disengage();
    switch (target) {
      case RailWorkspaceTarget(:final index):
        store.focusWorkspace(index);
      case RailSessionTarget(:final id):
        store.focusSession(id);
    }
  }

  /// Strip badge click: open the triage queue (the `A` binding).
  void _onBadgeClick() {
    final state = store.state;
    if (state.layer == KeyLayer.overlay) return; // overlays never stack
    if (state.layer == KeyLayer.engaged) store.disengage();
    _queueSelection = 0;
    store.openOverlay(OverlayKind.triageQueue);
  }

  /// Triage queue row click: select + jump-engage (Enter's landing).
  void _onQueueRowClick(int index, WallSession row) {
    setState(() => _queueSelection = index);
    store.closeOverlay();
    store.jumpToSession(row.id); // refuses stale rows, same as Enter
  }

  /// Spawn into the focused workspace with a generated `claude-N` label
  /// (spec tui-key-routing "Spawn from the rail"), then refetch so the tile
  /// appears.
  Future<void> _spawn({required bool worktree}) async {
    final workspace = store.state.focusedWorkspace;
    if (workspace == null || _spawning) return;
    _spawning = true;
    try {
      await client.spawnSession(workspace, _nextLabel(workspace),
          worktree: worktree);
      await _refetchSessions();
    } on Object catch (e) {
      _showNotice(e is GarageApiException ? 'spawn failed: ${e.message}' : 'spawn failed');
    } finally {
      _spawning = false;
    }
  }

  /// First free `claude-N` in the workspace (matches the web UI's default
  /// label family).
  String _nextLabel(String workspace) {
    final taken = <int>{};
    for (final s in store.state.sessions) {
      if (s.workspace != workspace) continue;
      final match = RegExp(r'^claude-(\d+)$').firstMatch(s.label);
      if (match != null) taken.add(int.parse(match.group(1)!));
    }
    var n = 1;
    while (taken.contains(n)) {
      n++;
    }
    return 'claude-$n';
  }

  // ── Restore / close / add-workspace effects (p8.1) ────────────────────

  /// Restore one restorable session (`POST /api/sessions/restore {id}`)
  /// with an optimistic "restoring…" placeholder; failures surface in the
  /// strip notice; the refetch settles the real status either way.
  Future<void> _restore(String id) async {
    if (_restoringIds.contains(id)) return;
    _restoringIds.add(id);
    setState(() {});
    try {
      final reason = await client.restoreSession(id);
      if (reason != null) _showNotice('restore failed: $reason');
    } on Object catch (e) {
      _showNotice(
          e is GarageApiException ? 'restore failed: ${e.message}' : 'restore failed');
    } finally {
      _restoringIds.remove(id);
      await _refetchSessions();
      if (mounted) setState(() {});
    }
  }

  /// `R`: restore ALL restorable sessions in the focused workspace —
  /// parallel per-id calls (the web UI's rail restore-all shape), so one
  /// failure never blocks the rest.
  Future<void> _restoreAll() async {
    final workspace = store.state.focusedWorkspace;
    if (workspace == null) return;
    final ids = restorableSessionIds(store.state.sessions, workspace)
        .where((id) => !_restoringIds.contains(id))
        .toList();
    if (ids.isEmpty) {
      _showNotice('nothing to restore in $workspace',
          ttl: const Duration(seconds: 2));
      return;
    }
    await Future.wait(ids.map(_restore));
  }

  /// `x`: armed double-press. First press arms (strip notice, 3s window);
  /// the second press on the same session within the window closes it.
  void _closePressed() {
    final state = store.state;
    final session = state.sessionById(state.focusedSessionId);
    if (session == null) return;
    final now = DateTime.now().millisecondsSinceEpoch;
    if (_armedClose.press(session.id, now)) {
      _clearArmNotice('press x again to close ');
      unawaited(_close(session));
    } else {
      _showNotice(_armNoticeFor(session.label),
          ttl: const Duration(seconds: 3));
    }
  }

  static String _armNoticeFor(String label) =>
      'press x again to close $label';

  void _disarmClose() {
    if (_armedClose.armedId == null) return;
    _armedClose.disarm();
    _clearArmNotice('press x again to close ');
  }

  void _clearArmNotice(String prefix) {
    if (_notice != null && _notice!.startsWith(prefix)) {
      _noticeTimer?.cancel();
      _notice = null;
      setState(() {});
    }
  }

  /// `X`: armed double-press removal of the FOCUSED workspace (spec
  /// tui-key-routing "p8.3 workspace removal"). Registered groups arm/
  /// confirm; a synthesized unregistered group (registered: false) has no
  /// registry entry to remove — explain instead of arming.
  void _removePressed() {
    final state = store.state;
    final group = state.groupByName(state.focusedWorkspace);
    if (group == null) return;
    if (!group.registered) {
      _showNotice(
          'already unregistered — sessions live in tmux; x closes them individually',
          ttl: const Duration(seconds: 5));
      return;
    }
    final now = DateTime.now().millisecondsSinceEpoch;
    if (_armedRemove.press(group.name, now)) {
      _clearArmNotice('press X again to remove ');
      unawaited(_removeWorkspace(group.name));
    } else {
      // Arm notice: with live sessions the p8.4 `K` clause is appended
      // (workspace_remove.dart owns the wording; zero live sessions omit it).
      final live = group.sessions.where((s) => s.live).length;
      _showNotice(removeArmNotice(group.name, live),
          ttl: const Duration(seconds: 3));
    }
  }

  /// `K` while the `X` remove arm is live (p8.4): confirm removal AND kill
  /// every live session — `DELETE /api/workspaces/<name>?sessions=kill`.
  /// Returns false when nothing (unexpired) is armed, so the caller treats
  /// `K` as an ordinary unbound key. Only registered workspaces can ever be
  /// armed (p8.3 blocks unregistered groups), and a refetch may have
  /// unregistered the target mid-arm — re-check before firing.
  bool _killRemovePressed() {
    final now = DateTime.now().millisecondsSinceEpoch;
    final name = confirmKillTarget(_armedRemove, now);
    if (name == null) return false;
    _clearArmNotice('press X again to remove ');
    final group = store.state.groupByName(name);
    if (group == null || !group.registered) return true; // stale arm: no-op
    unawaited(_removeWorkspaceKill(name));
    return true;
  }

  void _disarmRemove() {
    if (_armedRemove.armedId == null) return;
    _armedRemove.disarm();
    _clearArmNotice('press X again to remove ');
  }

  /// Registry-only `DELETE /api/workspaces/<name>` (the `X`-`X` confirm —
  /// never `?sessions=kill`): live sessions keep running and the refetch
  /// surfaces them again as a synthesized unregistered group.
  Future<void> _removeWorkspace(String name) async {
    try {
      await client.removeWorkspace(name);
      _showNotice('removed workspace $name — its sessions keep running',
          ttl: const Duration(seconds: 5));
    } on Object catch (e) {
      _showNotice(e is GarageApiException
          ? 'remove failed: ${e.message}'
          : 'remove failed');
    }
    await _refetchSessions();
  }

  /// The p8.4 `K` confirm: `DELETE /api/workspaces/<name>?sessions=kill` —
  /// the daemon kills every live `garage/<name>/*` tmux session, then drops
  /// the registration; the strip reports the kill count (or names the
  /// sessions that failed to die).
  Future<void> _removeWorkspaceKill(String name) async {
    try {
      final body = await client.removeWorkspace(name, killSessions: true);
      _showNotice(killRemoveNotice(name, body),
          ttl: const Duration(seconds: 5));
    } on Object catch (e) {
      _showNotice(e is GarageApiException
          ? 'remove failed: ${e.message}'
          : 'remove failed');
    }
    await _refetchSessions();
  }

  /// Close the session: a live one via plain `DELETE /api/sessions/<id>`
  /// (kills the tmux session and drops resume metadata); a restorable one
  /// via `?meta=1` (drops only the stored metadata — the plain DELETE 404s
  /// with no live tmux match). v1 worktree policy = keep: the daemon
  /// returns the worktree record and the strip says where to finish it.
  Future<void> _close(WallSession session) async {
    try {
      final worktree =
          await client.deleteSession(session.id, metaOnly: !session.live);
      if (worktree != null) {
        final branch = worktree['branch'] as String? ?? 'garage/${session.label}';
        _showNotice('worktree kept: $branch — merge or discard in the web wall',
            ttl: const Duration(seconds: 8));
      } else {
        _showNotice('closed ${session.label}', ttl: const Duration(seconds: 2));
      }
    } on Object catch (e) {
      _showNotice(
          e is GarageApiException ? 'close failed: ${e.message}' : 'close failed');
    }
    await _refetchSessions();
  }

  /// The add-workspace overlay's submit: `~` expansion, client-side
  /// dir-exists validation, web-UI-style name derivation, PUT, refetch,
  /// focus the new workspace. Errors keep the overlay open.
  Future<void> _submitWorkspaceAdd(String raw) async {
    if (_workspaceAddBusy) return;
    final path = expandTilde(raw, Platform.environment['HOME'] ?? '');
    if (path.isEmpty) {
      setState(() => _workspaceAddError = 'type a directory path');
      return;
    }
    if (!Directory(path).existsSync()) {
      setState(() => _workspaceAddError = 'no such directory: $path');
      return;
    }
    final existing = <String>{
      for (final w in store.state.workspaces) w.name,
      for (final g in store.state.groups) g.name,
    };
    final name = deriveWorkspaceName(path, existing);
    setState(() {
      _workspaceAddBusy = true;
      _workspaceAddError = null;
    });
    try {
      await client.putWorkspace(name, path);
      await _refetchSessions();
      store.closeOverlay();
      store.focusWorkspaceNamed(name);
      _showNotice('workspace $name added — press n to spawn a session',
          ttl: const Duration(seconds: 5));
    } on Object catch (e) {
      _workspaceAddError = e is GarageApiException
          ? e.message
          : 'could not register workspace';
    } finally {
      _workspaceAddBusy = false;
      if (mounted) setState(() {});
    }
  }

  void _cancelWorkspaceAdd() {
    _workspaceAddError = null;
    _workspaceAddBusy = false;
    store.closeOverlay();
  }

  /// A printable character (something the user plausibly meant as typing) —
  /// controls and escape-derived characters don't trigger the hint.
  static bool _isPrintable(String key) {
    final code = key.codeUnitAt(0);
    return code >= 0x20 && code != 0x7f;
  }

  static const String _typingHint =
      'enter engages the focused terminal — keys go to garage now';

  /// Garage-layer typing hint, rate-limited: while the hint is already up,
  /// further keypresses in the burst do NOT restart the timer — it clears
  /// ~2.5s after the keypress that raised it.
  void _showTypingHint() {
    if (_notice == _typingHint && (_noticeTimer?.isActive ?? false)) return;
    _showNotice(_typingHint, ttl: const Duration(milliseconds: 2500));
  }

  /// Transient strip notice, cleared after [ttl].
  void _showNotice(String text, {Duration ttl = const Duration(seconds: 5)}) {
    _notice = text;
    _noticeTimer?.cancel();
    _noticeTimer = Timer(ttl, () {
      _notice = null;
      if (mounted) setState(() {});
    });
    setState(() {});
  }

  /// Clean quit: stop the stream, kill attach clients (tmux sessions keep
  /// running), report invisible (bounded — a hung daemon must not block the
  /// exit), restore the tty (shutdownTui), exit.
  void _quit() {
    sse.stop();
    registry.disposeAll();
    unawaited(heartbeat
        .stop()
        .timeout(const Duration(seconds: 1), onTimeout: () {})
        .whenComplete(() {
      client.close();
      shutdownTui();
    }));
  }

  // ── Render ────────────────────────────────────────────────────────────

  @override
  Component build(BuildContext context) {
    final state = store.state;
    final nowMs = DateTime.now().millisecondsSinceEpoch;

    final main = Column(children: [
      Expanded(
        child: Row(children: [
          Rail(state: state, nowMs: nowMs, onTarget: _onRailTarget),
          Expanded(child: _grid(state, nowMs)),
        ]),
      ),
      Strip(state: state, notice: _notice, onBadgeClick: _onBadgeClick),
    ]);

    final overlay = switch (state.overlay) {
      OverlayKind.help => HelpOverlay(onDismiss: store.closeOverlay),
      OverlayKind.triageQueue => TriageOverlay(
          rows: triageQueueRows(state.sessions),
          selectedIndex: _queueSelection,
          nowMs: nowMs,
          onRowClick: _onQueueRowClick,
          onDismiss: store.closeOverlay,
        ),
      OverlayKind.workspaceAdd => WorkspaceAddOverlay(
          onSubmit: (path) => unawaited(_submitWorkspaceAdd(path)),
          onDismiss: _cancelWorkspaceAdd,
          error: _workspaceAddError,
          busy: _workspaceAddBusy,
        ),
      null => null,
    };

    return Focusable(
      // The root owns keys on the garage and overlay layers; while engaged,
      // the engaged tile's TerminalXterm Focusable owns them instead — and
      // while the add-workspace overlay is open, its TextField does.
      focused: state.layer != KeyLayer.engaged &&
          state.overlay != OverlayKind.workspaceAdd,
      onKeyEvent: _onGarageKey,
      child: overlay == null
          ? main
          : Stack(children: [
              Positioned(left: 0, top: 0, right: 0, bottom: 0, child: main),
              Positioned(
                  left: 0, top: 0, right: 0, bottom: 0, child: overlay),
            ]),
    );
  }

  /// Empty states (spec tui-wall "Empty states" — never amber): zero
  /// workspaces gets the onboarding panel, an empty workspace gets the
  /// spawn hint.
  static Component _emptyState(List<String> lines) => Center(
        child: Column(
          mainAxisSize: MainAxisSize.min,
          children: [
            Text(lines.first, style: const TextStyle(color: GarageColors.fg)),
            for (final line in lines.skip(1))
              Text(line, style: const TextStyle(color: GarageColors.faint)),
          ],
        ),
      );

  Component _grid(WallState state, int nowMs) {
    if (state.groups.isEmpty) {
      return _emptyState(const [
        'no workspaces yet — press w to add one',
        'every Claude session in it appears here live',
      ]);
    }
    final ids = state.griddedSessionIds;
    if (ids.isEmpty) {
      return _emptyState(const [
        'no sessions in this workspace',
        'press n for a session, N for a worktree session',
      ]);
    }
    // ceil(sqrt(n)) columns, row-major (spec tui-wall "Grid layout").
    // Rendered as keyed Positioned tiles in one Stack — not nested
    // Row/Column — so a tile's element (and its terminal buffer) survives
    // reorders and reshapes; only its rect changes. Slot-swapping elements
    // in place is what poisoned controllers before vendored patch 5.
    //
    // Maximize (spec tui-wall "Maximized tile"): the maximized tile is
    // positioned over the FULL grid area and painted last (topmost); its
    // siblings keep their normal rects underneath, mounted (their terminal
    // buffers must survive the round-trip) but `obscured` (no WheelRegion,
    // so wheel events can't land on an invisible tile — Stack hit-testing
    // is topmost-first). The tile's PTY follows the bigger rect via
    // vendored patch 8, which is half the point of maximizing.
    final maximizedId =
        ids.contains(state.maximizedSessionId) ? state.maximizedSessionId : null;
    return LayoutBuilder(builder: (context, constraints) {
      final width = constraints.maxWidth.floor();
      final height = constraints.maxHeight.floor();
      // One region for the whole grid area; the click handler re-derives
      // the tile from the same gridCellRect math that positioned it.
      return ClickRegion(
        onClick: (col, row) => _onGridClick(col, row, width, height),
        child: Stack(children: [
          for (var i = 0; i < ids.length; i++)
            if (ids[i] != maximizedId)
              _positionedTile(state, ids, i, width, height, nowMs,
                  obscured: maximizedId != null),
          if (maximizedId != null)
            Positioned(
              key: ValueKey(maximizedId),
              left: 0,
              top: 0,
              width: width.toDouble(),
              height: height.toDouble(),
              child: _tile(state, maximizedId, nowMs),
            ),
        ]),
      );
    });
  }

  Component _positionedTile(WallState state, List<String> ids, int index,
      int width, int height, int nowMs,
      {bool obscured = false}) {
    final rect = gridCellRect(index, ids.length, width, height);
    return Positioned(
      key: ValueKey(ids[index]),
      left: rect.x.toDouble(),
      top: rect.y.toDouble(),
      width: rect.width.toDouble(),
      height: rect.height.toDouble(),
      child: _tile(state, ids[index], nowMs, obscured: obscured),
    );
  }

  Component _tile(WallState state, String id, int nowMs,
      {bool obscured = false}) {
    final session = state.sessionById(id);
    if (session == null) return Container(); // refetch race; next frame fixes
    final focused = state.focusedSessionId == id;
    return Tile(
      // Keyed by session id so tiles move with their session across grid
      // reorders instead of swapping controllers in place — the vendored
      // TerminalXterm never unregisters its output callback, so an in-place
      // controller swap leaves a stale callback that throws (setState on a
      // defunct element) and starves every later-registered callback,
      // blanking the tile.
      key: ValueKey(id),
      session: session,
      controller: registry.controllerFor(id),
      writer: registry.writerFor(id),
      seedText: registry.seedFor(id),
      deadReason: registry.deadReasonFor(id),
      pasteForwarder: pasteForwarder,
      focused: focused,
      engaged: focused && state.layer == KeyLayer.engaged,
      restoring: _restoringIds.contains(id),
      obscured: obscured,
      nowMs: nowMs,
      onDisengage: store.disengage,
    );
  }
}
