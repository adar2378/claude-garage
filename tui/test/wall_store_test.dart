// WallStore / WallState (tui/lib/state/): layer transitions, the 6-cap grid
// with LRU swap-in, the a-jump selector, garage command mapping, and
// SSE-event application. Specs: tui-key-routing (layers, garage bindings),
// tui-wall (grid cap), tui-triage (a jump).
import 'package:garage_tui/api/models.dart';
import 'package:garage_tui/state/salience.dart' show restorableSessionIds;
import 'package:garage_tui/state/store.dart';
import 'package:garage_tui/state/wall_state.dart';
import 'package:test/test.dart';

WorkspaceInfo ws(String name) => WorkspaceInfo(name: name, dir: '/repos/$name');

SessionInfo si(
  String workspace,
  String label, {
  String status = 'working',
  int? since = 1000,
  String? message,
  String? dir,
  bool restorable = false,
}) =>
    SessionInfo(
      id: 'garage/$workspace/$label',
      workspace: workspace,
      label: label,
      dir: dir ?? '/repos/$workspace',
      status: status,
      since: since,
      message: message,
      restorable: restorable,
    );

String id(String workspace, String label) => 'garage/$workspace/$label';

WallStore storeWith(List<WorkspaceInfo> workspaces, List<SessionInfo> sessions) {
  final store = WallStore()..workspacesFetched(workspaces);
  store.sessionsFetched(sessions);
  return store;
}

void main() {
  group('fetch reconciliation', () {
    test('first fetch focuses the first group and grids its sessions', () {
      final store = storeWith(
        [ws('a'), ws('b')],
        [si('a', 'one'), si('a', 'two'), si('b', 'other')],
      );
      expect(store.state.focusedWorkspace, 'a');
      expect(store.state.focusedSessionId, id('a', 'one'));
      expect(store.state.griddedSessionIds, [id('a', 'one'), id('a', 'two')]);
      expect(store.state.layer, KeyLayer.garage);
      expect(store.state.keysTargetChip, 'keys → garage');
    });

    test('worktree flag derives from dir differing from the registered dir', () {
      final store = storeWith(
        [ws('a')],
        [
          si('a', 'plain'),
          si('a', 'wt', dir: '/repos/.garage-worktrees/a-wt'),
        ],
      );
      expect(store.state.sessionById(id('a', 'plain'))!.worktree, isFalse);
      expect(store.state.sessionById(id('a', 'wt'))!.worktree, isTrue);
    });

    test('a vanished focused workspace falls back to the first group', () {
      final store = storeWith([ws('a'), ws('b')], [si('a', 'one')]);
      store.workspacesFetched([ws('b')]);
      store.sessionsFetched([si('b', 'other')]);
      expect(store.state.focusedWorkspace, 'b');
      expect(store.state.focusedSessionId, id('b', 'other'));
    });

    test(
        'p8.3 X-remove transition: after a registry-only workspace removal '
        'the refetch resurfaces its live sessions as a synthesized '
        'unregistered group', () {
      // Before: `a` is a registered workspace with two live sessions.
      final store = storeWith(
        [ws('a'), ws('b')],
        [si('a', 'one'), si('a', 'two'), si('b', 'other')],
      );
      expect(store.state.groupByName('a')!.registered, isTrue);

      // DELETE /api/workspaces/a (registry-only) → the refetch lists no
      // workspace `a`, but its tmux sessions are still live and listed.
      store.workspacesFetched([ws('b')]);
      store.sessionsFetched(
          [si('a', 'one'), si('a', 'two'), si('b', 'other')]);

      final group = store.state.groupByName('a');
      expect(group, isNotNull,
          reason: 'live sessions must never become invisible');
      expect(group!.registered, isFalse,
          reason: 'the group is now synthesized from tmux discovery');
      expect([for (final s in group.sessions) s.id],
          [id('a', 'one'), id('a', 'two')]);
      expect(store.state.sessionById(id('a', 'one'))!.live, isTrue);
      // Registered groups keep registry order first; the synthesized group
      // trails them (salience.dart buildGroups insertion order).
      expect([for (final g in store.state.groups) g.name], ['b', 'a']);
    });
  });

  group('layer transitions', () {
    test('engage requires a focused live session — empty wall is a no-op', () {
      final store = storeWith([ws('a')], []);
      store.engage();
      expect(store.state.layer, KeyLayer.garage);
    });

    test('engage on a restorable placeholder is a no-op', () {
      final store = storeWith(
        [ws('a')],
        [si('a', 'dead', status: 'restorable', since: null, restorable: true)],
      );
      expect(store.state.focusedSessionId, id('a', 'dead'));
      store.engage();
      expect(store.state.layer, KeyLayer.garage);
    });

    test('engage on a live session flips the layer and the chip in the same '
        'state change', () {
      final store = storeWith([ws('apexlabs')], [si('apexlabs', 'api-fix')]);
      store.engage();
      expect(store.state.layer, KeyLayer.engaged);
      expect(store.state.keysTargetChip, 'keys → apexlabs/api-fix');
    });

    test('disengage returns to garage; disengaging from garage is a no-op', () {
      final store = storeWith([ws('a')], [si('a', 'one')]);
      store.disengage();
      expect(store.state.layer, KeyLayer.garage);
      store.engage();
      store.disengage();
      expect(store.state.layer, KeyLayer.garage);
      expect(store.state.keysTargetChip, 'keys → garage');
    });

    test('overlays never stack: opening a second overlay is a no-op', () {
      final store = storeWith([ws('a')], [si('a', 'one')]);
      store.openOverlay(OverlayKind.help);
      expect(store.state.overlay, OverlayKind.help);
      store.openOverlay(OverlayKind.triageQueue);
      expect(store.state.overlay, OverlayKind.help);
      expect(store.state.layer, KeyLayer.overlay);
    });

    test('an overlay cannot open while engaged', () {
      final store = storeWith([ws('a')], [si('a', 'one')]);
      store.engage();
      store.openOverlay(OverlayKind.triageQueue);
      expect(store.state.layer, KeyLayer.engaged);
      expect(store.state.overlay, isNull);
    });

    test('closeOverlay returns to garage and clears the overlay slot', () {
      final store = storeWith([ws('a')], [si('a', 'one')]);
      store.openOverlay(OverlayKind.triageQueue);
      expect(store.state.keysTargetChip, 'keys → queue');
      store.closeOverlay();
      expect(store.state.layer, KeyLayer.garage);
      expect(store.state.overlay, isNull);
    });

    test('engagement is dropped when the engaged session leaves the '
        'listing', () {
      final store = storeWith([ws('a')], [si('a', 'one'), si('a', 'two')]);
      store.engage();
      expect(store.state.layer, KeyLayer.engaged);
      store.sessionsFetched([si('a', 'two')]);
      expect(store.state.layer, KeyLayer.garage);
      expect(store.state.focusedSessionId, id('a', 'two'));
    });
  });

  group('grid: 6-cap with LRU swap-in', () {
    List<SessionInfo> seven() =>
        [for (var i = 1; i <= 7; i++) si('a', 's$i')];

    test('a seventh session stays off the grid (rail only)', () {
      final store = storeWith([ws('a')], seven());
      expect(store.state.griddedSessionIds, hasLength(6));
      expect(store.state.griddedSessionIds, isNot(contains(id('a', 's7'))));
    });

    test('focusing the overflow session swaps it into the LRU slot, '
        'other tiles keep their positions', () {
      final store = storeWith([ws('a')], seven());
      // Touch every tile except s1, making s1 the least-recently-focused.
      for (var i = 2; i <= 6; i++) {
        store.focusSession(id('a', 's$i'));
      }
      store.focusSession(id('a', 's7'));
      // s7 took s1's slot (slot 0) — slot-stable swap-in.
      expect(store.state.griddedSessionIds, [
        id('a', 's7'),
        id('a', 's2'),
        id('a', 's3'),
        id('a', 's4'),
        id('a', 's5'),
        id('a', 's6'),
      ]);
      expect(store.state.focusedSessionId, id('a', 's7'));
    });

    test('LRU follows focus recency, not grid order', () {
      final store = storeWith([ws('a')], seven());
      store.focusSession(id('a', 's1')); // s1 is now most-recent; s2 is LRU
      store.focusSession(id('a', 's7'));
      expect(store.state.griddedSessionIds, contains(id('a', 's1')));
      expect(store.state.griddedSessionIds, isNot(contains(id('a', 's2'))));
      expect(
        store.state.griddedSessionIds.indexOf(id('a', 's7')),
        1, // s2's old slot
      );
    });

    test('below the cap, focusing an ungridded session appends it', () {
      final store = storeWith(
        [ws('a'), ws('b')],
        [si('a', 'one'), si('b', 'other')],
      );
      // Cross-workspace focus switches workspace and grids the session.
      store.focusSession(id('b', 'other'));
      expect(store.state.focusedWorkspace, 'b');
      expect(store.state.griddedSessionIds, [id('b', 'other')]);
    });

    test('cycleFocus wraps in both directions', () {
      final store = storeWith(
        [ws('a')],
        [si('a', 's1'), si('a', 's2'), si('a', 's3')],
      );
      store.cycleFocus(-1);
      expect(store.state.focusedSessionId, id('a', 's3'));
      store.cycleFocus(1);
      expect(store.state.focusedSessionId, id('a', 's1'));
      store.cycleFocus(1);
      expect(store.state.focusedSessionId, id('a', 's2'));
    });

    test('a gone gridded session is pruned and the grid refills from the '
        'rail overflow', () {
      final store = storeWith([ws('a')], seven());
      store.sessionsFetched([
        for (final s in seven())
          if (s.label != 's3') s,
      ]);
      expect(store.state.griddedSessionIds, hasLength(6));
      expect(store.state.griddedSessionIds, contains(id('a', 's7')));
      expect(store.state.griddedSessionIds, isNot(contains(id('a', 's3'))));
    });
  });

  group('a-jump', () {
    test('jumps to the longest-waiting blocked session across workspaces '
        'and lands engaged', () {
      final store = storeWith(
        [ws('a'), ws('b')],
        [
          si('a', 'one'),
          si('a', 'young', status: 'needs-input', since: 5000),
          si('b', 'old', status: 'needs-input', since: 100),
        ],
      );
      expect(store.state.focusedWorkspace, 'a');
      expect(store.jumpToLongestWaiting(), isTrue);
      expect(store.state.focusedWorkspace, 'b');
      expect(store.state.focusedSessionId, id('b', 'old'));
      expect(store.state.layer, KeyLayer.engaged);
      expect(store.state.keysTargetChip, 'keys → b/old');
    });

    test('an overflow blocked session is swapped into the grid by the '
        'jump', () {
      // 7 blocked sessions: salience keeps listing order, so the grid holds
      // s1..s6 and the oldest-waiting s7 is overflow until the jump.
      final store = storeWith(
        [ws('a')],
        [
          for (var i = 1; i <= 7; i++)
            si('a', 's$i', status: 'needs-input', since: 800 - i),
        ],
      );
      expect(store.state.griddedSessionIds, isNot(contains(id('a', 's7'))));
      expect(store.jumpToLongestWaiting(), isTrue);
      expect(store.state.griddedSessionIds, contains(id('a', 's7')));
      expect(store.state.focusedSessionId, id('a', 's7'));
      expect(store.state.layer, KeyLayer.engaged);
    });

    test('with nothing blocked the jump is a no-op returning false', () {
      final store = storeWith([ws('a')], [si('a', 'one')]);
      final before = store.state;
      expect(store.jumpToLongestWaiting(), isFalse);
      expect(store.state, same(before));
      expect(store.state.layer, KeyLayer.garage);
    });

    test('the jump only fires from the garage layer', () {
      final store = storeWith(
        [ws('a')],
        [si('a', 'one'), si('a', 'blocked', status: 'needs-input')],
      );
      store.focusSession(id('a', 'one'));
      store.engage();
      expect(store.jumpToLongestWaiting(), isFalse);
      expect(store.state.focusedSessionId, id('a', 'one'));
      expect(store.state.layer, KeyLayer.engaged);
    });
  });

  group('garage command mapping', () {
    test('maps every bound key: 1-9 [ ] a A n N Enter ? q', () {
      expect(garageCommandFor('1'), isA<FocusWorkspaceCommand>());
      expect((garageCommandFor('1')! as FocusWorkspaceCommand).index, 0);
      expect((garageCommandFor('9')! as FocusWorkspaceCommand).index, 8);
      expect((garageCommandFor('[')! as CycleFocusCommand).delta, -1);
      expect((garageCommandFor(']')! as CycleFocusCommand).delta, 1);
      expect(garageCommandFor('a'), isA<JumpCommand>());
      expect(garageCommandFor('A'), isA<OpenQueueCommand>());
      expect((garageCommandFor('n')! as SpawnCommand).worktree, isFalse);
      expect((garageCommandFor('N')! as SpawnCommand).worktree, isTrue);
      expect(garageCommandFor('\n'), isA<EngageCommand>());
      expect(garageCommandFor('\r'), isA<EngageCommand>());
      expect(garageCommandFor('?'), isA<ToggleHelpCommand>());
      expect(garageCommandFor('q'), isA<QuitCommand>());
    });

    test('unbound keys map to nothing (garage typing never reaches an '
        'agent)', () {
      expect(garageCommandFor('z'), isNull);
      expect(garageCommandFor('0'), isNull);
      expect(garageCommandFor(' '), isNull);
    });

    test('p8.1 lifecycle keys map to their commands', () {
      expect(garageCommandFor('m'), isA<MaximizeCommand>());
      expect(garageCommandFor('R'), isA<RestoreAllCommand>());
      expect(garageCommandFor('x'), isA<CloseCommand>());
      expect(garageCommandFor('w'), isA<WorkspaceAddCommand>());
    });

    test('p8.3: X maps to workspace removal, an effect the store declines',
        () {
      expect(garageCommandFor('X'), isA<WorkspaceRemoveCommand>());
      final store = storeWith([ws('a')], [si('a', 'one')]);
      expect(store.dispatch(const WorkspaceRemoveCommand()), isFalse);
      expect(store.state.layer, KeyLayer.garage,
          reason: 'declining must leave state untouched');
    });

    test('digits focus workspaces in salience order', () {
      final store = storeWith(
        [ws('a'), ws('b')],
        [si('a', 'one'), si('b', 'blocked', status: 'needs-input')],
      );
      // b is blocked, so it holds rail position 1.
      expect(store.dispatch(const FocusWorkspaceCommand(0)), isTrue);
      expect(store.state.focusedWorkspace, 'b');
      expect(store.dispatch(const FocusWorkspaceCommand(1)), isTrue);
      expect(store.state.focusedWorkspace, 'a');
      // Out-of-range digit: handled (consumed) but state unchanged.
      store.dispatch(const FocusWorkspaceCommand(8));
      expect(store.state.focusedWorkspace, 'a');
    });

    test('? toggles the help overlay', () {
      final store = storeWith([ws('a')], [si('a', 'one')]);
      expect(store.dispatch(const ToggleHelpCommand()), isTrue);
      expect(store.state.overlay, OverlayKind.help);
      expect(store.dispatch(const ToggleHelpCommand()), isTrue);
      expect(store.state.layer, KeyLayer.garage);
      expect(store.state.overlay, isNull);
    });

    test('spawn and quit are caller effects, not store transitions', () {
      final store = storeWith([ws('a')], [si('a', 'one')]);
      expect(store.dispatch(const SpawnCommand(worktree: false)), isFalse);
      expect(store.dispatch(const QuitCommand()), isFalse);
      expect(store.state.layer, KeyLayer.garage);
    });

    test('state commands are inert outside the garage layer', () {
      final store = storeWith([ws('a'), ws('b')], [si('a', 'one')]);
      store.engage();
      expect(store.dispatch(const FocusWorkspaceCommand(1)), isFalse);
      expect(store.state.focusedWorkspace, 'a');
      expect(store.dispatch(const JumpCommand()), isFalse);
    });
  });

  group('status events', () {
    test('statusChanged reorders salience and stamps the new since', () {
      final store = storeWith(
        [ws('a'), ws('b')],
        [si('a', 'one'), si('b', 'other')],
      );
      store.statusChanged(id('b', 'other'), 'needs-input', 4242);
      expect(store.state.groups.first.name, 'b');
      final s = store.state.sessionById(id('b', 'other'))!;
      expect(s.status, 'needs-input');
      expect(s.since, 4242);
    });

    test('a transition away from needs-input clears the message (mirrors '
        'the daemon store)', () {
      final store = storeWith(
        [ws('a')],
        [
          si('a', 'one',
              status: 'needs-input', message: 'Claude needs your permission'),
        ],
      );
      expect(store.state.sessionById(id('a', 'one'))!.message,
          'Claude needs your permission');
      store.statusChanged(id('a', 'one'), 'working', 9000);
      expect(store.state.sessionById(id('a', 'one'))!.message, isNull);
    });

    test('statusChanged for an unknown id is ignored until the refetch', () {
      final store = storeWith([ws('a')], [si('a', 'one')]);
      final before = store.state;
      store.statusChanged('garage/a/ghost', 'needs-input', 1);
      expect(store.state, same(before));
    });
  });

  group('maximize (p8.1, spec tui-wall "Maximized tile")', () {
    test('m toggles the focused tile full-grid and back', () {
      final store = storeWith([ws('a')], [si('a', 'one'), si('a', 'two')]);
      expect(store.state.maximizedSessionId, isNull);
      store.dispatch(const MaximizeCommand());
      expect(store.state.maximizedSessionId, id('a', 'one'));
      store.dispatch(const MaximizeCommand());
      expect(store.state.maximizedSessionId, isNull);
    });

    test('focusing another session exits maximize', () {
      final store = storeWith([ws('a')], [si('a', 'one'), si('a', 'two')]);
      store.toggleMaximize();
      expect(store.state.maximizedSessionId, id('a', 'one'));
      store.focusSession(id('a', 'two'));
      expect(store.state.maximizedSessionId, isNull);
      expect(store.state.focusedSessionId, id('a', 'two'));
    });

    test('cycling focus ([ / ]) exits maximize', () {
      final store = storeWith([ws('a')], [si('a', 'one'), si('a', 'two')]);
      store.toggleMaximize();
      store.cycleFocus(1);
      expect(store.state.maximizedSessionId, isNull);
    });

    test('switching workspace exits maximize', () {
      final store =
          storeWith([ws('a'), ws('b')], [si('a', 'one'), si('b', 'other')]);
      store.toggleMaximize();
      store.focusWorkspace(1);
      expect(store.state.maximizedSessionId, isNull);
    });

    test('re-focusing the maximized session keeps it maximized', () {
      final store = storeWith([ws('a')], [si('a', 'one'), si('a', 'two')]);
      store.toggleMaximize();
      store.focusSession(id('a', 'one'));
      expect(store.state.maximizedSessionId, id('a', 'one'));
    });

    test('engagement keeps maximize (maximize+engage is the workflow)', () {
      final store = storeWith([ws('a')], [si('a', 'one'), si('a', 'two')]);
      store.toggleMaximize();
      store.engage();
      expect(store.state.layer, KeyLayer.engaged);
      expect(store.state.maximizedSessionId, id('a', 'one'));
      // m is a garage-layer binding: while engaged the toggle is inert.
      store.toggleMaximize();
      expect(store.state.maximizedSessionId, id('a', 'one'));
    });

    test('reconciliation clears maximize when the session dies', () {
      final store = storeWith([ws('a')], [si('a', 'one'), si('a', 'two')]);
      store.toggleMaximize();
      store.sessionsFetched([si('a', 'two')]);
      expect(store.state.maximizedSessionId, isNull);
      expect(store.state.focusedSessionId, id('a', 'two'));
    });

    test('maximize with no focus (empty wall) is a no-op', () {
      final store = storeWith([ws('a')], []);
      store.toggleMaximize();
      expect(store.state.maximizedSessionId, isNull);
    });
  });

  group('restore / close / workspace-add commands (p8.1)', () {
    test('engage returns false for a restorable placeholder (caller '
        'restores instead)', () {
      final store = storeWith(
        [ws('a')],
        [si('a', 'gone', status: 'restorable', since: null, restorable: true)],
      );
      expect(store.dispatch(const EngageCommand()), isFalse);
      expect(store.state.layer, KeyLayer.garage,
          reason: 'engage must still require a live session');
    });

    test('engage still works (and returns true) for a live session', () {
      final store = storeWith([ws('a')], [si('a', 'one')]);
      expect(store.dispatch(const EngageCommand()), isTrue);
      expect(store.state.layer, KeyLayer.engaged);
    });

    test('R and x are effects — the store always declines them', () {
      final store = storeWith([ws('a')], [si('a', 'one')]);
      expect(store.dispatch(const RestoreAllCommand()), isFalse);
      expect(store.dispatch(const CloseCommand()), isFalse);
      expect(store.state.layer, KeyLayer.garage);
    });

    test('w opens the add-workspace overlay; overlays never stack', () {
      final store = storeWith([ws('a')], [si('a', 'one')]);
      expect(store.dispatch(const WorkspaceAddCommand()), isTrue);
      expect(store.state.layer, KeyLayer.overlay);
      expect(store.state.overlay, OverlayKind.workspaceAdd);
      expect(store.state.keysTargetChip, 'keys → add workspace');
      expect(store.dispatch(const WorkspaceAddCommand()), isFalse);
      store.closeOverlay();
      expect(store.state.layer, KeyLayer.garage);
    });

    test('focusWorkspaceNamed lands on the named group', () {
      final store =
          storeWith([ws('a'), ws('b')], [si('a', 'one'), si('b', 'other')]);
      store.focusWorkspaceNamed('b');
      expect(store.state.focusedWorkspace, 'b');
      expect(store.state.focusedSessionId, id('b', 'other'));
      store.focusWorkspaceNamed('ghost'); // unknown name is a no-op
      expect(store.state.focusedWorkspace, 'b');
    });

    test('restorableSessionIds selects only restorable sessions of the '
        'workspace, listing order preserved', () {
      final store = storeWith(
        [ws('a'), ws('b')],
        [
          si('a', 'live'),
          si('a', 'r1', status: 'restorable', since: null, restorable: true),
          si('b', 'r-other',
              status: 'restorable', since: null, restorable: true),
          si('a', 'r2', status: 'restorable', since: null, restorable: true),
        ],
      );
      expect(restorableSessionIds(store.state.sessions, 'a'),
          [id('a', 'r1'), id('a', 'r2')]);
      expect(restorableSessionIds(store.state.sessions, 'b'),
          [id('b', 'r-other')]);
      expect(restorableSessionIds(store.state.sessions, 'ghost'), isEmpty);
    });
  });
}
