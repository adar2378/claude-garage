// Regression test for the vendored nocterm size-propagation patch
// (tui/vendor/NOCTERM_VERSION, patch 8).
//
// Upstream rendered a hardcoded 80×24 ("for now") and reported that to
// onSizeChange, so the tile size never reached _updateSize; and
// PtyController.resize silently no-ops before the process runs, so the one
// resize issued before the (deferred, staggered) attach was dropped — every
// tmux attach client stayed 80×24 and tmux (window-size latest) wrapped
// session content at 80 cols inside narrower tiles. The patch derives
// cols/rows from layout constraints and re-pushes a dropped resize on the
// controller's transition to running (first attach AND restarts).
import 'package:nocterm/nocterm.dart';
import 'package:test/test.dart';

/// Fake controller: overridable running state, recorded resizes, listeners
/// firable from the test (the real _notifyListeners is private).
class FakePtyController extends PtyController {
  FakePtyController() : super(command: '/bin/true');

  bool running = false;
  final List<(int, int)> resizes = [];
  final List<VoidCallback> listeners = [];

  @override
  bool get isRunning => running;

  @override
  void resize(int columns, int rows) {
    if (!running) return; // upstream behavior: dropped before running
    resizes.add((columns, rows));
  }

  @override
  void addListener(VoidCallback listener) => listeners.add(listener);

  @override
  void removeListener(VoidCallback listener) => listeners.remove(listener);

  void setRunning(bool value) {
    running = value;
    for (final listener in List.of(listeners)) {
      listener();
    }
  }
}

/// The resize apply is deferred to a microtask (it must not run inside
/// build/layout) — yield the event loop before asserting.
Future<void> settle() => Future<void>.delayed(Duration.zero);

void main() {
  late NoctermTester tester;
  setUpAll(() async => tester = await NoctermTester.create());

  Component sized(FakePtyController controller, {double w = 46, double h = 9}) =>
      Center(
        child: SizedBox(
          width: w,
          height: h,
          child: TerminalXterm(controller: controller, autoStart: false),
        ),
      );

  test('a running PTY is resized to the laid-out size, not 80x24', () async {
    final controller = FakePtyController()..running = true;
    await tester.pumpComponent(sized(controller));
    await settle();
    expect(controller.resizes.length, greaterThan(0));
    expect(controller.resizes.last, (46, 9),
        reason: 'the renderer must report the constraint size upward');
    await tester.pumpComponent(Text('teardown'));
  });

  test('a resize dropped before start is re-pushed when the PTY runs',
      () async {
    final controller = FakePtyController(); // not running (staggered attach)
    await tester.pumpComponent(sized(controller));
    await settle();
    expect(controller.resizes.length, 0,
        reason: 'resize before running is dropped by the controller');

    controller.setRunning(true); // the registry's start() notifies listeners
    expect(controller.resizes.last, (46, 9),
        reason: 'the running transition must re-apply the tracked size');
    await tester.pumpComponent(Text('teardown'));
  });

  test('a restart (down → up again) gets the size re-pushed', () async {
    final controller = FakePtyController()..running = true;
    await tester.pumpComponent(sized(controller));
    await settle();
    final applied = controller.resizes.length;
    expect(applied, greaterThan(0));

    controller.setRunning(false); // attach died; registry restarts it
    controller.setRunning(true); // restart lands at the spawn default size
    expect(controller.resizes.length, greaterThan(applied),
        reason: 'the new PTY must be resized to the tile again');
    expect(controller.resizes.last, (46, 9));
    await tester.pumpComponent(Text('teardown'));
  });

  test('restart preserves listeners (patch 8c — a reattach must keep '
      'notifying the embedding component)', () async {
    final controller = PtyController(command: '/bin/true');
    var notified = 0;
    controller.addListener(() => notified++);
    expect(controller.debugListenerCount, 1);

    await controller.restart(); // dispose()+start(); dispose clears listeners
    expect(controller.debugListenerCount, 1,
        reason: 'restart() must carry the listeners across its dispose()');
    expect(notified, greaterThan(0),
        reason: 'the restarted start() must notify through the preserved '
            'listener (this is how the size re-push fires)');
    await controller.dispose();
  });

  test('a rect change (grid reshape / maximize) resizes the PTY', () async {
    final controller = FakePtyController()..running = true;
    await tester.pumpComponent(sized(controller));
    await settle();
    expect(controller.resizes.last, (46, 9));

    await tester.pumpComponent(sized(controller, w: 70, h: 20));
    await settle();
    expect(controller.resizes.last, (70, 20),
        reason: 'a new tile rect must propagate to the PTY');
    await tester.pumpComponent(Text('teardown'));
  });
}
