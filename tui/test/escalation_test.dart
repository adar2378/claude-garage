// EscalationPolicy (tui/lib/ui/escalation.dart) as a pure policy fed
// session snapshots, writes captured by an injected sink. Spec: tui-triage
// "Off-screen escalation" — BEL once per needs-input transition, OSC 0
// title carrying the blocked count on every count change. Plus the
// VisibilityHeartbeat contract (true on start / every beat, false on stop).
import 'dart:async';

import 'package:garage_tui/state/wall_state.dart';
import 'package:garage_tui/ui/escalation.dart';
import 'package:test/test.dart';

WallSession session(String label, {String status = 'working', int? since = 1}) =>
    WallSession(
      id: 'garage/ws/$label',
      workspace: 'ws',
      label: label,
      status: status,
      since: since,
    );

String title(String text) => '\x1b]0;$text\x07';

void main() {
  group('EscalationPolicy', () {
    late List<String> writes;
    late EscalationPolicy policy;

    setUp(() {
      writes = [];
      policy = EscalationPolicy(write: writes.add);
    });

    test('title strings for 0/1/3 blocked', () {
      expect(policy.titleFor(0), 'garage');
      expect(policy.titleFor(1), '(1) garage');
      expect(policy.titleFor(3), '(3) garage');
    });

    test('first update claims the title without a bell at zero blocked', () {
      policy.update([session('a')]);
      expect(writes, [title('garage')]);
    });

    test('entering needs-input rings once and retitles', () {
      policy.update([session('a')]);
      writes.clear();
      policy.update([session('a', status: 'needs-input')]);
      // Exactly one write: a single leading BEL, then the OSC title.
      expect(writes, ['\x07${title('(1) garage')}']);
    });

    test('needs-input → needs-input repeat emits nothing', () {
      policy.update([session('a', status: 'needs-input')]);
      writes.clear();
      // Same snapshot again (an SSE refetch), even with a new since.
      policy.update([session('a', status: 'needs-input', since: 99)]);
      expect(writes, isEmpty);
    });

    test('leaving needs-input retitles without a bell', () {
      policy.update([session('a', status: 'needs-input')]);
      writes.clear();
      policy.update([session('a', status: 'working')]);
      expect(writes, [title('garage')]); // count change, no leading BEL
    });

    test('leave-and-reenter rings again', () {
      policy.update([session('a', status: 'needs-input')]);
      policy.update([session('a', status: 'working')]);
      writes.clear();
      policy.update([session('a', status: 'needs-input')]);
      expect(writes, ['\x07${title('(1) garage')}']);
    });

    test('a second blocked session rings and bumps the count', () {
      policy.update([session('a', status: 'needs-input'), session('b')]);
      writes.clear();
      policy.update([
        session('a', status: 'needs-input'),
        session('b', status: 'needs-input'),
      ]);
      expect(writes, ['\x07${title('(2) garage')}']);
    });

    test('simultaneous transitions coalesce to one bell', () {
      policy.update([session('a'), session('b')]);
      writes.clear();
      policy.update([
        session('a', status: 'needs-input'),
        session('b', status: 'needs-input'),
      ]);
      expect(writes, ['\x07${title('(2) garage')}']);
    });

    test('title returns to plain garage when the last blocker clears', () {
      policy.update([session('a', status: 'needs-input')]);
      writes.clear();
      policy.update([]);
      expect(writes, [title('garage')]);
    });
  });

  group('VisibilityHeartbeat', () {
    test('posts visible on start, invisible once on stop, same clientId',
        () async {
      final calls = <(String, bool)>[];
      final beat = VisibilityHeartbeat(
        post: (clientId, visible) async => calls.add((clientId, visible)),
        interval: const Duration(milliseconds: 20),
      );
      beat.start();
      await Future<void>.delayed(const Duration(milliseconds: 50));
      await beat.stop();

      expect(calls.length, greaterThanOrEqualTo(3)); // start + ≥1 beat + stop
      expect(calls.first.$2, isTrue);
      expect(calls.last.$2, isFalse);
      expect(calls.where((c) => !c.$2).length, 1);
      expect({for (final c in calls) c.$1}.length, 1); // one clientId
      expect(beat.clientId, isNotEmpty);
    });

    test('swallows post failures', () async {
      final beat = VisibilityHeartbeat(
        post: (_, __) async => throw StateError('daemon down'),
        interval: const Duration(seconds: 30),
      );
      beat.start();
      await beat.stop(); // must not throw
    });

    test('generated clientIds are per-process-unique-ish', () {
      Future<void> post(String c, bool v) async {}
      final a = VisibilityHeartbeat(post: post).clientId;
      final b = VisibilityHeartbeat(post: post).clientId;
      expect(a, isNot(b));
    });
  });
}
