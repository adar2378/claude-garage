// Regression test for the vendored nocterm output-callback-leak patch
// (tui/vendor/NOCTERM_VERSION, patch 5).
//
// Upstream TerminalXterm registers an anonymous output callback on its
// PtyController and never removes it — the callback leaks past unmount and
// past controller swaps. A leaked callback's setState throws on the defunct
// element, aborting the controller's callback fan-out loop and starving
// every callback registered after it (symptom: a reattached/reshuffled tile
// renders blank forever). The patch keeps a reference and detaches on
// dispose() and on controller change; `debugOutputCallbackCount` is the
// paired test hook on PtyController.
import 'package:nocterm/nocterm.dart';
import 'package:test/test.dart';

void main() {
  // The test binding is a singleton — one tester for the whole file.
  late NoctermTester tester;
  setUpAll(() async => tester = await NoctermTester.create());

  test('unmounting TerminalXterm removes its output callback', () async {
    final controller = PtyController(command: '/bin/true');

    await tester.pumpComponent(
        TerminalXterm(controller: controller, autoStart: false));
    expect(controller.debugOutputCallbackCount, 1);

    // Replace the terminal with something else — the element unmounts.
    await tester.pumpComponent(Text('gone'));
    expect(controller.debugOutputCallbackCount, 0,
        reason: 'dispose() must detach the output callback');
  });

  test('controller swap moves the callback instead of leaking it', () async {
    final a = PtyController(command: '/bin/true');
    final b = PtyController(command: '/bin/true');

    await tester.pumpComponent(TerminalXterm(controller: a, autoStart: false));
    expect(a.debugOutputCallbackCount, 1);

    await tester.pumpComponent(TerminalXterm(controller: b, autoStart: false));
    expect(a.debugOutputCallbackCount, 0,
        reason: 'didUpdateComponent must detach from the old controller');
    expect(b.debugOutputCallbackCount, 1);
  });
}
