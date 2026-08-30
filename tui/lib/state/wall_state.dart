/// Immutable wall state (design: "State architecture" — one immutable
/// WallState; every render is a pure function of it plus live terminal
/// buffers). Mutation happens only in `store.dart`.
library;

import '../api/models.dart';
import 'salience.dart';

/// Exactly one layer governs keyboard input at a time
/// (spec tui-key-routing: "Three key layers with a visible target chip").
enum KeyLayer { garage, engaged, overlay }

/// Which overlay is open while [KeyLayer.overlay] is active. Overlays never
/// stack: at most one is open, tracked by a single slot.
enum OverlayKind { help, triageQueue, workspaceAdd }

/// One session as the wall tracks it. Mirrors the daemon entry plus the
/// derived [worktree] flag (the listing does not expose the worktree record;
/// like the web UI, a session whose starting dir differs from its
/// workspace's registered dir was spawned into a worktree).
class WallSession {
  const WallSession({
    required this.id,
    required this.workspace,
    required this.label,
    this.dir,
    required this.status,
    this.since,
    this.message,
    this.branch,
    this.worktree = false,
  });

  factory WallSession.fromInfo(SessionInfo info, {String? workspaceDir}) =>
      WallSession(
        id: info.id,
        workspace: info.workspace,
        label: info.label,
        dir: info.dir,
        status: info.status,
        since: info.since,
        message: info.message,
        branch: info.branch,
        worktree: !info.restorable &&
            info.dir != null &&
            workspaceDir != null &&
            info.dir != workspaceDir,
      );

  final String id;
  final String workspace;
  final String label;
  final String? dir;

  /// `needs-input | working | done | idle | restorable`.
  final String status;

  /// Epoch ms the current status began.
  final int? since;

  /// Notification text; non-null only while needs-input.
  final String? message;
  final String? branch;
  final bool worktree;

  bool get needsInput => status == 'needs-input';

  /// A live session has a tmux session behind it — anything but restorable.
  bool get live => status != 'restorable';

  WallSession copyWith({String? status, int? since, String? message}) =>
      WallSession(
        id: id,
        workspace: workspace,
        label: label,
        dir: dir,
        status: status ?? this.status,
        // since/message are replaced, not merged: a status transition always
        // carries its own values (null clears).
        since: since,
        message: message,
        branch: branch,
        worktree: worktree,
      );
}

class WallState {
  WallState({
    required List<WorkspaceInfo> workspaces,
    required List<WallSession> sessions,
    required List<WorkspaceGroup> groups,
    this.focusedWorkspace,
    this.focusedSessionId,
    this.layer = KeyLayer.garage,
    this.overlay,
    this.maximizedSessionId,
    List<String> griddedSessionIds = const [],
    List<String> gridFocusRecency = const [],
  })  : workspaces = List.unmodifiable(workspaces),
        sessions = List.unmodifiable(sessions),
        groups = List.unmodifiable(groups),
        griddedSessionIds = List.unmodifiable(griddedSessionIds),
        gridFocusRecency = List.unmodifiable(gridFocusRecency);

  factory WallState.initial() =>
      WallState(workspaces: const [], sessions: const [], groups: const []);

  /// At most this many tiles render at once (spec tui-wall: six-tile cap).
  static const int gridCap = 6;

  /// Raw registry order, as fetched.
  final List<WorkspaceInfo> workspaces;

  /// All sessions, as fetched (live + restorable).
  final List<WallSession> sessions;

  /// Salience-ordered groups (see salience.dart). This IS the rail render
  /// order and the order the `1`–`9` bindings index into.
  final List<WorkspaceGroup> groups;

  /// Focused workspace by name — name, not index, so salience reorders never
  /// silently move the focus.
  final String? focusedWorkspace;
  final String? focusedSessionId;
  final KeyLayer layer;

  /// Non-null exactly while [layer] == [KeyLayer.overlay].
  final OverlayKind? overlay;

  /// The tile taking the full grid area (`m` toggle, spec tui-wall
  /// "Maximized tile"). Always one of [griddedSessionIds]; cleared whenever
  /// focus moves to a different session, and by reconciliation when the
  /// session leaves the grid.
  final String? maximizedSessionId;

  /// Session ids in the grid for the focused workspace, display order,
  /// length <= [gridCap]. Slot-stable: swap-ins replace the evicted tile's
  /// slot instead of reshuffling the grid.
  final List<String> griddedSessionIds;

  /// Focus recency for LRU eviction — least-recently-focused first.
  final List<String> gridFocusRecency;

  WallSession? sessionById(String? id) {
    if (id == null) return null;
    for (final s in sessions) {
      if (s.id == id) return s;
    }
    return null;
  }

  WorkspaceGroup? groupByName(String? name) {
    if (name == null) return null;
    for (final g in groups) {
      if (g.name == name) return g;
    }
    return null;
  }

  /// Count of needs-input sessions across all workspaces (strip badge).
  int get blockedCount => sessions.where((s) => s.needsInput).length;

  /// The bottom strip's keys-target chip
  /// (spec: `keys → garage` / `keys → <workspace>/<label>`).
  String get keysTargetChip {
    switch (layer) {
      case KeyLayer.garage:
        return 'keys → garage';
      case KeyLayer.engaged:
        final s = sessionById(focusedSessionId);
        return s == null ? 'keys → garage' : 'keys → ${s.workspace}/${s.label}';
      case KeyLayer.overlay:
        return switch (overlay) {
          OverlayKind.triageQueue => 'keys → queue',
          OverlayKind.workspaceAdd => 'keys → add workspace',
          _ => 'keys → help',
        };
    }
  }

  WallState copyWith({
    List<WorkspaceInfo>? workspaces,
    List<WallSession>? sessions,
    List<WorkspaceGroup>? groups,
    String? focusedWorkspace,
    bool clearFocusedWorkspace = false,
    String? focusedSessionId,
    bool clearFocusedSessionId = false,
    KeyLayer? layer,
    OverlayKind? overlay,
    bool clearOverlay = false,
    String? maximizedSessionId,
    bool clearMaximized = false,
    List<String>? griddedSessionIds,
    List<String>? gridFocusRecency,
  }) =>
      WallState(
        workspaces: workspaces ?? this.workspaces,
        sessions: sessions ?? this.sessions,
        groups: groups ?? this.groups,
        focusedWorkspace: clearFocusedWorkspace
            ? null
            : (focusedWorkspace ?? this.focusedWorkspace),
        focusedSessionId: clearFocusedSessionId
            ? null
            : (focusedSessionId ?? this.focusedSessionId),
        layer: layer ?? this.layer,
        overlay: clearOverlay ? null : (overlay ?? this.overlay),
        maximizedSessionId: clearMaximized
            ? null
            : (maximizedSessionId ?? this.maximizedSessionId),
        griddedSessionIds: griddedSessionIds ?? this.griddedSessionIds,
        gridFocusRecency: gridFocusRecency ?? this.gridFocusRecency,
      );
}
