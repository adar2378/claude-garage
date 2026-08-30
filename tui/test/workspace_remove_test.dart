// p8.4 kill-all removal confirm (tui/lib/state/workspace_remove.dart):
// `K` while the `X` remove arm is live confirms removal WITH session kill —
// spec tui-key-routing "p8.4 kill-all removal confirm". The ArmedAction
// machine stays generic; these tests pin the caller-level `K` branch and
// the strip wording.
import 'package:garage_tui/state/armed_action.dart';
import 'package:garage_tui/state/store.dart' show garageCommandFor;
import 'package:garage_tui/state/workspace_remove.dart';
import 'package:test/test.dart';

void main() {
  group('confirmKillTarget (the K branch on the X arm)', () {
    test('K with nothing armed confirms nothing', () {
      final armed = ArmedAction();
      expect(confirmKillTarget(armed, 1000), isNull);
      expect(armed.armedId, isNull, reason: 'K must never arm');
    });

    test('K confirms the armed workspace within the window and consumes '
        'the arm', () {
      final armed = ArmedAction();
      expect(armed.press('proj', 1000), isFalse); // X arms
      expect(confirmKillTarget(armed, 2500), 'proj');
      expect(armed.armedId, isNull, reason: 'confirm consumes the arm');
      expect(confirmKillTarget(armed, 2600), isNull,
          reason: 'a second K after the confirm is unbound again');
    });

    test('an expired arm never confirms — and K never re-arms it', () {
      final armed = ArmedAction(timeout: const Duration(seconds: 3));
      armed.press('proj', 1000);
      expect(confirmKillTarget(armed, 4001), isNull, reason: 'window expired');
      expect(armed.armedId, isNull,
          reason: 'the stale arm is disarmed, not re-armed by K');
    });

    test('after an X-X confirm the machine is empty, so a trailing K is '
        'unbound', () {
      final armed = ArmedAction();
      armed.press('proj', 1000);
      expect(armed.press('proj', 1500), isTrue); // X-X registry-only confirm
      expect(confirmKillTarget(armed, 1600), isNull);
    });

    test('only a REGISTERED workspace can ever reach a K confirm: p8.3 '
        'never arms unregistered groups, so the machine stays empty', () {
      // The TUI's _removePressed returns before press() for a group with
      // registered: false — modeled here as "no press happened".
      final armed = ArmedAction();
      expect(armed.armedId, isNull);
      expect(confirmKillTarget(armed, 1000), isNull);
    });

    test('K outside an arm maps to no garage command (typing hint path)',
        () {
      expect(garageCommandFor('K'), isNull);
    });
  });

  group('removeArmNotice wording', () {
    test('live sessions append the K clause with the count', () {
      expect(
          removeArmNotice('proj', 2),
          'press X again to remove proj (sessions keep running)'
          ' · K to also kill its 2 sessions');
    });

    test('zero live sessions omit the K clause (p8.3 wording unchanged)',
        () {
      expect(removeArmNotice('proj', 0),
          'press X again to remove proj (sessions keep running)');
    });
  });

  group('killRemoveNotice wording', () {
    test('clean kill reports the count', () {
      expect(
          killRemoveNotice('proj', {
            'removed': 'proj',
            'killedSessions': ['garage/proj/a', 'garage/proj/b'],
          }),
          'removed proj · killed 2 sessions');
    });

    test('failures are named, never silently dropped', () {
      expect(
          killRemoveNotice('proj', {
            'removed': 'proj',
            'killedSessions': ['garage/proj/a'],
            'failedSessions': [
              {'id': 'garage/proj/b', 'error': 'boom'},
            ],
          }),
          'removed proj · killed 1 sessions'
          ' · failed to kill: garage/proj/b');
    });

    test('a missing/odd body still yields a sane notice', () {
      expect(killRemoveNotice('proj', null),
          'removed proj · killed 0 sessions');
    });
  });
}
