// Shift+Tab repro, step 2: spike-app shape — 3 TerminalXterm tiles
// attached to inner tmux sessions (shifttab-inner-N), engage/disengage,
// every handler logs. Drive with `tmux send-keys -H` at the outer session.
import 'dart:io';

import 'package:nocterm/nocterm.dart';

final logFile =
    File(Platform.environment['REPRO_LOG'] ?? '/tmp/shifttab-repro.log');

const tileCount = 3;

void main() async {
  try {
    Process.runSync('/bin/sh', ['-c', 'stty -ixon -ixoff < /dev/tty']);
  } catch (_) {}
  logFile.writeAsStringSync('START ${DateTime.now()}\n',
      mode: FileMode.append);
  await runApp(const WallRepro());
}

class WallRepro extends StatefulComponent {
  const WallRepro({super.key});

  @override
  State<WallRepro> createState() => _WallReproState();
}

class _WallReproState extends State<WallRepro> {
  late final List<PtyController> controllers;
  int focus = 0;
  bool engaged = true; // start engaged on tile 0, like the failing scenario

  @override
  void initState() {
    super.initState();
    SchedulerBinding.instance.targetFrameDuration = FrameRate.fps15;
    final env = Map<String, String>.of(Platform.environment)
      ..remove('TMUX')
      ..['TERM'] = 'xterm-256color';
    controllers = List.generate(tileCount, (i) {
      return PtyController(
        command: 'tmux',
        arguments: ['attach', '-t', 'shifttab-inner-${i + 1}'],
        environment: env,
      );
    });
  }

  @override
  void dispose() {
    for (final c in controllers) {
      c.kill();
    }
    super.dispose();
  }

  void _log(String where, KeyboardEvent e) {
    final cp = e.character?.codeUnits.map((c) => c.toRadixString(16)).join(',');
    logFile.writeAsStringSync(
        '$where ${e.logicalKey} mods=${e.modifiers} cp=[$cp] engaged=$engaged\n',
        mode: FileMode.append);
  }

  bool _handleGarageKey(KeyboardEvent e) {
    _log('root   ', e);
    if (e.character == 'q') {
      shutdownApp();
      return true;
    }
    if (e.logicalKey == LogicalKey.enter) {
      setState(() => engaged = true);
      return true;
    }
    return true;
  }

  bool _handleEngagedKey(int i, KeyboardEvent e) {
    _log('tile[$i]', e);
    if (e.matches(LogicalKey.keyQ, ctrl: true)) {
      setState(() => engaged = false);
      return true;
    }
    return true;
  }

  @override
  Component build(BuildContext context) {
    return Focusable(
      focused: !engaged,
      onKeyEvent: _handleGarageKey,
      child: Row(
        children: [
          for (var i = 0; i < tileCount; i++)
            Expanded(
              child: TerminalXterm(
                controller: controllers[i],
                focused: engaged && focus == i,
                onKeyEvent: (e) => _handleEngagedKey(i, e),
              ),
            ),
        ],
      ),
    );
  }
}
