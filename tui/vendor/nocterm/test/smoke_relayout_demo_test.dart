// E2E smoke test for example/relayout_fixes_demo.dart - drives the demo
// through the real key-handling path and asserts every panel reacts.
// Each panel exercises one of the explicit-markNeedsLayout fixes, so this
// doubles as an integration guard for the layout-skip contract.
import 'package:nocterm/nocterm.dart';
import 'package:test/test.dart';

import '../example/relayout_fixes_demo.dart';

void main() {
  test('relayout fixes demo: every panel reacts to its key', () async {
    await testNocterm('demo smoke', (tester) async {
      await tester.pumpComponent(const RelayoutFixesDemo());
      // Settle transient first-layout dirtiness so each keypress below
      // exercises the explicit markNeedsLayout paths.
      await tester.pump();

      // Initial state of all panels.
      expect(tester.terminalState.containsText('Overlay left=1'), isTrue);
      expect(tester.terminalState.containsText('[BOX]'), isTrue);
      expect(tester.terminalState.containsText('Padding all(0)'), isTrue);
      expect(tester.terminalState.containsText('Align topLeft'), isTrue);
      expect(tester.terminalState.containsText('order=A,B'), isTrue);
      expect(tester.terminalState.containsText('counter=0'), isTrue);

      // [o] Overlay box must move.
      final boxBefore = tester.terminalState.findText('[BOX]').single;
      await tester.sendKey(LogicalKey.keyO);
      await tester.pump();
      final boxAfter = tester.terminalState.findText('[BOX]').single;
      expect(boxAfter.x, isNot(boxBefore.x),
          reason: 'Positioned box inside Overlay must move on [o]');

      // [p] Divider must shrink when padding grows.
      // The demo's divider uses ═ (double style) precisely so it can be
      // told apart from the ─ panel borders.
      int dividerWidth() {
        final text = tester.terminalState.getText();
        return RegExp(r'═+')
            .allMatches(text)
            .fold(0, (max, m) => m.end - m.start > max ? m.end - m.start : max);
      }

      final wideDivider = dividerWidth();
      await tester.sendKey(LogicalKey.keyP);
      await tester.pump();
      expect(tester.terminalState.containsText('Padding all(2)'), isTrue);
      expect(dividerWidth(), lessThan(wideDivider),
          reason: 'Divider must shrink when padding grows on [p]');

      // [a] @@ must move to center.
      final xBefore = tester.terminalState.findText('@@').single;
      await tester.sendKey(LogicalKey.keyA);
      await tester.pump();
      expect(tester.terminalState.containsText('Align center'), isTrue);
      final xAfter = tester.terminalState.findText('@@').single;
      expect((xAfter.x, xAfter.y), isNot((xBefore.x, xBefore.y)),
          reason: 'Aligned @@ must move on [a]');

      // [s] AAA/BB must swap offsets.
      final aBefore = tester.terminalState.findText('AAA').single.x;
      final bBefore = tester.terminalState.findText('BB').single.x;
      expect(aBefore, lessThan(bBefore));
      await tester.sendKey(LogicalKey.keyS);
      await tester.pump();
      final aAfter = tester.terminalState.findText('AAA').single.x;
      final bAfter = tester.terminalState.findText('BB').single.x;
      expect(bAfter, lessThan(aAfter),
          reason: 'const keyed Row children must swap offsets on [s]');

      // [h] Hoisted builders must re-render the new counter.
      await tester.sendKey(LogicalKey.keyH);
      await tester.pump();
      expect(tester.terminalState.containsText('item 0 -> counter=1'), isTrue,
          reason: 'hoisted ListView itemBuilder must see counter=1 on [h]');
      expect(tester.terminalState.containsText('LayoutBuilder -> counter=1'),
          isTrue,
          reason: 'hoisted LayoutBuilder builder must see counter=1 on [h]');
    }, size: const Size(80, 26));
  });
}
