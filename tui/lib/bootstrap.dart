/// Startup/shutdown plumbing for the garage TUI.
///
/// Ordering (see openspec/changes/p8-nocterm-tui/design.md, "Rendering
/// budget"): daemon health check → disable terminal flow control → runApp →
/// restore flow control on exit. The fps15 frame cap is NOT set here — it must
/// be set in the root component's initState, because
/// SchedulerBinding.instance is null before runApp.
library;

import 'dart:async';
import 'dart:io';

import 'package:nocterm/nocterm.dart' show shutdownApp;

import 'api/client.dart' show defaultDaemonBaseUrl;

/// The daemon the TUI is a thin client of. No auth needed (the daemon allows
/// no-Origin requests from localhost). Honors GARAGE_TUI_PORT/GARAGE_PORT —
/// see [defaultDaemonBaseUrl] in api/client.dart.
final String daemonBaseUrl = defaultDaemonBaseUrl;

/// Returns true when the daemon answers `GET /api/health` with a 2xx.
Future<bool> daemonHealthy({Duration timeout = const Duration(seconds: 2)}) async {
  final client = HttpClient()..connectionTimeout = timeout;
  try {
    final request = await client
        .getUrl(Uri.parse('$daemonBaseUrl/api/health'))
        .timeout(timeout);
    final response = await request.close().timeout(timeout);
    await response.drain<void>();
    return response.statusCode >= 200 && response.statusCode < 300;
  } on Object {
    return false;
  } finally {
    client.close(force: true);
  }
}

/// Health-gate the UI: print an actionable error and exit(1) when the daemon
/// is unreachable. Call before touching the terminal.
Future<void> requireDaemon() async {
  if (await daemonHealthy()) return;
  stderr
    ..writeln('claude-garage daemon is not reachable at $daemonBaseUrl.')
    ..writeln('Start it first:')
    ..writeln()
    ..writeln('  npx claude-garage')
    ..writeln()
    ..writeln('then run the TUI again.');
  exit(1);
}

/// nocterm's raw mode only clears echo/line mode; IXON/IXOFF stay on (the
/// tty driver eats Ctrl+Q/Ctrl+S before the app ever sees them) and so does
/// ISIG (Ctrl+C becomes SIGINT for the whole foreground process group —
/// including the tiles' tmux attach clients — instead of the 0x03 byte an
/// engaged tile must forward; spec tui-key-routing "Control characters
/// pass"). Disable both ourselves before runApp, like any full-screen
/// terminal app. Emergency kill remains SIGTERM from outside (the backend
/// watches it and shuts down).
void disableFlowControl() {
  try {
    Process.runSync('/bin/sh', ['-c', 'stty -ixon -ixoff -isig < /dev/tty']);
  } catch (_) {
    // No controlling tty (tests, pipes) — nothing to do.
  }
}

/// Restore the tty settings changed by [disableFlowControl].
/// Call after the app has shut down, even on error paths.
void restoreFlowControl() {
  try {
    // icanon/echo too: nocterm's shutdown cancels the stdin subscription
    // (closing the fd) BEFORE disableRawMode, so its own echoMode/lineMode
    // restore throws EBADF and is swallowed — the tty would stay raw.
    Process.runSync(
        '/bin/sh', ['-c', 'stty ixon ixoff isig icanon echo < /dev/tty']);
  } catch (_) {
    // No controlling tty — nothing to restore.
  }
}

/// Quit the TUI, restoring the terminal first.
///
/// Always exit through this instead of [shutdownApp]: nocterm's shutdown path
/// ends in dart:io `exit()` without unwinding the stack, so a `finally` around
/// `runApp` never runs and stty settings would stay broken.
void shutdownTui([int exitCode = 0]) {
  restoreFlowControl();
  shutdownApp(exitCode);
}
