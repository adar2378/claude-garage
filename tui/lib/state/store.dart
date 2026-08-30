/// The wall store: the only mutable slot in the TUI. It applies events
/// (SSE, fetches, key commands) to the immutable [WallState] — rendering and
/// PTY wiring subscribe via [onChange] and are pure consumers.
library;

import '../api/models.dart';
import 'salience.dart';
import 'wall_state.dart';

/// Garage-layer commands (spec tui-key-routing: "Garage-layer bindings" +
/// "p8.1 session lifecycle bindings" + "p8.3 workspace removal",
/// `1-9 [ ] a A n N m R x X w Enter ? q`).
/// State-affecting commands are applied by [WallStore.dispatch];
/// [SpawnCommand], [RestoreAllCommand], [CloseCommand],
/// [WorkspaceRemoveCommand] and [QuitCommand]
/// are effects the caller performs (daemon API calls, process exit) — the
/// store never does IO. [EngageCommand] is declined when the focused tile
/// is a restorable placeholder, so the caller can run the restore effect.
sealed class GarageCommand {
  const GarageCommand();
}

class FocusWorkspaceCommand extends GarageCommand {
  const FocusWorkspaceCommand(this.index);

  /// Zero-based index into [WallState.groups] (`1` → 0).
  final int index;
}

class CycleFocusCommand extends GarageCommand {
  const CycleFocusCommand(this.delta);

  /// `]` → +1, `[` → -1, cycling the focused tile through the grid.
  final int delta;
}

class JumpCommand extends GarageCommand {
  const JumpCommand();
}

class OpenQueueCommand extends GarageCommand {
  const OpenQueueCommand();
}

class SpawnCommand extends GarageCommand {
  const SpawnCommand({required this.worktree});

  final bool worktree;
}

class EngageCommand extends GarageCommand {
  const EngageCommand();
}

/// `m`: toggle the focused tile full-grid (spec tui-wall "Maximized tile").
class MaximizeCommand extends GarageCommand {
  const MaximizeCommand();
}

/// `R`: restore every restorable session in the focused workspace — an
/// effect (parallel per-id `POST /api/sessions/restore` calls, like the web
/// UI's restore-all), so the store always declines it.
class RestoreAllCommand extends GarageCommand {
  const RestoreAllCommand();
}

/// `x`: armed double-press close of the focused session — an effect
/// (`DELETE /api/sessions/<id>`), so the store always declines it.
class CloseCommand extends GarageCommand {
  const CloseCommand();
}

/// `w`: open the add-workspace overlay.
class WorkspaceAddCommand extends GarageCommand {
  const WorkspaceAddCommand();
}

/// `X`: armed double-press removal of the FOCUSED workspace's registration
/// — an effect (`DELETE /api/workspaces/<name>`, registry-only: live
/// sessions keep running and reappear as an unregistered group), so the
/// store always declines it.
class WorkspaceRemoveCommand extends GarageCommand {
  const WorkspaceRemoveCommand();
}

class ToggleHelpCommand extends GarageCommand {
  const ToggleHelpCommand();
}

class QuitCommand extends GarageCommand {
  const QuitCommand();
}

/// Maps a garage-layer key to its command; null for unbound keys (which are
/// consumed silently — garage typing never reaches an agent). Enter arrives
/// as `'\n'` or `'\r'` depending on the host terminal.
GarageCommand? garageCommandFor(String key) {
  if (key.length == 1) {
    final code = key.codeUnitAt(0);
    if (code >= 0x31 && code <= 0x39) {
      return FocusWorkspaceCommand(code - 0x31); // '1'..'9'
    }
  }
  switch (key) {
    case '[':
      return const CycleFocusCommand(-1);
    case ']':
      return const CycleFocusCommand(1);
    case 'a':
      return const JumpCommand();
    case 'A':
      return const OpenQueueCommand();
    case 'n':
      return const SpawnCommand(worktree: false);
    case 'N':
      return const SpawnCommand(worktree: true);
    case 'm':
      return const MaximizeCommand();
    case 'R':
      return const RestoreAllCommand();
    case 'x':
      return const CloseCommand();
    case 'w':
      return const WorkspaceAddCommand();
    case 'X':
      return const WorkspaceRemoveCommand();
    case '\n':
    case '\r':
      return const EngageCommand();
    case '?':
      return const ToggleHelpCommand();
    case 'q':
      return const QuitCommand();
  }
  return null;
}

class WallStore {
  WallState _state = WallState.initial();

  WallState get state => _state;

  /// Called after every state change; render wiring hangs off this.
  void Function(WallState state)? onChange;

  void _set(WallState next) {
    _state = next;
    onChange?.call(next);
  }

  // ── Fetch / SSE events ────────────────────────────────────────────────

  void workspacesFetched(List<WorkspaceInfo> workspaces) {
    // Re-derive worktree flags: the flag compares a session's dir against
    // its workspace's registered dir, which may just have changed.
    final dirs = {for (final w in workspaces) w.name: w.dir};
    final sessions = [
      for (final s in _state.sessions)
        WallSession(
          id: s.id,
          workspace: s.workspace,
          label: s.label,
          dir: s.dir,
          status: s.status,
          since: s.since,
          message: s.message,
          branch: s.branch,
          worktree: s.live &&
              s.dir != null &&
              dirs[s.workspace] != null &&
              s.dir != dirs[s.workspace],
        ),
    ];
    _set(_reconcile(_state.copyWith(workspaces: workspaces), sessions));
  }

  void sessionsFetched(List<SessionInfo> infos) {
    final dirs = {for (final w in _state.workspaces) w.name: w.dir};
    final sessions = [
      for (final info in infos)
        WallSession.fromInfo(info, workspaceDir: dirs[info.workspace]),
    ];
    _set(_reconcile(_state, sessions));
  }

  /// SSE `status` event `{id, status, since}`. The event carries no message;
  /// the daemon clears the message on any transition away from needs-input,
  /// so mirror that here — a needs-input transition's message text arrives
  /// with the next sessions refetch.
  void statusChanged(String id, String status, int? since) {
    if (!_state.sessions.any((s) => s.id == id)) {
      return; // Unknown id — the sessions refetch will bring it.
    }
    final sessions = [
      for (final s in _state.sessions)
        if (s.id == id)
          s.copyWith(
            status: status,
            since: since,
            message: status == 'needs-input' ? s.message : null,
          )
        else
          s,
    ];
    _set(_reconcile(_state, sessions));
  }

  // ── Focus ─────────────────────────────────────────────────────────────

  /// Focus the [index]-th group in salience order (the `1`–`9` bindings).
  void focusWorkspace(int index) {
    if (index < 0 || index >= _state.groups.length) return;
    _set(_withFocusedWorkspace(_state, _state.groups[index].name));
  }

  /// Focus a workspace by name (the add-workspace flow lands on the group
  /// it just created — an index would race the salience reorder).
  void focusWorkspaceNamed(String name) {
    if (_state.groupByName(name) == null) return;
    _set(_withFocusedWorkspace(_state, name));
  }

  /// Focus a session by id, switching workspace and swapping the session
  /// into the grid (LRU eviction) when needed.
  void focusSession(String id) {
    final next = _focusSession(_state, id);
    if (next != null) _set(next);
  }

  /// Cycle the focused tile through the grid (`[` / `]`).
  void cycleFocus(int delta) {
    final grid = _state.griddedSessionIds;
    if (grid.isEmpty) return;
    final current = grid.indexOf(_state.focusedSessionId ?? '');
    final from = current < 0 ? 0 : current;
    final next = (from + delta) % grid.length;
    focusSession(grid[next < 0 ? next + grid.length : next]);
  }

  /// Toggle the focused tile full-grid (`m`, spec tui-wall "Maximized
  /// tile"). Only a gridded tile can maximize; toggling the maximized tile
  /// (or focusing another session — see [_clearStaleMaximize]) restores the
  /// normal grid.
  void toggleMaximize() {
    if (_state.layer != KeyLayer.garage) return;
    final id = _state.focusedSessionId;
    if (id == null) return;
    if (_state.maximizedSessionId == id) {
      _set(_state.copyWith(clearMaximized: true));
    } else if (_state.griddedSessionIds.contains(id)) {
      _set(_state.copyWith(maximizedSessionId: id));
    }
  }

  /// Invariant: a maximized tile is always the focused one. Any transition
  /// that lands focus elsewhere (focus keys, rail clicks, reconciliation
  /// after the session died) exits maximize.
  WallState _clearStaleMaximize(WallState s) =>
      s.maximizedSessionId != null && s.maximizedSessionId != s.focusedSessionId
          ? s.copyWith(clearMaximized: true)
          : s;

  // ── Layers ────────────────────────────────────────────────────────────

  /// Engage the focused tile. Requires the garage layer and a focused live
  /// session — a restorable placeholder (or no focus) cannot be engaged.
  /// Returns whether the engage happened, so the caller can offer the
  /// restore effect for a declined restorable tile.
  bool engage() {
    if (_state.layer != KeyLayer.garage) return false;
    final s = _state.sessionById(_state.focusedSessionId);
    if (s == null || !s.live) return false;
    _set(_state.copyWith(layer: KeyLayer.engaged));
    return true;
  }

  /// Return from engaged to garage (the Ctrl+G/Ctrl+Q chord).
  void disengage() {
    if (_state.layer != KeyLayer.engaged) return;
    _set(_state.copyWith(layer: KeyLayer.garage));
  }

  /// Open an overlay from the garage layer. Overlays never stack: while one
  /// is open (or a tile is engaged) this is a no-op.
  void openOverlay(OverlayKind kind) {
    if (_state.layer != KeyLayer.garage) return;
    _set(_state.copyWith(layer: KeyLayer.overlay, overlay: kind));
  }

  void closeOverlay() {
    if (_state.layer != KeyLayer.overlay) return;
    _set(_state.copyWith(layer: KeyLayer.garage, clearOverlay: true));
  }

  // ── Triage ────────────────────────────────────────────────────────────

  /// The `a` jump (spec tui-triage): focus the longest-waiting needs-input
  /// session across all workspaces — switching workspace, swapping the tile
  /// in if it was overflow — and land engaged. Returns false (state
  /// untouched) when nothing is blocked; the caller renders the strip notice.
  bool jumpToLongestWaiting() {
    if (_state.layer != KeyLayer.garage) return false;
    final target = jumpTarget(_state.sessions);
    if (target == null) return false;
    final focused = _focusSession(_state, target.id);
    if (focused == null) return false;
    // needs-input implies live, so the engage precondition holds.
    _set(focused.copyWith(layer: KeyLayer.engaged));
    return true;
  }

  /// Jump-and-engage a specific blocked session (Enter in the triage queue,
  /// spec tui-triage: "Jump from queue lands engaged") — same landing as
  /// [jumpToLongestWaiting] but for a chosen id. Returns false (state
  /// untouched) when the session is gone or no longer needs-input, so a
  /// stale queue row can never engage the wrong tile.
  bool jumpToSession(String id) {
    if (_state.layer != KeyLayer.garage) return false;
    final target = _state.sessionById(id);
    if (target == null || !target.needsInput) return false;
    final focused = _focusSession(_state, id);
    if (focused == null) return false;
    // needs-input implies live, so the engage precondition holds.
    _set(focused.copyWith(layer: KeyLayer.engaged));
    return true;
  }

  // ── Command dispatch ──────────────────────────────────────────────────

  /// Apply a garage-layer command to the state. Returns true when the store
  /// handled it; [SpawnCommand], [RestoreAllCommand], [CloseCommand],
  /// [WorkspaceRemoveCommand] and [QuitCommand] always return false — they
  /// are the caller's effects.
  /// [EngageCommand] returns false when the focused tile cannot engage (a
  /// restorable placeholder), so the caller can run the restore effect.
  /// [ToggleHelpCommand] also closes an open help overlay, so `?` toggles.
  bool dispatch(GarageCommand command) {
    switch (command) {
      case ToggleHelpCommand():
        if (_state.layer == KeyLayer.overlay) {
          if (_state.overlay != OverlayKind.help) return false;
          closeOverlay();
          return true;
        }
        if (_state.layer != KeyLayer.garage) return false;
        openOverlay(OverlayKind.help);
        return true;
      case SpawnCommand() ||
            RestoreAllCommand() ||
            CloseCommand() ||
            WorkspaceRemoveCommand() ||
            QuitCommand():
        return false;
      case _ when _state.layer != KeyLayer.garage:
        return false;
      case FocusWorkspaceCommand(:final index):
        focusWorkspace(index);
        return true;
      case CycleFocusCommand(:final delta):
        cycleFocus(delta);
        return true;
      case JumpCommand():
        return jumpToLongestWaiting();
      case OpenQueueCommand():
        openOverlay(OverlayKind.triageQueue);
        return true;
      case MaximizeCommand():
        toggleMaximize();
        return true;
      case WorkspaceAddCommand():
        openOverlay(OverlayKind.workspaceAdd);
        return true;
      case EngageCommand():
        return engage();
    }
  }

  // ── Internals ─────────────────────────────────────────────────────────

  /// Rebuild groups from [sessions] and reconcile every derived slot:
  /// focused workspace still exists (else first group), grid pruned/refilled,
  /// focused session still valid, engagement dropped if its session died.
  WallState _reconcile(WallState base, List<WallSession> sessions) {
    final groups = buildGroups(base.workspaces, sessions);
    // Engagement is pinned to a specific session: if reconciliation moves or
    // clears the focus (the engaged session died), drop back to garage —
    // never silently transfer engagement to another tile.
    final engagedId =
        base.layer == KeyLayer.engaged ? base.focusedSessionId : null;
    var next = base.copyWith(sessions: sessions, groups: groups);

    final focusedGroup = next.groupByName(next.focusedWorkspace);
    if (focusedGroup == null) {
      return _withFocusedWorkspace(
        // The engaged session's workspace vanished with it.
        next.layer == KeyLayer.engaged
            ? next.copyWith(layer: KeyLayer.garage)
            : next,
        groups.isEmpty ? null : groups.first.name,
      );
    }

    final ids = {for (final s in focusedGroup.sessions) s.id};
    final grid = [
      for (final id in next.griddedSessionIds)
        if (ids.contains(id)) id,
    ];
    for (final s in focusedGroup.sessions) {
      if (grid.length >= WallState.gridCap) break;
      if (!grid.contains(s.id)) grid.add(s.id);
    }
    final recency = [
      for (final id in next.gridFocusRecency)
        if (grid.contains(id)) id,
      for (final id in grid)
        if (!next.gridFocusRecency.contains(id)) id,
    ];

    var focusedId = next.focusedSessionId;
    if (focusedId == null || !ids.contains(focusedId)) {
      focusedId = grid.isEmpty ? null : grid.first;
    }
    next = next.copyWith(
      griddedSessionIds: grid,
      gridFocusRecency: recency,
      focusedSessionId: focusedId,
      clearFocusedSessionId: focusedId == null,
    );

    // Engagement survives only while its own session is still focused and
    // live.
    if (next.layer == KeyLayer.engaged) {
      final s = next.sessionById(next.focusedSessionId);
      if (s == null || !s.live || s.id != engagedId) {
        next = next.copyWith(layer: KeyLayer.garage);
      }
    }
    return _clearStaleMaximize(next);
  }

  /// Switch the focused workspace: rebuild the grid as the group's first
  /// [WallState.gridCap] sessions in salience order, focus the first tile.
  WallState _withFocusedWorkspace(WallState base, String? name) {
    final group = base.groupByName(name);
    final grid = group == null
        ? const <String>[]
        : [for (final s in group.sessions.take(WallState.gridCap)) s.id];
    return _clearStaleMaximize(base.copyWith(
      focusedWorkspace: name,
      clearFocusedWorkspace: name == null,
      focusedSessionId: grid.isEmpty ? null : grid.first,
      clearFocusedSessionId: grid.isEmpty,
      griddedSessionIds: grid,
      gridFocusRecency: grid,
    ));
  }

  /// Focus [id], switching workspace and swapping into the grid if needed.
  /// Returns null when the session doesn't exist.
  WallState? _focusSession(WallState base, String id) {
    final session = base.sessionById(id);
    if (session == null) return null;

    var next = base;
    if (session.workspace != next.focusedWorkspace) {
      next = _withFocusedWorkspace(next, session.workspace);
    }

    final grid = [...next.griddedSessionIds];
    final recency = [...next.gridFocusRecency];
    if (!grid.contains(id)) {
      if (grid.length < WallState.gridCap) {
        grid.add(id);
      } else {
        // LRU swap-in: the least-recently-focused tile leaves; the new tile
        // takes its slot so the other five don't reshuffle.
        final lru = recency.firstWhere(grid.contains, orElse: () => grid.first);
        grid[grid.indexOf(lru)] = id;
        recency.remove(lru);
      }
    }
    recency
      ..remove(id)
      ..add(id);
    return _clearStaleMaximize(next.copyWith(
      focusedSessionId: id,
      griddedSessionIds: grid,
      gridFocusRecency: recency,
    ));
  }
}
