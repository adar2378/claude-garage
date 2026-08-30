// Reconnect state machine (tui/lib/api/sse.dart): exponential backoff
// 500ms → 8s, reset on successful connect, and the 5s sessions-poll
// fallback active only while disconnected (design: "Poll fallback").
import 'package:garage_tui/api/sse.dart';
import 'package:test/test.dart';

void main() {
  test('backoff doubles from 500ms and caps at 8s', () {
    final machine = SseReconnectMachine()..start();
    final delays = [for (var i = 0; i < 7; i++) machine.onDisconnected()];
    expect(delays, const [
      Duration(milliseconds: 500),
      Duration(seconds: 1),
      Duration(seconds: 2),
      Duration(seconds: 4),
      Duration(seconds: 8),
      Duration(seconds: 8),
      Duration(seconds: 8),
    ]);
  });

  test('a successful connect resets the backoff to 500ms', () {
    final machine = SseReconnectMachine()..start();
    machine.onDisconnected();
    machine.onDisconnected();
    expect(machine.onDisconnected(), const Duration(seconds: 2));
    machine.onConnected();
    expect(machine.onDisconnected(), const Duration(milliseconds: 500));
  });

  test('poll fallback is active exactly while started-but-disconnected', () {
    final machine = SseReconnectMachine();
    expect(machine.pollFallbackActive, isFalse); // not started yet

    machine.start();
    expect(machine.pollFallbackActive, isTrue); // connecting

    machine.onConnected();
    expect(machine.pollFallbackActive, isFalse); // stream healthy
    expect(machine.connected, isTrue);

    machine.onDisconnected();
    expect(machine.pollFallbackActive, isTrue); // dropped — poll again
    expect(machine.connected, isFalse);

    machine.onConnected();
    expect(machine.pollFallbackActive, isFalse);
  });

  test('backoff stays capped over a very long outage (no overflow)', () {
    final machine = SseReconnectMachine()..start();
    Duration last = Duration.zero;
    for (var i = 0; i < 100; i++) {
      last = machine.onDisconnected();
    }
    expect(last, const Duration(seconds: 8));
  });
}
