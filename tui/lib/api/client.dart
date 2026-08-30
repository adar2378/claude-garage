/// HTTP client for the garage daemon (design: "API client" — plain dart:io
/// HttpClient against 127.0.0.1:4747, no auth: the daemon allows no-Origin
/// localhost requests). SSE lives in `sse.dart`; this file is request/response
/// only.
library;

import 'dart:convert';
import 'dart:io';

import 'models.dart';

/// The daemon port, honoring `GARAGE_TUI_PORT` then `GARAGE_PORT` (the same
/// env the daemon itself reads — see daemon/src/index.js) so the TUI can be
/// pointed at a scratch daemon for testing. Defaults to 4747.
int garageDaemonPort() {
  for (final name in const ['GARAGE_TUI_PORT', 'GARAGE_PORT']) {
    final port = int.tryParse(Platform.environment[name] ?? '');
    if (port != null && port > 0 && port <= 65535) return port;
  }
  return 4747;
}

/// Base URL for the daemon (also used by bootstrap.dart's health gate; this
/// lives here so the api layer never imports nocterm transitively).
final String defaultDaemonBaseUrl = 'http://127.0.0.1:${garageDaemonPort()}';

/// A non-2xx daemon response, carrying the daemon's `error` body when present.
class GarageApiException implements Exception {
  const GarageApiException(this.statusCode, this.message);

  final int statusCode;
  final String message;

  @override
  String toString() => 'GarageApiException($statusCode): $message';
}

class GarageClient {
  GarageClient({String? baseUrl, HttpClient? httpClient})
      : baseUrl = baseUrl ?? defaultDaemonBaseUrl,
        _http = httpClient ?? HttpClient();

  final String baseUrl;
  final HttpClient _http;

  Future<List<SessionInfo>> fetchSessions() async {
    final body = await _getJson('/api/sessions');
    return [
      for (final entry in body as List<Object?>)
        SessionInfo.fromJson((entry as Map).cast<String, Object?>()),
    ];
  }

  Future<List<WorkspaceInfo>> fetchWorkspaces() async {
    final body = await _getJson('/api/workspaces');
    return [
      for (final entry in body as List<Object?>)
        WorkspaceInfo.fromJson((entry as Map).cast<String, Object?>()),
    ];
  }

  /// `POST /api/sessions`. With [worktree] the daemon spawns into an isolated
  /// git worktree (same contract as the web UI, `ui/src/lib/api.js`).
  Future<SpawnedSession> spawnSession(
    String workspace,
    String label, {
    bool worktree = false,
  }) async {
    final body = await _postJson('/api/sessions', {
      'workspace': workspace,
      'label': label,
      if (worktree) 'worktree': true,
    });
    return SpawnedSession.fromJson((body as Map).cast<String, Object?>());
  }

  /// Visibility heartbeat (`POST /api/ui/visibility`) so daemon-side macOS
  /// notifications stay suppressed while the TUI is visible — same contract
  /// as the web UI (spec tui-triage "Off-screen escalation").
  Future<void> postVisibility(String clientId, bool visible) async {
    await _postJson('/api/ui/visibility', {
      'clientId': clientId,
      'visible': visible,
    });
  }

  /// `POST /api/sessions/restore {id}` — restore one restorable session
  /// (same per-id contract the web UI uses; restore-all is the caller
  /// issuing parallel per-id calls). Returns the failure reason for this id
  /// when the daemon reports one, null on success.
  Future<String?> restoreSession(String id) async {
    final body = await _postJson('/api/sessions/restore', {'id': id});
    final map = (body as Map).cast<String, Object?>();
    final failed = map['failed'];
    if (failed is List && failed.isNotEmpty) {
      final first = (failed.first as Map).cast<String, Object?>();
      return (first['reason'] as String?) ?? 'restore failed';
    }
    return null;
  }

  /// `DELETE /api/sessions/<id>` — kill a live session (and drop its resume
  /// metadata). With [metaOnly] (`?meta=1`) only the stored resume metadata
  /// of a NON-live (restorable) session is dropped — the daemon returns 404
  /// for a plain DELETE of a session with no live tmux match. Returns the
  /// worktree record from the response (`{path, branch, repoDir}`) or null.
  Future<Map<String, Object?>?> deleteSession(String id,
      {bool metaOnly = false}) async {
    final encoded = Uri.encodeComponent(id);
    final path = '/api/sessions/$encoded${metaOnly ? '?meta=1' : ''}';
    final request = await _http.deleteUrl(Uri.parse('$baseUrl$path'));
    final body = await _readJson(await request.close());
    final worktree = (body as Map?)?['worktree'];
    return worktree is Map ? worktree.cast<String, Object?>() : null;
  }

  /// `PUT /api/workspaces {name, dir}` — register a workspace (same
  /// contract as the web UI's putWorkspace).
  Future<void> putWorkspace(String name, String dir) async {
    final request = await _http.putUrl(Uri.parse('$baseUrl/api/workspaces'));
    request.headers.contentType = ContentType.json;
    request.write(jsonEncode({'name': name, 'dir': dir}));
    await _readJson(await request.close());
  }

  /// `DELETE /api/workspaces/<name>` — registry-only removal by default
  /// (the TUI's `X`-`X` confirm must not touch tmux: live sessions keep
  /// running and reappear in the rail as an unregistered group after the
  /// refetch; the 204 yields null). With [killSessions] (`?sessions=kill`,
  /// the p8.4 `X` then `K` confirm) the daemon kills every live
  /// `garage/<name>/*` tmux session first and returns
  /// `{removed, killedSessions, failedSessions?}` — returned here for the
  /// strip notice.
  Future<Map<String, Object?>?> removeWorkspace(String name,
      {bool killSessions = false}) async {
    final encoded = Uri.encodeComponent(name);
    final path =
        '/api/workspaces/$encoded${killSessions ? '?sessions=kill' : ''}';
    final request = await _http.deleteUrl(Uri.parse('$baseUrl$path'));
    final body = await _readJson(await request.close());
    return body is Map ? body.cast<String, Object?>() : null;
  }

  void close() => _http.close(force: true);

  Future<Object?> _getJson(String path) async {
    final request = await _http.getUrl(Uri.parse('$baseUrl$path'));
    return _readJson(await request.close());
  }

  Future<Object?> _postJson(String path, Map<String, Object?> body) async {
    final request = await _http.postUrl(Uri.parse('$baseUrl$path'));
    request.headers.contentType = ContentType.json;
    request.write(jsonEncode(body));
    return _readJson(await request.close());
  }

  Future<Object?> _readJson(HttpClientResponse response) async {
    final text = await utf8.decodeStream(response);
    if (response.statusCode < 200 || response.statusCode >= 300) {
      String message = text;
      try {
        final decoded = jsonDecode(text);
        if (decoded is Map && decoded['error'] is String) {
          message = decoded['error'] as String;
        }
      } on FormatException {
        // Non-JSON error body — keep the raw text.
      }
      throw GarageApiException(response.statusCode, message);
    }
    return text.isEmpty ? null : jsonDecode(text);
  }
}
