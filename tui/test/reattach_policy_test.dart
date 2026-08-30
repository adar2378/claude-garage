// Reattach policy (tui/lib/ui/tile_registry.dart): PTY exit while the
// session is still live → restart after 500 ms, max 4 attempts, then dead
// (spec tui-wall: "Tile PTY death recovery" — 4 × 500 ms fits the 2-second
// reattach budget). A healthy uptime resets the budget so a session
// recreated much later reattaches again.
import 'package:garage_tui/ui/tile_registry.dart';
import 'package:test/test.dart';

void main() {
  test('retries at 500ms up to 4 times, then reports dead', () {
    final policy = ReattachPolicy();
    for (var attempt = 1; attempt <= 4; attempt++) {
      expect(policy.onExit(Duration.zero), const Duration(milliseconds: 500),
          reason: 'attempt $attempt should retry');
    }
    expect(policy.onExit(Duration.zero), isNull,
        reason: 'the 5th exit exhausts the budget');
    expect(policy.onExit(Duration.zero), isNull, reason: 'and stays dead');
  });

  test('a healthy uptime resets the attempt budget', () {
    final policy = ReattachPolicy();
    policy.onExit(Duration.zero);
    policy.onExit(Duration.zero);
    policy.onExit(Duration.zero);
    // Attach then ran fine for a while before dying (e.g. the inner session
    // was recreated hours later): fresh budget.
    expect(policy.onExit(const Duration(seconds: 30)),
        const Duration(milliseconds: 500));
    expect(policy.attempts, 1);
  });

  test('an uptime just under healthy does not reset', () {
    final policy = ReattachPolicy();
    for (var i = 0; i < 4; i++) {
      policy.onExit(const Duration(seconds: 4));
    }
    expect(policy.onExit(const Duration(seconds: 4)), isNull);
  });

  test('attachEnvironment strips TMUX and pins TERM', () {
    final env = attachEnvironment();
    expect(env.containsKey('TMUX'), isFalse);
    expect(env['TERM'], 'xterm-256color');
  });
}
