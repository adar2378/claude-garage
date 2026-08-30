// Capture-pane history seeding seam (spec tui-scrollback "History depth",
// task 7.3) — flag-gated OFF by default on TilePtyRegistry.
//
// Policy-level: capture is injected, controllers are created but never
// started (the attach stagger is set beyond the test's lifetime and
// disposeAll cancels the pending timers).
import 'package:garage_tui/ui/tile_registry.dart';
import 'package:nocterm/nocterm.dart' show PtyController;
import 'package:test/test.dart';

TilePtyRegistry registry({
  required bool seedHistory,
  required String? Function(String) capturePane,
}) =>
    TilePtyRegistry(
      isSessionLive: (_) => true,
      seedHistory: seedHistory,
      // Never fires within a test run — controllers stay un-started.
      attachStagger: const Duration(days: 1),
      createController: (id) => PtyController(command: 'true'),
      capturePane: capturePane,
    );

void main() {
  test('default: seeding is OFF — capture never runs, seedFor is null', () {
    var captures = 0;
    final r = registry(
        seedHistory: false,
        capturePane: (_) {
          captures++;
          return 'nope';
        });
    r.sync(['garage/ws/a']);
    expect(captures, 0);
    expect(r.seedFor('garage/ws/a'), isNull);
    r.disposeAll();
  });

  test('flag on: capture runs once per entry, before the first attach', () {
    final captured = <String>[];
    final r = registry(
        seedHistory: true,
        capturePane: (id) {
          captured.add(id);
          return 'history of $id\r\n';
        });
    r.sync(['garage/ws/a', 'garage/ws/b']);
    expect(captured, unorderedEquals(['garage/ws/a', 'garage/ws/b']));
    expect(r.seedFor('garage/ws/a'), 'history of garage/ws/a\r\n');
    // Re-sync with the same ids: entries already exist, no re-capture.
    r.sync(['garage/ws/a', 'garage/ws/b']);
    expect(captured.length, 2);
    r.disposeAll();
  });

  test('capture failure degrades to no seed', () {
    final r = registry(seedHistory: true, capturePane: (_) => null);
    r.sync(['garage/ws/a']);
    expect(r.seedFor('garage/ws/a'), isNull);
    r.disposeAll();
  });

  test('seedFor is null for unknown/removed entries', () {
    final r = registry(seedHistory: true, capturePane: (_) => 'x\r\n');
    r.sync(['garage/ws/a']);
    r.sync([]); // removed
    expect(r.seedFor('garage/ws/a'), isNull);
    r.disposeAll();
  });
}
