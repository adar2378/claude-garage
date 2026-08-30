// ArmedClose (tui/lib/state/armed_close.dart): the `x` double-press close
// state machine — spec tui-key-routing "p8.1 session lifecycle bindings"
// (second `x` within 3s confirms; any other key disarms; a different
// session re-arms instead of confirming).
import 'package:garage_tui/state/armed_close.dart';
import 'package:test/test.dart';

void main() {
  const a = 'garage/ws/a';
  const b = 'garage/ws/b';

  test('first press arms, second press within the window confirms', () {
    final armed = ArmedClose();
    expect(armed.press(a, 1000), isFalse);
    expect(armed.armedId, a);
    expect(armed.press(a, 2000), isTrue);
    expect(armed.armedId, isNull, reason: 'confirming consumes the arm');
  });

  test('a press after the 3s window re-arms instead of confirming', () {
    final armed = ArmedClose();
    expect(armed.press(a, 1000), isFalse);
    expect(armed.press(a, 4001), isFalse, reason: 'window expired');
    expect(armed.press(a, 4500), isTrue, reason: 'the re-arm opened a new window');
  });

  test('exactly at the window edge still confirms', () {
    final armed = ArmedClose();
    armed.press(a, 1000);
    expect(armed.press(a, 4000), isTrue);
  });

  test('an x aimed at a different session re-arms, never confirms', () {
    final armed = ArmedClose();
    expect(armed.press(a, 1000), isFalse);
    expect(armed.press(b, 1500), isFalse, reason: 'focus moved — new target');
    expect(armed.armedId, b);
    expect(armed.press(b, 2000), isTrue);
  });

  test('disarm (any other key) cancels the pending close', () {
    final armed = ArmedClose();
    armed.press(a, 1000);
    armed.disarm();
    expect(armed.armedId, isNull);
    expect(armed.press(a, 1100), isFalse, reason: 'must arm from scratch');
  });

  test('confirm never fires twice without a fresh arm', () {
    final armed = ArmedClose();
    armed.press(a, 1000);
    expect(armed.press(a, 1500), isTrue);
    expect(armed.press(a, 1600), isFalse,
        reason: 'the third x starts a new arming cycle');
  });
}
