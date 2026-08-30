# Testing nocterm

Quick guide to writing tests that actually catch bugs. The harness
(`testNocterm`, `NoctermTester`, `TerminalState`) is in
`package:nocterm/nocterm.dart` — see `test/visual/tui_test_example_test.dart`
for a tour.

## Settle before you mutate

The first layout pass leaves transient dirtiness behind: `adoptChild`
re-marks render objects *while* children are being built, so the frame
after `pumpComponent` re-layouts no matter what. A test that pumps once and
then mutates is exercising the always-dirty path — it can pass against code
where the mutation's relayout is silently skipped.

```dart
await tester.pumpComponent(MyApp());
await tester.pump();          // <- settle: next frame starts genuinely clean

state.mutateSomething();
await tester.pump();
expect(...);                  // now this discriminates
```

If the bug you're locking in involves a *skipped* relayout/repaint, the
settle pump is mandatory.

## Prefer positional assertions

`containsText('foo')` passes if the text appears anywhere on screen. It
cannot catch wrong position, stale ghosts painted alongside fresh content,
or one child overdrawing another. Reach for the stronger matchers:

```dart
expect(tester.terminalState, hasTextAt(0, 2, 'info'));   // exact position
expect(tester.terminalState, containsTextOnce('row-3')); // no duplicates
final m = tester.terminalState.findText('XX').single;    // position algebra
expect(m.x, greaterThan(previousX));
```

Use `containsText` only when position genuinely doesn't matter (e.g. "the
error message is shown somewhere").

Beware substring collisions: `findText('X')` also matches the X in
`[BOX]`. Pick markers that can't appear inside other strings.

## Prove a regression test discriminates

A regression test that has never failed proves nothing. Before committing
one, run it against the broken code and watch it fail:

```bash
git stash            # if the fix is uncommitted
dart test test/regression/my_test.dart   # must FAIL
git stash pop
dart test test/regression/my_test.dart   # must PASS
```

For already-committed fixes, check out the parent commit in a worktree:

```bash
git worktree add /tmp/check <fix-commit>~1
cp test/regression/my_test.dart /tmp/check/test/regression/
(cd /tmp/check && dart pub get -q && dart test test/regression/my_test.dart)
git worktree remove --force /tmp/check
```

This suite has had tests that passed on the very commit they were written
to guard against — the failure-first check is cheap insurance.

## Unit-test pure logic directly

State machines and data structures (gesture recognizers, `PersistentHashMap`)
deserve direct unit tests with short timeouts/synthetic events — see
`test/input/long_press_recognizer_test.dart`. Going through the full
component harness for these adds noise without adding coverage.

## Before committing

CI gates on formatting and analysis:

```bash
dart format <changed files>
dart analyze
dart test
```
