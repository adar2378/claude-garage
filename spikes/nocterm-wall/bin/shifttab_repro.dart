// Minimal repro for the Shift+Tab (ESC[Z) drop — task group 8 of
// p8-nocterm-tui. One Focusable, logs every KeyboardEvent it receives.
// Run inside a detached tmux session and drive with `tmux send-keys -H`.
import 'dart:io';

import 'package:nocterm/nocterm.dart';

final logFile =
    File(Platform.environment['REPRO_LOG'] ?? '/tmp/shifttab-repro.log');

void main() async {
  logFile.writeAsStringSync('START ${DateTime.now()}\n',
      mode: FileMode.append);
  await runApp(const ReproApp());
}

class ReproApp extends StatefulComponent {
  const ReproApp({super.key});

  @override
  State<ReproApp> createState() => _ReproAppState();
}

class _ReproAppState extends State<ReproApp> {
  String last = '(none)';

  bool _onKey(KeyboardEvent e) {
    final cp = e.character?.codeUnits.map((c) => c.toRadixString(16)).join(',');
    logFile.writeAsStringSync(
        'HANDLER ${e.logicalKey} mods=${e.modifiers} cp=[$cp]\n',
        mode: FileMode.append);
    if (e.character == 'q') {
      shutdownApp();
      return true;
    }
    setState(() => last = '${e.logicalKey} mods=${e.modifiers}');
    return true;
  }

  @override
  Component build(BuildContext context) {
    return Focusable(
      focused: true,
      onKeyEvent: _onKey,
      child: Text('shifttab repro — last: $last'),
    );
  }
}
