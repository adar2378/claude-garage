// Salience ordering (tui/lib/state/salience.dart), ported from
// ui/src/lib/groups.js buildGroups(): blocked-first at both levels, stable
// otherwise, synthesized groups for unregistered workspaces. Dart's
// List.sort is not stable, so these tests pin the stable-partition port
// against the JS stable-sort semantics.
import 'package:garage_tui/api/models.dart';
import 'package:garage_tui/state/salience.dart';
import 'package:garage_tui/state/wall_state.dart';
import 'package:test/test.dart';

WorkspaceInfo ws(String name, {String? dir}) =>
    WorkspaceInfo(name: name, dir: dir ?? '/repos/$name');

WallSession session(
  String workspace,
  String label, {
  String status = 'working',
  int? since,
}) =>
    WallSession(
      id: 'garage/$workspace/$label',
      workspace: workspace,
      label: label,
      status: status,
      since: since,
    );

void main() {
  group('buildGroups', () {
    test('registry order is preserved when nothing is blocked', () {
      final groups = buildGroups(
        [ws('alpha'), ws('beta'), ws('gamma')],
        [session('beta', 'x'), session('alpha', 'y')],
      );
      expect(groups.map((g) => g.name), ['alpha', 'beta', 'gamma']);
    });

    test('a blocked workspace bubbles above earlier ones, others unchanged '
        '(spec: Blocked workspace bubbles up)', () {
      final groups = buildGroups(
        [ws('a'), ws('b'), ws('c')],
        [
          session('a', 's1'),
          session('b', 's2', status: 'needs-input'),
          session('c', 's3'),
        ],
      );
      expect(groups.map((g) => g.name), ['b', 'a', 'c']);
    });

    test('two blocked workspaces keep their relative order (stable within '
        'the blocked bucket)', () {
      final groups = buildGroups(
        [ws('a'), ws('b'), ws('c'), ws('d')],
        [
          session('b', 's', status: 'needs-input'),
          session('d', 's', status: 'needs-input'),
        ],
      );
      expect(groups.map((g) => g.name), ['b', 'd', 'a', 'c']);
    });

    test('within a workspace, needs-input sessions sort first, stable '
        'otherwise', () {
      final groups = buildGroups(
        [ws('a')],
        [
          session('a', 'one', status: 'working'),
          session('a', 'two', status: 'needs-input'),
          session('a', 'three', status: 'done'),
          session('a', 'four', status: 'needs-input'),
        ],
      );
      expect(
        groups.single.sessions.map((s) => s.label),
        ['two', 'four', 'one', 'three'],
      );
    });

    test('unblocked sessions keep listing order (stable within the '
        'unblocked bucket)', () {
      final groups = buildGroups(
        [ws('a')],
        [
          session('a', 'one', status: 'done'),
          session('a', 'two', status: 'idle'),
          session('a', 'three', status: 'working'),
        ],
      );
      expect(
        groups.single.sessions.map((s) => s.label),
        ['one', 'two', 'three'],
      );
    });

    test('a session in an unregistered workspace gets a synthesized group '
        '(groups.js: nothing is invisible)', () {
      final groups = buildGroups(
        [ws('a')],
        [
          session('a', 's1'),
          WallSession(
            id: 'garage/stray/s2',
            workspace: 'stray',
            label: 's2',
            dir: '/tmp/stray',
            status: 'working',
          ),
        ],
      );
      expect(groups.map((g) => g.name), ['a', 'stray']);
      final stray = groups.last;
      expect(stray.registered, isFalse);
      expect(stray.dir, '/tmp/stray');
      expect(groups.first.registered, isTrue);
    });

    test('a registered workspace with no sessions still appears', () {
      final groups = buildGroups([ws('empty')], []);
      expect(groups.single.name, 'empty');
      expect(groups.single.sessions, isEmpty);
      expect(groups.single.hasNeedsInput, isFalse);
    });

    test('empty inputs produce no groups', () {
      expect(buildGroups([], []), isEmpty);
    });
  });

  group('jumpTarget', () {
    test('picks the needs-input session with the oldest since across '
        'workspaces', () {
      final target = jumpTarget([
        session('a', 'young', status: 'needs-input', since: 3000),
        session('b', 'old', status: 'needs-input', since: 1000),
        session('a', 'busy', status: 'working', since: 500),
      ]);
      expect(target?.label, 'old');
    });

    test('ignores non-blocked sessions entirely', () {
      final target = jumpTarget([
        session('a', 'w', status: 'working', since: 1),
        session('a', 'd', status: 'done', since: 2),
        session('a', 'r', status: 'restorable'),
      ]);
      expect(target, isNull);
    });

    test('a null since never beats a known one; all-null keeps the first '
        'listed', () {
      expect(
        jumpTarget([
          session('a', 'unknown', status: 'needs-input'),
          session('a', 'known', status: 'needs-input', since: 99),
        ])?.label,
        'known',
      );
      expect(
        jumpTarget([
          session('a', 'first', status: 'needs-input'),
          session('a', 'second', status: 'needs-input'),
        ])?.label,
        'first',
      );
    });
  });
}
