import 'package:test/test.dart';
import 'package:nocterm/nocterm.dart' hide isNotEmpty;

/// End-to-end layout tests for East Asian Ambiguous punctuation
/// (General Punctuation 0x2010-0x205F: em dash, en dash, smart quotes,
/// ellipsis, bullet, prime).
///
/// These characters must render single-width. Treating them as
/// full-width inflates the measured width of any Latin text that
/// contains them, which shifts cell placement and breaks the
/// alignment of lists, padded tables, and bordered boxes. The unit
/// test in unicode_width_test.dart guards runeWidth(); these tests
/// guard the actual render pipeline (component -> buffer cells).
void main() {
  group('Ambiguous punctuation layout (e2e)', () {
    test('buffer places punctuation in exactly one cell', () {
      // setString is the placement path that drives rendering. A
      // wide character would push the next glyph one column right and
      // leave a zero-width marker behind. Ambiguous punctuation must not.
      final buffer = Buffer(20, 1);
      buffer.setString(0, 0, 'A—B•C…D');

      expect(buffer.getCell(0, 0).char, 'A');
      expect(buffer.getCell(1, 0).char, '—'); // em dash, width 1
      expect(buffer.getCell(2, 0).char, 'B');
      expect(buffer.getCell(3, 0).char, '•'); // bullet, width 1
      expect(buffer.getCell(4, 0).char, 'C');
      expect(buffer.getCell(5, 0).char, '…'); // ellipsis, width 1
      expect(buffer.getCell(6, 0).char, 'D');

      // No zero-width markers were inserted anywhere in the run.
      for (int x = 0; x <= 6; x++) {
        expect(buffer.getCell(x, 0).char, isNot('\u200B'),
            reason: 'unexpected wide-char marker at column $x');
      }
    });

    test('rendered text advances one column per punctuation glyph', () async {
      await testNocterm(
        'punctuation column advance',
        (tester) async {
          await tester.pumpComponent(
            const Align(
              alignment: Alignment.topLeft,
              child: Text('“A”—B…C'),
            ),
          );

          final state = tester.terminalState;
          expect(state.getCellAt(0, 0)?.char, '“'); // left double quote
          expect(state.getCellAt(1, 0)?.char, 'A');
          expect(state.getCellAt(2, 0)?.char, '”'); // right double quote
          expect(state.getCellAt(3, 0)?.char, '—'); // em dash
          expect(state.getCellAt(4, 0)?.char, 'B');
          expect(state.getCellAt(5, 0)?.char, '…'); // ellipsis
          expect(state.getCellAt(6, 0)?.char, 'C');
        },
      );
    });

    test('content-sized bordered box hugs punctuation text', () async {
      // A min-sized box wraps Text('A—B') (display width 3). The right
      // border must sit at column left+4 (1 border + 3 content). If the
      // em dash measured as width 2 the box would be one column wider.
      await testNocterm(
        'border hugs punctuation',
        (tester) async {
          await tester.pumpComponent(
            Align(
              alignment: Alignment.topLeft,
              child: Container(
                decoration: BoxDecoration(
                  border: BoxBorder.all(color: Colors.cyan),
                ),
                child: const Text('A—B'),
              ),
            ),
          );

          // Top border row: find the box-drawing corners.
          int? leftCol;
          int? rightCol;
          for (int x = 0; x < 80; x++) {
            final ch = tester.terminalState.getCellAt(x, 0)?.char;
            if (ch == '┌') leftCol = x;
            if (ch == '┐') rightCol = x;
          }
          expect(leftCol, isNotNull, reason: 'no top-left corner rendered');
          expect(rightCol, isNotNull, reason: 'no top-right corner rendered');
          // border + 3 content cells + border => width 5, corners 4 apart.
          expect(rightCol! - leftCol!, equals(4),
              reason: 'box width should hug 3-cell content, '
                  'got inner width ${rightCol - leftCol - 1}');
        },
      );
    });
  });
}
