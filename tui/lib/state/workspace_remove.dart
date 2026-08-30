/// p8.4 kill-all removal confirm (spec tui-key-routing "p8.4 kill-all
/// removal confirm"): while the `X` remove arm is active, `K` confirms the
/// removal WITH session kill (`DELETE /api/workspaces/<name>?sessions=kill`).
///
/// The ArmedAction machine stays generic — `K` is a caller-level branch on
/// the armed state, expressed here as pure helpers so the wording and the
/// confirm decision unit-test without a live TUI:
///  - [confirmKillTarget]: does this `K` press confirm, and for which
///    workspace? (Only a live, unexpired `X` arm confirms — and p8.3 only
///    ever arms REGISTERED workspaces, so an unregistered group can never
///    reach a `K` confirm.)
///  - [removeArmNotice]: the arm strip text, with the `K` clause appended
///    only when the workspace actually has live sessions to kill.
///  - [killRemoveNotice]: the post-confirm strip text from the daemon's
///    `{removed, killedSessions, failedSessions?}` response.
library;

import 'armed_action.dart';

/// Caller-level `K` branch on the `X` remove arm: returns the armed
/// workspace name when the arm is live (consuming the arm — the caller then
/// fires the kill-remove), or null when `K` is an ordinary unbound key
/// (nothing armed, or the 3s window expired — an expired arm is disarmed
/// here so the stale target can't linger).
String? confirmKillTarget(ArmedAction armedRemove, int nowMs) {
  final name = armedRemove.armedId;
  if (name == null) return null;
  if (armedRemove.press(name, nowMs)) return name; // confirmed + consumed
  // press() re-armed an expired window — K must never (re-)arm, undo it.
  armedRemove.disarm();
  return null;
}

/// The `X` arm strip notice. With zero live sessions there is nothing for
/// `K` to kill, so the clause is omitted (p8.3's exact wording).
String removeArmNotice(String name, int liveSessions) {
  final base = 'press X again to remove $name (sessions keep running)';
  if (liveSessions <= 0) return base;
  return '$base · K to also kill its $liveSessions sessions';
}

/// Strip notice for a completed `K` confirm, from the daemon response body
/// (`{removed, killedSessions, failedSessions?}`). Failures are named so a
/// half-dead workspace is never reported as cleanly removed.
String killRemoveNotice(String name, Map<String, Object?>? response) {
  final killed = (response?['killedSessions'] as List?)?.length ?? 0;
  final failed = (response?['failedSessions'] as List?)
          ?.whereType<Map>()
          .map((f) => '${f['id']}')
          .toList() ??
      const <String>[];
  final base = 'removed $name · killed $killed sessions';
  if (failed.isEmpty) return base;
  return '$base · failed to kill: ${failed.join(', ')}';
}
