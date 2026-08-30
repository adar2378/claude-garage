// Triage queue model (tui/lib/ui/triage_overlay.dart) and the queue-jump
// store path (WallStore.jumpToSession). Spec: tui-triage "Triage queue
// overlay" — rows sorted by waiting time (longest first, stable), j/k
// selection wraps, Enter jump-and-engages the SELECTED session.
import 'package:garage_tui/api/models.dart';
import 'package:garage_tui/state/store.dart';
import 'package:garage_tui/state/wall_state.dart';
import 'package:garage_tui/ui/triage_overlay.dart';
import 'package:test/test.dart';

WallSession session(
  String workspace,
  String label, {
  String status = 'needs-input',
  int? since,
  String? message,
}) =>
    WallSession(
      id: 'garage/$workspace/$label',
      workspace: workspace,
      label: label,
      status: status,
      since: since,
      message: message,
    );

WorkspaceInfo ws(String name) => WorkspaceInfo(name: name, dir: '/repos/$name');

SessionInfo si(
  String workspace,
  String label, {
  String status = 'working',
  int? since = 1000,
  String? message,
}) =>
    SessionInfo(
      id: 'garage/$workspace/$label',
      workspace: workspace,
      label: label,
      dir: '/repos/$workspace',
      status: status,
      since: since,
      message: message,
      restorable: false,
    );

String id(String workspace, String label) => 'garage/$workspace/$label';

void main() {
  group('triageQueueRows', () {
    test('longest-waiting first (smallest since)', () {
      final rows = triageQueueRows([
        session('a', 's1', since: 500),
        session('b', 's2', since: 100),
        session('c', 's3', since: 300),
      ]);
      expect([for (final r in rows) r.since], [100, 300, 500]);
    });

    test('only needs-input sessions appear', () {
      final rows = triageQueueRows([
        session('a', 'working', status: 'working', since: 1),
        session('a', 'blocked', since: 900),
        session('a', 'done', status: 'done', since: 2),
        session('a', 'restorable', status: 'restorable', since: 3),
      ]);
      expect([for (final r in rows) r.label], ['blocked']);
    });

    test('ties keep listing order (stable)', () {
      final rows = triageQueueRows([
        session('a', 'first', since: 100),
        session('b', 'second', since: 100),
        session('c', 'third', since: 100),
      ]);
      expect([for (final r in rows) r.label], ['first', 'second', 'third']);
    });

    test('null since sorts after every known since, stable among itself', () {
      final rows = triageQueueRows([
        session('a', 'unknown1', since: null),
        session('b', 'old', since: 100),
        session('c', 'unknown2', since: null),
        session('d', 'young', since: 900),
      ]);
      expect([for (final r in rows) r.label],
          ['old', 'young', 'unknown1', 'unknown2']);
    });
  });

  group('wrapSelection', () {
    test('moves down and wraps to top', () {
      expect(wrapSelection(0, 1, 3), 1);
      expect(wrapSelection(1, 1, 3), 2);
      expect(wrapSelection(2, 1, 3), 0);
    });

    test('moves up and wraps to bottom', () {
      expect(wrapSelection(2, -1, 3), 1);
      expect(wrapSelection(0, -1, 3), 2);
    });

    test('degenerate lengths pin to 0', () {
      expect(wrapSelection(0, 1, 0), 0);
      expect(wrapSelection(5, -1, 0), 0);
      expect(wrapSelection(0, 1, 1), 0);
      expect(wrapSelection(0, -1, 1), 0);
    });

    test('out-of-range current is clamped before moving', () {
      // Rows shrank between frames: selection 5 into 3 rows, j → wraps sanely.
      expect(wrapSelection(5, 1, 3), 0);
    });
  });

  group('WallStore.jumpToSession (Enter in the queue)', () {
    WallStore storeWith(List<WorkspaceInfo> workspaces, List<SessionInfo> infos) {
      final store = WallStore()..workspacesFetched(workspaces);
      store.sessionsFetched(infos);
      return store;
    }

    test('jump-target of the selected row: focuses, switches workspace, engages',
        () {
      final store = storeWith(
        [ws('alpha'), ws('beta')],
        [
          si('alpha', 'claude-1'),
          si('beta', 'claude-1', status: 'needs-input', since: 100),
          si('beta', 'claude-2', status: 'needs-input', since: 500),
        ],
      );
      // The queue lists beta/claude-1 (longest) then beta/claude-2; the user
      // selects the SECOND row and presses Enter: overlay closes, then the
      // selected — not the longest-waiting — session is jumped to.
      store.openOverlay(OverlayKind.triageQueue);
      final rows = triageQueueRows(store.state.sessions);
      expect([for (final r in rows) r.id],
          [id('beta', 'claude-1'), id('beta', 'claude-2')]);
      final selected = rows[1].id;

      store.closeOverlay();
      expect(store.jumpToSession(selected), isTrue);
      expect(store.state.focusedWorkspace, 'beta');
      expect(store.state.focusedSessionId, selected);
      expect(store.state.layer, KeyLayer.engaged);
      expect(store.state.griddedSessionIds, contains(selected));
    });

    test('declines a session that is gone or no longer blocked', () {
      final store = storeWith(
        [ws('alpha')],
        [si('alpha', 'claude-1', status: 'working')],
      );
      expect(store.jumpToSession('garage/alpha/claude-1'), isFalse);
      expect(store.jumpToSession('garage/alpha/ghost'), isFalse);
      expect(store.state.layer, KeyLayer.garage);
    });

    test('declines outside the garage layer', () {
      final store = storeWith(
        [ws('alpha')],
        [si('alpha', 'claude-1', status: 'needs-input', since: 100)],
      );
      store.openOverlay(OverlayKind.triageQueue);
      expect(store.jumpToSession(id('alpha', 'claude-1')), isFalse);
      expect(store.state.layer, KeyLayer.overlay);
    });
  });
}
