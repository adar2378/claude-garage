import 'package:nocterm/nocterm.dart';
import 'package:test/test.dart';

/// Tests for NavigatorObserver notifications - previously zero-covered:
/// observers are the integration point for analytics/route-aware widgets,
/// so each navigation operation must report the right routes in order.
void main() {
  group('NavigatorObserver', () {
    test('didPush/didPop/didReplace fire with the correct routes', () async {
      await testNocterm('observer events', (tester) async {
        final log = <String>[];
        final observer = _RecordingObserver(log);

        await tester.pumpComponent(Navigator(
          observers: [observer],
          home: const Text('Home Page'),
          routes: {
            '/settings': (context) => const Text('Settings Page'),
            '/about': (context) => const Text('About Page'),
          },
        ));

        expect(log, ['push:/:none'],
            reason: 'installing the home route must notify didPush');

        final navigator = tester.findState<NavigatorState>();

        navigator.pushNamed('/settings');
        await tester.pump();
        expect(log.last, 'push:/settings:/');
        expect(tester.terminalState, containsText('Settings Page'));

        navigator.pop();
        await tester.pump();
        expect(log.last, 'pop:/settings:/');
        expect(tester.terminalState, containsText('Home Page'));

        navigator.pushNamed('/settings');
        await tester.pump();
        navigator.pushReplacementNamed('/about');
        await tester.pump();
        expect(log.last, 'replace:/about:/settings');
        expect(tester.terminalState, containsText('About Page'));
      }, size: const Size(30, 5));
    });
  });
}

class _RecordingObserver extends NavigatorObserver {
  _RecordingObserver(this.log);

  final List<String> log;

  static String _name(Route? route) => route?.settings.name ?? 'none';

  @override
  void didPush(Route route, Route? previousRoute) {
    log.add('push:${_name(route)}:${_name(previousRoute)}');
  }

  @override
  void didPop(Route route, Route? previousRoute) {
    log.add('pop:${_name(route)}:${_name(previousRoute)}');
  }

  @override
  void didReplace({Route? newRoute, Route? oldRoute}) {
    log.add('replace:${_name(newRoute)}:${_name(oldRoute)}');
  }
}
