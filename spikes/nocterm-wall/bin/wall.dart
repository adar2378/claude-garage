// Spike: 3x3 grid of live tmux-attached terminals in nocterm.
// Gates: (1) rendering perf under heavy output, (2) verbatim key
// passthrough incl. Alt+arrows, (3) local scrollback vs live.
//
// Keys (garage layer): 1-9 focus tile, Enter engage, q quit.
// Engaged: everything goes to the PTY raw; Ctrl+G returns.
import 'dart:io';

import 'package:nocterm/nocterm.dart';

const sessionCount = 9;

void main() async {
  // nocterm's enableRawMode only clears echo/line mode; IXON stays on and
  // the tty driver eats Ctrl+Q/Ctrl+S. Disable flow control ourselves.
  try {
    Process.runSync('/bin/sh', ['-c', 'stty -ixon -ixoff < /dev/tty']);
  } catch (_) {}
  await runApp(const WallApp());
}

class WallApp extends StatefulComponent {
  const WallApp({super.key});

  @override
  State<WallApp> createState() => _WallAppState();
}

class _WallAppState extends State<WallApp> {
  late final List<PtyController> controllers;
  int focus = 0;
  bool engaged = false;
  String lastKey = '';

  @override
  void initState() {
    super.initState();
    SchedulerBinding.instance.targetFrameDuration = FrameRate.fps15;
    final env = Map<String, String>.of(Platform.environment)
      ..remove('TMUX')
      ..['TERM'] = 'xterm-256color';
    controllers = List.generate(sessionCount, (i) {
      return PtyController(
        command: 'tmux',
        arguments: ['attach', '-t', 'garage-spike-${i + 1}'],
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
    File('/tmp/wall-keys.log').writeAsStringSync(
        '$where ${e.logicalKey} mods=${e.modifiers} cp=[$cp] engaged=$engaged focus=$focus\n',
        mode: FileMode.append);
  }

  bool _handleGarageKey(KeyboardEvent e) {
    _log('root   ', e);
    final cp = e.character?.codeUnits.map((c) => c.toRadixString(16)).join(',');
    setState(() => lastKey = '${e.logicalKey} mods=${e.modifiers} cp=[$cp]');
    final ch = e.character;
    if (ch != null && ch.length == 1 && '123456789'.contains(ch)) {
      setState(() => focus = int.parse(ch) - 1);
      return true;
    }
    if (e.logicalKey == LogicalKey.enter || ch == 'e') {
      setState(() => engaged = true);
      return true;
    }
    if (ch == 'q') {
      shutdownApp();
      return true;
    }
    return true; // garage layer consumes everything
  }

  bool _handleEngagedKey(int i, KeyboardEvent e) {
    _log('tile[$i]', e);
    if (e.matches(LogicalKey.keyQ, ctrl: true)) {
      setState(() => engaged = false);
      return true;
    }
    // nocterm collapses bracketed paste AND coalesced typed chars into a
    // synthetic Ctrl+V with the text parked in ClipboardManager. Recover it
    // and forward as a bracketed paste to the PTY.
    if (e.matches(LogicalKey.keyV, ctrl: true)) {
      final text = ClipboardManager.paste();
      if (text != null && text.isNotEmpty) {
        controllers[i].write('\x1b[200~$text\x1b[201~');
        return true;
      }
    }
    // Shift+PageUp/Down: local scrollback — return false so TerminalXterm's
    // own _scrollUp/_scrollDown handles it (gate 3: history vs live seam).
    if (e.modifiers.shift &&
        (e.logicalKey == LogicalKey.pageUp ||
            e.logicalKey == LogicalKey.pageDown)) {
      return false;
    }
    final bytes = encodeKey(e);
    if (bytes != null) controllers[i].write(bytes);
    return true; // never fall through to TerminalXterm's lossy default
  }

  @override
  Component build(BuildContext context) {
    return Focusable(
      focused: !engaged,
      onKeyEvent: _handleGarageKey,
      child: Column(
        children: [
          for (var row = 0; row < 3; row++)
            Expanded(
              child: Row(
                children: [
                  for (var col = 0; col < 3; col++)
                    Expanded(child: _tile(row * 3 + col)),
                ],
              ),
            ),
          Container(
            color: Colors.black,
            child: Text(
              engaged
                  ? ' keys -> garage-spike-${focus + 1} · ctrl+q back · last: $lastKey '
                  : ' keys -> garage · 1-9 focus · enter engage · q quit · last: $lastKey ',
              style: TextStyle(
                color: engaged ? Colors.yellow : Colors.gray,
              ),
            ),
          ),
        ],
      ),
    );
  }

  Component _tile(int i) {
    final isFocused = focus == i;
    final isEngaged = engaged && isFocused;
    return Column(
      children: [
        Container(
          color: isEngaged
              ? Colors.yellow
              : (isFocused ? Colors.white : Colors.grey),
          child: Text(
            ' ${i + 1} garage-spike-${i + 1}${isEngaged ? " [engaged]" : ""} ',
            style: TextStyle(
              color: isEngaged || isFocused ? Colors.black : Colors.gray,
            ),
          ),
        ),
        Expanded(
          child: TerminalXterm(
            controller: controllers[i],
            focused: isEngaged,
            onKeyEvent: (e) => _handleEngagedKey(i, e),
          ),
        ),
      ],
    );
  }
}

/// Re-encode a parsed KeyboardEvent back to the raw byte sequence a real
/// terminal would have sent. This is the verbatim-passthrough encoder —
/// TerminalXterm's built-in translation drops modifiers (the Alt+arrow bug).
String? encodeKey(KeyboardEvent e) {
  final m = e.modifiers;
  final mod = 1 +
      (m.shift ? 1 : 0) +
      (m.alt ? 2 : 0) +
      (m.ctrl ? 4 : 0) +
      (m.meta ? 8 : 0);
  String csi(String fin) =>
      m.hasAnyModifier ? '\x1b[1;$mod$fin' : '\x1b[$fin';
  String tilde(int n) =>
      m.hasAnyModifier ? '\x1b[$n;$mod~' : '\x1b[$n~';

  switch (e.logicalKey) {
    case LogicalKey.arrowUp:
      return csi('A');
    case LogicalKey.arrowDown:
      return csi('B');
    case LogicalKey.arrowRight:
      return csi('C');
    case LogicalKey.arrowLeft:
      return csi('D');
    case LogicalKey.home:
      return csi('H');
    case LogicalKey.end:
      return csi('F');
    case LogicalKey.enter:
      return m.alt ? '\x1b\r' : '\r';
    case LogicalKey.tab:
      return m.shift ? '\x1b[Z' : '\t';
    case LogicalKey.backspace:
      return m.alt ? '\x1b\x7f' : '\x7f';
    case LogicalKey.escape:
      return '\x1b';
    case LogicalKey.pageUp:
      return tilde(5);
    case LogicalKey.pageDown:
      return tilde(6);
    case LogicalKey.delete:
      return tilde(3);
    case LogicalKey.insert:
      return tilde(2);
    default:
      break;
  }

  final ch = e.character;
  if (ch == null || ch.isEmpty) return null;
  var s = ch;
  if (m.ctrl && ch.length == 1) {
    final c = ch.toLowerCase().codeUnitAt(0);
    if (c >= 0x61 && c <= 0x7a) s = String.fromCharCode(c - 0x60);
  }
  if (m.alt) s = '\x1b$s';
  return s;
}
