/// Salience ordering, ported from `ui/src/lib/groups.js` buildGroups()
/// semantics (spec tui-triage: "Salience-first ordering"):
///  - workspaces containing >=1 needs-input session sort before the rest,
///    stable otherwise (registration/discovery order preserved within each
///    bucket);
///  - within a workspace, needs-input sessions sort before the rest, stable
///    otherwise;
///  - sessions whose workspace isn't registered still get a synthesized
///    group (registered: false) so nothing is invisible.
///
/// groups.js leans on JS Array#sort being stable; Dart's List.sort is NOT
/// stable, so the two-bucket rank sort is implemented as a stable partition
/// instead — identical semantics for a boolean key.
library;

import '../api/models.dart';
import 'wall_state.dart' show WallSession;

/// One rail group: a workspace (registered or synthesized) with its sessions
/// in salience order.
class WorkspaceGroup {
  WorkspaceGroup({
    required this.name,
    this.dir,
    this.branch,
    required this.registered,
    required List<WallSession> sessions,
  }) : sessions = List.unmodifiable(sessions);

  final String name;
  final String? dir;
  final String? branch;
  final bool registered;
  final List<WallSession> sessions;

  bool get hasNeedsInput => sessions.any((s) => s.needsInput);
}

/// Stable two-bucket sort: everything matching [first] in original order,
/// then the rest in original order.
List<T> _stablePartition<T>(Iterable<T> items, bool Function(T) first) => [
      ...items.where(first),
      ...items.where((item) => !first(item)),
    ];

List<WorkspaceGroup> buildGroups(
  List<WorkspaceInfo> workspaces,
  List<WallSession> sessions,
) {
  // Insertion-ordered, like groups.js's Map: registered workspaces first in
  // registry order, synthesized groups after in discovery order.
  final byName = <String, ({WorkspaceInfo? ws, List<WallSession> sessions})>{};

  for (final ws in workspaces) {
    byName[ws.name] = (ws: ws, sessions: []);
  }
  for (final s in sessions) {
    final group = byName.putIfAbsent(s.workspace, () => (ws: null, sessions: []));
    group.sessions.add(s);
  }

  final groups = [
    for (final entry in byName.entries)
      WorkspaceGroup(
        name: entry.key,
        dir: entry.value.ws?.dir ?? entry.value.sessions.firstOrNull?.dir,
        branch: entry.value.ws?.branch,
        registered: entry.value.ws != null,
        sessions: _stablePartition(entry.value.sessions, (s) => s.needsInput),
      ),
  ];

  return _stablePartition(groups, (g) => g.hasNeedsInput);
}

/// The `R` restore-all selection (spec tui-key-routing "p8.1 session
/// lifecycle bindings"): every restorable session in [workspace], listing
/// order preserved. The caller issues one `POST /api/sessions/restore {id}`
/// per id in parallel — same shape as the web UI's rail restore-all, so one
/// failure never blocks the rest.
List<String> restorableSessionIds(
        Iterable<WallSession> sessions, String workspace) =>
    [
      for (final s in sessions)
        if (s.workspace == workspace && !s.live) s.id,
    ];

/// The `a`-jump target (spec tui-triage: "The `a` jump lands engaged"): the
/// longest-waiting needs-input session across all workspaces — oldest
/// `since`. Null when nothing is blocked. A null `since` never beats a known
/// one; ties keep the earliest-listed session (stable).
WallSession? jumpTarget(List<WallSession> sessions) {
  WallSession? best;
  for (final s in sessions) {
    if (!s.needsInput) continue;
    if (best == null) {
      best = s;
      continue;
    }
    final candidate = s.since;
    final incumbent = best.since;
    if (candidate != null && (incumbent == null || candidate < incumbent)) {
      best = s;
    }
  }
  return best;
}
