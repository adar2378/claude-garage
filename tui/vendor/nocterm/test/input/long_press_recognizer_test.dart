import 'package:nocterm/nocterm.dart';
import 'package:test/test.dart';

/// Direct unit tests for LongPressGestureRecognizer's state machine.
///
/// The e2e path (GestureDetector + tester.press/pump) covers the happy
/// path; these cover the timing and slop branches deterministically with
/// a short duration.
void main() {
  const shortDuration = Duration(milliseconds: 40);

  MouseEvent down(int x, int y) =>
      MouseEvent(button: MouseButton.left, x: x, y: y, pressed: true);
  MouseEvent up(int x, int y) =>
      MouseEvent(button: MouseButton.left, x: x, y: y, pressed: false);
  MouseEvent move(int x, int y) => MouseEvent(
      button: MouseButton.left, x: x, y: y, pressed: true, isMotion: true);

  group('LongPressGestureRecognizer', () {
    test('fires start, callback, and end after holding past the duration',
        () async {
      LongPressStartDetails? startDetails;
      LongPressEndDetails? endDetails;
      var pressed = 0;
      final recognizer = LongPressGestureRecognizer(
        duration: shortDuration,
        onLongPress: () => pressed++,
        onLongPressStart: (d) => startDetails = d,
        onLongPressEnd: (d) => endDetails = d,
      );

      recognizer.addPointer(down(5, 3), const Offset(5, 3));
      expect(pressed, 0, reason: 'must not fire before the duration');

      await Future.delayed(shortDuration * 2);
      expect(pressed, 1);
      expect(startDetails?.localPosition, const Offset(5, 3));
      expect(endDetails, isNull, reason: 'end fires on release, not accept');

      recognizer.handlePointerUp(up(5, 3), const Offset(5, 3));
      expect(endDetails?.localPosition, const Offset(5, 3));
      expect(pressed, 1, reason: 'release must not re-fire the callback');
    });

    test('release before the duration fires nothing', () async {
      var pressed = 0;
      LongPressEndDetails? endDetails;
      final recognizer = LongPressGestureRecognizer(
        duration: shortDuration,
        onLongPress: () => pressed++,
        onLongPressEnd: (d) => endDetails = d,
      );

      recognizer.addPointer(down(5, 3), const Offset(5, 3));
      recognizer.handlePointerUp(up(5, 3), const Offset(5, 3));

      // Wait past the original deadline: the cancelled timer must not fire.
      await Future.delayed(shortDuration * 2);
      expect(pressed, 0);
      expect(endDetails, isNull,
          reason: 'end must not fire for a never-accepted long press');
    });

    test('moving beyond the touch slop cancels the long press', () async {
      var pressed = 0;
      final recognizer = LongPressGestureRecognizer(
        duration: shortDuration,
        onLongPress: () => pressed++,
      );

      recognizer.addPointer(down(5, 3), const Offset(5, 3));
      // Slop is 2 cells; move 3 cells away.
      recognizer.handlePointerMove(move(8, 3), const Offset(8, 3));

      await Future.delayed(shortDuration * 2);
      expect(pressed, 0, reason: 'drag beyond slop must cancel');
      // The recognizer resets to ready (it is persistent and reusable):
      // a fresh press afterwards must long-press normally.
      expect(recognizer.state, GestureRecognizerState.ready);
      recognizer.addPointer(down(5, 3), const Offset(5, 3));
      await Future.delayed(shortDuration * 2);
      expect(pressed, 1, reason: 'recognizer must be reusable after cancel');
    });

    test('moving within the touch slop keeps the long press alive', () async {
      var pressed = 0;
      final recognizer = LongPressGestureRecognizer(
        duration: shortDuration,
        onLongPress: () => pressed++,
      );

      recognizer.addPointer(down(5, 3), const Offset(5, 3));
      // Slop is 2 cells; jitter by 1 cell.
      recognizer.handlePointerMove(move(6, 3), const Offset(6, 3));

      await Future.delayed(shortDuration * 2);
      expect(pressed, 1, reason: 'jitter within slop must still long-press');
    });

    test('losing the arena cancels the pending timer', () async {
      var pressed = 0;
      final recognizer = LongPressGestureRecognizer(
        duration: shortDuration,
        onLongPress: () => pressed++,
      );

      recognizer.addPointer(down(5, 3), const Offset(5, 3));
      recognizer.resolve(GestureDisposition.rejected);

      await Future.delayed(shortDuration * 2);
      expect(pressed, 0, reason: 'a rejected recognizer must not fire');
    });
  });
}
