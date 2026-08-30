import 'dart:async';
import 'dart:convert';
import 'dart:io';
import 'package:meta/meta.dart';

/// Internal PTY handler that manages a subprocess with pseudo-terminal support.
/// This is used internally by [PtyController] and should not be used directly.
///
/// This class handles the low-level process management and terminal emulation,
/// while [PtyController] provides the high-level API following Flutter patterns.
@internal
class PtyHandler {
  final String command;
  final List<String> arguments;
  final String? workingDirectory;
  final Map<String, String>? environment;

  Process? _process;
  StreamController<String>? _outputController;
  StreamSubscription? _stdoutSubscription;
  StreamSubscription? _stderrSubscription;

  int? get pid => _process?.pid;

  PtyHandler({
    required this.command,
    this.arguments = const [],
    this.workingDirectory,
    this.environment,
  });

  /// Start the process with pseudo-terminal support
  Future<void> start({
    required int columns,
    required int rows,
  }) async {
    _outputController = StreamController<String>.broadcast();

    // Build environment with terminal settings
    final effectiveEnv = <String, String>{};

    // Set terminal environment variables
    effectiveEnv['TERM'] = 'xterm-256color';
    effectiveEnv['LANG'] = 'en_US.UTF-8';
    effectiveEnv['LINES'] = rows.toString();
    effectiveEnv['COLUMNS'] = columns.toString();

    // Set our own terminal program identifier
    effectiveEnv['TERM_PROGRAM'] = 'DartTUI';

    // Copy important environment variables from parent process
    const envValuesToCopy = {
      'LOGNAME',
      'USER',
      'DISPLAY',
      'LC_TYPE',
      'HOME',
      'PATH',
      'SHELL',
    };

    // Variables to explicitly exclude (shell integrations)
    const envVariablesToExclude = {
      'WARP_TERMINAL',
      'WARP_BOOTSTRAPPED',
      'WARP_IS_LOCAL_SHELL_SESSION',
      'WARP_USE_SSH_WRAPPER',
      'FIG_TERM',
      'FIG_ENV_VAR',
      'ITERM_SHELL_INTEGRATION_INSTALLED',
      'VSCODE_SHELL_INTEGRATION',
      'INSIDE_EMACS',
      'STARSHIP_SHELL',
    };

    for (var entry in Platform.environment.entries) {
      if (envValuesToCopy.contains(entry.key) &&
          !envVariablesToExclude.contains(entry.key)) {
        effectiveEnv[entry.key] = entry.value;
      }
    }

    // Add user-provided environment variables
    if (environment != null) {
      effectiveEnv.addAll(environment!);
    }

    // Start the process with PTY mode
    _process = await Process.start(
      command,
      arguments,
      workingDirectory: workingDirectory,
      environment: effectiveEnv,
      mode: ProcessStartMode.normal,
      runInShell: false, // We handle shell directly
      // Note: On Unix systems, we need to use PTY mode
      // This is platform-specific and might need adjustment
    );

    // Set up output handling with UTF-8 decoding
    _stdoutSubscription = _process!.stdout
        .transform(const Utf8Decoder(allowMalformed: true))
        .listen((data) {
      _outputController?.add(data);
    });

    _stderrSubscription = _process!.stderr
        .transform(const Utf8Decoder(allowMalformed: true))
        .listen((data) {
      _outputController?.add(data);
    });
  }

  /// Get the output stream from the process
  Stream<String> get output {
    if (_outputController == null) {
      throw StateError('Process not started');
    }
    return _outputController!.stream;
  }

  /// Get the exit code of the process
  Future<int> get exitCode async {
    if (_process == null) {
      throw StateError('Process not started');
    }
    return await _process!.exitCode;
  }

  /// Write data to the process stdin
  void write(String data) {
    if (_process == null) {
      throw StateError('Process not started');
    }
    _process!.stdin.write(data);
    _process!.stdin.flush();
  }

  /// Write raw bytes to the process stdin
  void writeBytes(List<int> bytes) {
    if (_process == null) {
      throw StateError('Process not started');
    }
    _process!.stdin.add(bytes);
    _process!.stdin.flush();
  }

  /// Resize the terminal (send window size change signal)
  void resize(int rows, int columns) {
    // On Unix systems, we would send SIGWINCH signal
    // For now, we'll update environment variables and send escape sequence
    if (_process != null) {
      // Send escape sequence to notify terminal of size change
      final resizeSequence = '\x1b[8;$rows;${columns}t';
      write(resizeSequence);
    }
  }

  /// Kill the process
  bool kill([ProcessSignal signal = ProcessSignal.sigterm]) {
    if (_process == null) return false;
    return _process!.kill(signal);
  }

  /// Dispose of resources
  Future<void> dispose() async {
    await _stdoutSubscription?.cancel();
    await _stderrSubscription?.cancel();
    await _outputController?.close();

    // Try to gracefully terminate the process
    if (_process != null) {
      _process!.kill(ProcessSignal.sigterm);

      // Wait a bit for graceful shutdown
      await Future.delayed(const Duration(milliseconds: 100));

      // Force kill if still running
      _process!.kill(ProcessSignal.sigkill);
    }
  }
}

/// Platform-specific PTY handler that uses actual PTY on Unix systems.
/// Internal implementation detail of the PTY system.
@internal
class UnixPtyHandler extends PtyHandler {
  UnixPtyHandler({
    required super.command,
    super.arguments,
    super.workingDirectory,
    super.environment,
  });

  @override
  Future<void> start({
    required int columns,
    required int rows,
  }) async {
    // PATCHED (garage): size propagation (patch 8) — a new process means a
    // new pty; drop the cached tty path.
    _resizeTty = null;
    _resizeRetry?.cancel();
    _outputController = StreamController<String>.broadcast();

    // Build environment with terminal settings
    final effectiveEnv = <String, String>{};

    // Set terminal environment variables
    effectiveEnv['TERM'] = 'xterm-256color';
    effectiveEnv['LANG'] = 'en_US.UTF-8';
    effectiveEnv['LINES'] = rows.toString();
    effectiveEnv['COLUMNS'] = columns.toString();

    // Set our own terminal program identifier
    effectiveEnv['TERM_PROGRAM'] = 'DartTUI';

    // Copy important environment variables
    const envValuesToCopy = {
      'LOGNAME',
      'USER',
      'DISPLAY',
      'LC_TYPE',
      'HOME',
      'PATH',
      'SHELL',
    };

    for (var entry in Platform.environment.entries) {
      if (envValuesToCopy.contains(entry.key)) {
        effectiveEnv[entry.key] = entry.value;
      }
    }

    // Add user-provided environment variables
    if (environment != null) {
      effectiveEnv.addAll(environment!);
    }

    // On Unix systems (macOS, Linux), we can use 'script' command to allocate a PTY
    // This is a workaround since Dart doesn't directly support PTY allocation
    final scriptCommand = Platform.isMacOS ? 'script' : 'script';
    final scriptArgs = <String>[];

    if (Platform.isMacOS) {
      // macOS script command syntax
      scriptArgs.addAll(['-q', '/dev/null', command, ...arguments]);
    } else {
      // Linux script command syntax
      scriptArgs
          .addAll(['-q', '-c', '$command ${arguments.join(' ')}', '/dev/null']);
    }

    // Start the process wrapped in 'script' to get PTY
    _process = await Process.start(
      scriptCommand,
      scriptArgs,
      workingDirectory: workingDirectory,
      environment: effectiveEnv,
      mode: ProcessStartMode.normal,
      runInShell: false,
    );

    // Set up output handling
    _stdoutSubscription = _process!.stdout
        .transform(const Utf8Decoder(allowMalformed: true))
        .listen((data) {
      _outputController?.add(data);
    });

    _stderrSubscription = _process!.stderr
        .transform(const Utf8Decoder(allowMalformed: true))
        .listen((data) {
      _outputController?.add(data);
    });
  }

  // PATCHED (garage): size propagation — see patch 8 in
  // tui/vendor/NOCTERM_VERSION. Cached controlling-tty path of the child
  // running on the script-allocated pty (resolved once per process), the
  // last requested winsize, and the retry timer for a resize that arrived
  // before the child had forked onto the pty.
  String? _resizeTty;
  int? _wantRows;
  int? _wantColumns;
  Timer? _resizeRetry;
  int _resizeAttempts = 0;

  @override
  void resize(int rows, int columns) {
    if (_process == null || !(Platform.isLinux || Platform.isMacOS)) return;
    // PATCHED (garage): size propagation (patch 8) — perform a REAL
    // TIOCSWINSZ on the script-allocated pty instead of upstream's
    // approach (writing an in-band `ESC[8;rows;colst` report into the
    // child's stdin — which tmux consumes only as the answer to its own
    // startup `CSI 18 t` query and otherwise forwards into the pane as
    // typed junk — plus a `kill -WINCH` at a process group whose winsize
    // had never actually changed). The child (e.g. `tmux attach`) sits on
    // the new pty; `pgrep -P` + `ps -o tty=` resolve its tty, stty sets
    // the winsize, and the kernel delivers SIGWINCH itself. A resize
    // issued before the child has forked retries briefly with the LAST
    // requested size.
    _wantRows = rows;
    _wantColumns = columns;
    _resizeAttempts = 0;
    _resizeRetry?.cancel();
    _applyWinsize();
  }

  void _applyWinsize() {
    final rows = _wantRows;
    final columns = _wantColumns;
    if (_process == null || rows == null || columns == null) return;
    var applied = false;
    try {
      var tty = _resizeTty;
      if (tty == null) {
        final child = Process.runSync('pgrep', ['-P', '${_process!.pid}'])
            .stdout
            .toString()
            .trim()
            .split('\n')
            .first
            .trim();
        if (child.isNotEmpty) {
          final name = Process.runSync('ps', ['-o', 'tty=', '-p', child])
              .stdout
              .toString()
              .trim();
          if (name.isNotEmpty && !name.startsWith('?')) {
            tty = name.startsWith('/dev/') ? name : '/dev/$name';
            _resizeTty = tty;
          }
        }
      }
      if (tty != null) {
        final flag = Platform.isMacOS ? '-f' : '-F';
        final r = Process.runSync(
            'stty', [flag, tty, 'rows', '$rows', 'columns', '$columns']);
        applied = r.exitCode == 0;
      }
    } catch (_) {
      // Fall through to the retry below.
    }
    if (!applied && _resizeAttempts < 10) {
      _resizeAttempts++;
      _resizeRetry = Timer(const Duration(milliseconds: 200), _applyWinsize);
    }
  }

  @override
  Future<void> dispose() async {
    _resizeRetry?.cancel();
    await super.dispose();
  }
}

/// Factory to create the appropriate PTY handler for the platform.
/// Used internally by [PtyController].
@internal
class PtyHandlerFactory {
  static PtyHandler create({
    required String command,
    List<String> arguments = const [],
    String? workingDirectory,
    Map<String, String>? environment,
  }) {
    if (Platform.isWindows) {
      // On Windows, use the basic handler (no true PTY support)
      return PtyHandler(
        command: command,
        arguments: arguments,
        workingDirectory: workingDirectory,
        environment: environment,
      );
    } else {
      // On Unix systems, use the Unix PTY handler
      return UnixPtyHandler(
        command: command,
        arguments: arguments,
        workingDirectory: workingDirectory,
        environment: environment,
      );
    }
  }
}
