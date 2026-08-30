// PtyWriter (tui/lib/ui/pty_writer.dart): engaged-tile key bytes must
// survive the vendored PtyHandler's flush-collision StateError — Dart's
// IOSink throws synchronously on add-while-flushing, which used to drop
// every key after the first in a single stdin chunk. The writer queues,
// coalesces, and retries in order.
import 'dart:async';

import 'package:garage_tui/ui/pty_writer.dart';
import 'package:test/test.dart';

void main() {
  test('writes pass through in order when the sink accepts', () {
    final out = <String>[];
    final writer = PtyWriter.raw(isRunning: () => true, rawWrite: out.add);
    writer.write('l');
    writer.write('s');
    writer.write('\r');
    expect(out, ['l', 's', '\r']);
    expect(writer.hasPending, isFalse);
  });

  test('a mid-flush StateError queues and retries, preserving order',
      () async {
    final out = <String>[];
    var throwing = false;
    final writer = PtyWriter.raw(
      isRunning: () => true,
      rawWrite: (data) {
        if (throwing) throw StateError('StreamSink is bound to a stream');
        out.add(data);
      },
    );

    writer.write('l'); // accepted; the real sink now has a pending flush
    throwing = true;
    writer.write('s'); // both throw sync in the same chunk...
    writer.write('\r');
    expect(out, ['l']);
    expect(writer.hasPending, isTrue);

    throwing = false; // ...the flush completed
    await Future<void>.delayed(const Duration(milliseconds: 20));
    expect(out, ['l', 's\r'], reason: 'retry delivers the rest, coalesced, in order');
    expect(writer.hasPending, isFalse);
  });

  test('drops queued data instead of replaying into a dead PTY', () async {
    final out = <String>[];
    var running = true;
    var throwing = true;
    final writer = PtyWriter.raw(
      isRunning: () => running,
      rawWrite: (data) {
        if (throwing) throw StateError('flushing');
        out.add(data);
      },
    );
    writer.write('x');
    running = false; // PTY died while the retry was pending
    throwing = false;
    await Future<void>.delayed(const Duration(milliseconds: 20));
    expect(out, isEmpty);
    expect(writer.hasPending, isFalse);
  });

  test('dispose cancels the retry', () async {
    var calls = 0;
    final writer = PtyWriter.raw(
      isRunning: () => true,
      rawWrite: (_) {
        calls++;
        throw StateError('flushing');
      },
    );
    writer.write('x');
    final before = calls;
    writer.dispose();
    await Future<void>.delayed(const Duration(milliseconds: 20));
    expect(calls, before, reason: 'no retry fires after dispose');
  });
}
