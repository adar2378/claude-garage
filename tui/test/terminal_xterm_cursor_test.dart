// Regression test for the vendored nocterm engaged-cursor patch
// (tui/vendor/NOCTERM_VERSION, patch 9).
//
// Upstream _TerminalRenderer never painted the inner terminal's cursor: the
// cursor cell's style was computed with a reverse flip and then dropped (a
// stale "reverse is not supported in TextStyle" note — nocterm's TextStyle
// does support reverse/SGR 7), the multi-style line fallback collapsed the
// whole line to one style anyway, an empty cursor cell was never styled at
// all, and the cursor line was matched against the view-relative cursorY
// instead of the absolute buffer line. Combined with nocterm hiding the
// host terminal's real cursor, an engaged Claude Code composer showed no
// caret whatsoever. Patch 9: TerminalXterm gains `showCursor` (default
// false); when set — and the view is live (not frozen/scrolled) and the
// application has not hidden the cursor via DECTCEM (`CSI ?25l`) — the
// renderer paints the cell at (cursorX, absoluteCursorY) in inverse video.
import 'package:nocterm/nocterm.dart' hide isEmpty;
import 'package:test/test.dart';

void main() {
  // The test binding is a singleton — one tester for the whole file
  // (default size 80×24; the emulator lays out to the same size).
  late NoctermTester tester;
  setUpAll(() async => tester = await NoctermTester.create());

  /// Every screen cell rendered with the inverse-video style.
  List<(int, int)> reversedCells() {
    final state = tester.terminalState;
    final cells = <(int, int)>[];
    for (var y = 0; y < 24; y++) {
      for (var x = 0; x < 80; x++) {
        final cell = state.getCellAt(x, y);
        if (cell != null && cell.style.reverse) cells.add((x, y));
      }
    }
    return cells;
  }

  Component terminal({
    bool showCursor = false,
    String seed = 'hello',
    int? frozenTopLine,
  }) =>
      TerminalXterm(
        controller: PtyController(command: '/bin/true'),
        autoStart: false,
        showCursor: showCursor,
        initialContent: seed,
        frozenTopLine: frozenTopLine,
      );

  /// Unmount between tests so each initialContent seeds a fresh emulator.
  Future<void> teardown() => tester.pumpComponent(Text('teardown'));

  test('showCursor paints the cell after the text as inverse video',
      () async {
    await tester.pumpComponent(terminal(showCursor: true));
    expect(reversedCells(), [(5, 0)],
        reason: 'cursor sits on the (empty) cell right after "hello" — '
            'exactly one inverted cell');
    expect(tester.terminalState.getCellAt(5, 0)!.char, ' ',
        reason: 'an empty cursor cell renders as an inverted space (solid '
            'block look)');
    await teardown();
  });

  test('cursor over a character keeps the character, inverted', () async {
    // CUP to row 1 col 3 → cursor on the third cell ("l" of hello).
    await tester.pumpComponent(
        terminal(showCursor: true, seed: 'hello\x1b[1;3H'));
    expect(reversedCells(), [(2, 0)]);
    expect(tester.terminalState.getCellAt(2, 0)!.char, 'l');
    await teardown();
  });

  test('unengaged (showCursor false, the default) paints no cursor',
      () async {
    await tester.pumpComponent(terminal());
    expect(reversedCells(), isEmpty,
        reason: 'a wall of unengaged tiles must not show carets');
    await teardown();
  });

  test('DECTCEM hide (CSI ?25l) suppresses the cursor even when engaged',
      () async {
    await tester.pumpComponent(
        terminal(showCursor: true, seed: 'hello\x1b[?25l'));
    expect(reversedCells(), isEmpty,
        reason: 'an app that hides its cursor keeps it hidden');
    await teardown();
  });

  test('DECTCEM show (CSI ?25h) brings it back', () async {
    await tester.pumpComponent(
        terminal(showCursor: true, seed: 'hello\x1b[?25l\x1b[?25h'));
    expect(reversedCells(), [(5, 0)]);
    await teardown();
  });

  test('a frozen history view never shows a cursor', () async {
    await tester.pumpComponent(
        terminal(showCursor: true, frozenTopLine: 0));
    expect(reversedCells(), isEmpty,
        reason: 'frozen scrollback is history — no live caret in it');
    await teardown();
  });
}
