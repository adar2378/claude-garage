/// Typed views over the daemon's JSON payloads.
///
/// Shapes mirror `daemon/src/sessions.js` (GET /api/sessions — live entries
/// plus restorable placeholders, both carrying `status`/`since`/`message`/
/// `branch`) and `daemon/src/workspaces.js` (GET /api/workspaces). The TUI is
/// a thin client: nothing here interprets state, it only types it.
library;

int? _asEpochMs(Object? v) => v is num ? v.toInt() : null;

/// One registered workspace from `GET /api/workspaces`.
class WorkspaceInfo {
  const WorkspaceInfo({required this.name, this.dir, this.branch});

  factory WorkspaceInfo.fromJson(Map<String, Object?> json) => WorkspaceInfo(
        name: json['name'] as String,
        dir: json['dir'] as String?,
        branch: json['branch'] as String?,
      );

  final String name;
  final String? dir;
  final String? branch;
}

/// One session entry from `GET /api/sessions` (live or restorable).
class SessionInfo {
  const SessionInfo({
    required this.id,
    required this.workspace,
    required this.label,
    this.dir,
    this.attached = false,
    required this.status,
    this.since,
    this.message,
    this.branch,
    this.restorable = false,
  });

  factory SessionInfo.fromJson(Map<String, Object?> json) => SessionInfo(
        id: json['id'] as String,
        workspace: json['workspace'] as String,
        label: json['label'] as String,
        dir: json['dir'] as String?,
        attached: json['attached'] == true,
        status: json['status'] as String? ?? 'idle',
        since: _asEpochMs(json['since']),
        message: json['message'] as String?,
        branch: json['branch'] as String?,
        restorable: json['restorable'] == true,
      );

  final String id;
  final String workspace;
  final String label;

  /// The session's starting dir (tmux `session_path`, frozen at creation) —
  /// for a worktree session this is the worktree path, not the registered
  /// workspace dir.
  final String? dir;
  final bool attached;

  /// `needs-input | working | done | idle | restorable`.
  final String status;

  /// Epoch ms the current status began (daemon falls back to tmux creation
  /// time for never-transitioned sessions; null only for restorable entries).
  final int? since;

  /// The Notification hook's text; non-null only while `needs-input`.
  final String? message;
  final String? branch;
  final bool restorable;
}

/// `POST /api/sessions` 201 body.
class SpawnedSession {
  const SpawnedSession({
    required this.id,
    required this.workspace,
    required this.label,
    this.dir,
    this.worktree = false,
  });

  factory SpawnedSession.fromJson(Map<String, Object?> json) => SpawnedSession(
        id: json['id'] as String,
        workspace: json['workspace'] as String,
        label: json['label'] as String,
        dir: json['dir'] as String?,
        worktree: json['worktree'] != null,
      );

  final String id;
  final String workspace;
  final String label;
  final String? dir;

  /// Whether the daemon created an isolated git worktree for this session.
  final bool worktree;
}
