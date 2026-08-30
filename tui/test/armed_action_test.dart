// ArmedAction (tui/lib/state/armed_action.dart): the generic armed
// double-press machine behind the `x` session close (armed_close_test.dart
// covers that instance end-to-end via the ArmedClose alias) and p8.3's `X`
// workspace removal — spec tui-key-routing "p8.3 workspace removal".
import 'package:garage_tui/state/armed_action.dart';
import 'package:garage_tui/state/armed_close.dart' show ArmedClose;
import 'package:test/test.dart';

void main() {
  test('workspace-name keys arm and confirm like session ids', () {
    final armed = ArmedAction();
    expect(armed.press('apexlabs', 1000), isFalse);
    expect(armed.armedId, 'apexlabs');
    expect(armed.press('apexlabs', 2500), isTrue);
    expect(armed.armedId, isNull, reason: 'confirming consumes the arm');
  });

  test('a press aimed at a different workspace re-arms, never confirms', () {
    final armed = ArmedAction();
    expect(armed.press('apexlabs', 1000), isFalse);
    expect(armed.press('garage', 1500), isFalse,
        reason: 'focus moved to another workspace — new target');
    expect(armed.armedId, 'garage');
  });

  test('the x and X machines are independent instances: arming one never '
      'confirms the other', () {
    final close = ArmedAction(); // `x`, keyed by session id
    final remove = ArmedAction(); // `X`, keyed by workspace name
    expect(close.press('garage/ws/a', 1000), isFalse);
    expect(remove.press('ws', 1100), isFalse,
        reason: 'X after x must arm removal, not confirm anything');
    // In the TUI the X keypress also disarms the close machine ("any other
    // key disarms") — after that, x must arm from scratch.
    close.disarm();
    expect(close.press('garage/ws/a', 1200), isFalse);
    // The remove arm was untouched by the close machine's traffic.
    expect(remove.armedId, 'ws');
  });

  test('an expired arm re-arms instead of confirming', () {
    final armed = ArmedAction(timeout: const Duration(seconds: 3));
    armed.press('ws', 1000);
    expect(armed.press('ws', 4001), isFalse, reason: 'window expired');
    expect(armed.press('ws', 4500), isTrue);
  });

  test('ArmedClose is an alias of the generic machine (tests and callers '
      'keep compiling)', () {
    final ArmedClose armed = ArmedAction();
    expect(armed, isA<ArmedAction>());
  });
}
