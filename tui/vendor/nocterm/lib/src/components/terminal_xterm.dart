import 'dart:async';
import 'dart:io';
import 'dart:math' as math;

import 'package:nocterm/nocterm.dart';
import 'package:nocterm/src/third_party/xterm_pure.dart/xterm.dart' as xterm;
import '../process/pty_controller.dart' as pty;

/// A terminal component using the xterm.dart library for proper terminal emulation.
///
/// This component requires a [PtyController] following the same pattern as Flutter's
/// TextField with TextEditingController.
class TerminalXterm extends StatefulComponent {
  /// The controller that manages the PTY process.
  final pty.PtyController controller;

  /// Whether the terminal is focused.
  final bool focused;

  /// Custom key event handler.
  /// Return true to consume the event and prevent default handling.
  final bool Function(KeyboardEvent)? onKeyEvent;

  /// Maximum number of lines in the terminal buffer.
  final int maxLines;

  /// Whether to auto-start the terminal if not already running.
  final bool autoStart;

  // PATCHED (garage): absolute scroll anchor — see patch 6 in
  // tui/vendor/NOCTERM_VERSION. Upstream's private relative _scrollOffset
  // cannot express a frozen history view (live output drags it along), so
  // the embedding app supplies an ABSOLUTE first-visible-line index and
  // reads back the buffer geometry it needs to compute one.

  /// Absolute buffer index of the first rendered line. When non-null the
  /// renderer slices the buffer at exactly this line (clamped to the buffer)
  /// regardless of new output, and the cursor is not drawn. Null = upstream
  /// behavior (follow the tail with the relative scroll offset).
  final int? frozenTopLine;

  /// Reports the active buffer's geometry on every build:
  /// (totalLines, droppedLines, viewHeight). droppedLines is the monotonic
  /// count of lines trimmed off the buffer front (circular-buffer overflow).
  final void Function(int totalLines, int droppedLines, int viewHeight)?
      onBufferMetrics;

  /// Written into the terminal once at initState, before any PTY output —
  /// used to seed scrollback history (e.g. from tmux capture-pane).
  final String? initialContent;

  /// When true the emulator ignores alt-screen switches (modes
  /// 47/1047/1049): everything renders into the main buffer so scrolled
  /// lines accumulate as scrollback — required for scrollback over
  /// `tmux attach`, which otherwise parks the client in the alt buffer.
  final bool ignoreAltBuffer;

  /// PATCHED (garage): size propagation (patch 8) — when true, `CSI 18 t`
  /// size queries are not answered. Default false, and MUST stay false for
  /// tmux-attach tiles: the PTY here is pipe-based (script wrapper, stdin
  /// not a tty), so the `CSI 18 t` → `CSI 8;rows;cols t` handshake is how
  /// tmux learns the client size at attach — suppressing it leaves the
  /// client sizeless and the attach exits.
  final bool suppressSizeReports;

  /// PATCHED (garage): engaged cursor (patch 9) — when true the renderer
  /// paints the inner terminal's cursor cell in inverse video (solid block,
  /// no blink). Gated on the emulator's DECTCEM cursor-visible state
  /// (`CSI ?25l` hides it) and suppressed while a frozen history view is
  /// active ([frozenTopLine] non-null) or a relative scroll is applied.
  /// Default false: upstream never painted the cursor at all (see
  /// _convertCellStyle), and an unengaged wall of tiles must not show one
  /// caret per tile.
  final bool showCursor;

  const TerminalXterm({
    super.key,
    required this.controller,
    this.focused = false,
    this.onKeyEvent,
    this.maxLines = 10000,
    this.autoStart = true,
    this.frozenTopLine,
    this.onBufferMetrics,
    this.initialContent,
    this.ignoreAltBuffer = false,
    this.suppressSizeReports = false,
    this.showCursor = false,
  });

  @override
  State<TerminalXterm> createState() => _TerminalXtermState();
}

class _TerminalXtermState extends State<TerminalXterm> {
  late final xterm.Terminal _terminal;
  VoidCallback? _controllerListener;

  // PATCHED (garage): output-callback leak — see patch 5 in
  // tui/vendor/NOCTERM_VERSION. Upstream registers an anonymous output
  // callback it can never remove: it survives unmount and controller swaps,
  // and its setState on a defunct element throws, aborting the controller's
  // callback fan-out loop and starving every later-registered callback.
  void Function(String)? _outputCallback;
  bool _defunct = false;

  // Terminal dimensions
  int _rows = 24;
  int _cols = 80;

  // Scrolling support
  int _scrollOffset = 0;

  @override
  void initState() {
    super.initState();

    // Create the xterm terminal
    _terminal = xterm.Terminal(
      maxLines: component.maxLines,
      platform: _getPlatform(),
    );

    // Initialize terminal size
    _terminal.resize(_cols, _rows);

    // PATCHED (garage): absolute scroll anchor (patch 6) — main-buffer-only
    // mode so tmux-attach output accumulates as scrollback.
    _terminal.ignoreAltBuffer = component.ignoreAltBuffer;

    // PATCHED (garage): size propagation (patch 8) — see the field doc.
    _terminal.suppressSizeReports = component.suppressSizeReports;

    // PATCHED (garage): absolute scroll anchor (patch 6) — seed history
    // before any PTY output arrives.
    final seed = component.initialContent;
    if (seed != null && seed.isNotEmpty) {
      _terminal.write(seed);
    }

    // Set up terminal callbacks
    _terminal.onOutput = (data) {
      if (component.controller.isRunning) {
        component.controller.write(data);
      }
    };

    _terminal.onResize = (width, height, _, __) {
      if (component.controller.isRunning) {
        component.controller.resize(width, height);
      }
    };

    _terminal.onTitleChange = (title) {
      // Could update the window title if needed
    };

    _terminal.onBell = () {
      // Terminal bell - could play a sound or flash
    };

    // Set up controller output handler
    _setupControllerHandler();

    // Listen to controller changes
    _controllerListener = _onControllerChanged;
    component.controller.addListener(_controllerListener!);

    // Auto-start if requested
    if (component.autoStart && !component.controller.isRunning) {
      _startTerminal();
    }
  }

  void _setupControllerHandler() {
    // PATCHED (garage): output-callback leak — keep a reference so the
    // callback can be removed on dispose/controller change, and never touch
    // a defunct element.
    _outputCallback = (data) {
      if (_defunct) return;
      _terminal.write(data);
      setState(() {
        // Trigger rebuild when terminal updates
      });
    };
    component.controller.addOutputCallback(_outputCallback!);
  }

  void _onControllerChanged() {
    // PATCHED (garage): size propagation (patch 8) — a PTY that is not
    // running cannot hold our size (a restart respawns at the start()
    // default), so mark dirty while down and push the tracked size on the
    // transition to running. Dirty clears BEFORE the resize call: resize
    // notifies listeners, and the re-entrant call must be a no-op.
    _rlog('controllerChanged running=${component.controller.isRunning} dirty=$_ptyResizeDirty size=$_cols x $_rows');
    if (!component.controller.isRunning) {
      _ptyResizeDirty = true;
    } else if (_ptyResizeDirty) {
      _ptyResizeDirty = false;
      _terminal.resize(_cols, _rows);
      component.controller.resize(_cols, _rows);
    }
    // Handle controller state changes
    setState(() {});
  }

  xterm.TerminalTargetPlatform _getPlatform() {
    if (Platform.isMacOS) return xterm.TerminalTargetPlatform.macos;
    if (Platform.isLinux) return xterm.TerminalTargetPlatform.linux;
    if (Platform.isWindows) return xterm.TerminalTargetPlatform.windows;
    return xterm.TerminalTargetPlatform.unknown;
  }

  void _startTerminal() async {
    try {
      await component.controller.start(columns: _cols, rows: _rows);
    } catch (e) {
      // Handle error
      _terminal.write('\r\nError starting terminal: $e\r\n');
    }
  }

  void _handleKeyEvent(KeyboardEvent event) {
    if (!component.controller.isRunning) return;

    // Call parent's key handler first, if provided
    if (component.onKeyEvent != null) {
      final handled = component.onKeyEvent!(event);
      if (handled) return; // Parent consumed the event
    }

    // Handle special keys using xterm's key input
    if (event.logicalKey == LogicalKey.enter) {
      _terminal.keyInput(xterm.TerminalKey.enter);
    } else if (event.logicalKey == LogicalKey.backspace) {
      _terminal.keyInput(xterm.TerminalKey.backspace);
    } else if (event.logicalKey == LogicalKey.tab) {
      _terminal.keyInput(xterm.TerminalKey.tab);
    } else if (event.logicalKey == LogicalKey.escape) {
      _terminal.keyInput(xterm.TerminalKey.escape);
    } else if (event.logicalKey == LogicalKey.arrowUp) {
      _terminal.keyInput(xterm.TerminalKey.arrowUp);
    } else if (event.logicalKey == LogicalKey.arrowDown) {
      _terminal.keyInput(xterm.TerminalKey.arrowDown);
    } else if (event.logicalKey == LogicalKey.arrowRight) {
      _terminal.keyInput(xterm.TerminalKey.arrowRight);
    } else if (event.logicalKey == LogicalKey.arrowLeft) {
      _terminal.keyInput(xterm.TerminalKey.arrowLeft);
    } else if (event.logicalKey == LogicalKey.home) {
      _terminal.keyInput(xterm.TerminalKey.home);
    } else if (event.logicalKey == LogicalKey.end) {
      _terminal.keyInput(xterm.TerminalKey.end);
    } else if (event.logicalKey == LogicalKey.pageUp) {
      _scrollUp(5);
    } else if (event.logicalKey == LogicalKey.pageDown) {
      _scrollDown(5);
    } else if (event.logicalKey == LogicalKey.delete) {
      _terminal.keyInput(xterm.TerminalKey.delete);
    } else if (event.logicalKey == LogicalKey.insert) {
      _terminal.keyInput(xterm.TerminalKey.insert);
    } else if (event.character != null && event.character!.isNotEmpty) {
      // Handle control characters
      final charCode = event.character!.codeUnitAt(0);
      if (charCode >= 1 && charCode <= 26) {
        // Control character (Ctrl+A through Ctrl+Z)
        _terminal.charInput(charCode);
      } else {
        // Regular text input
        _terminal.textInput(event.character!);
      }
    }
  }

  void _scrollUp(int lines) {
    final maxScroll = _terminal.buffer.lines.length - _terminal.viewHeight;
    setState(() {
      _scrollOffset = (_scrollOffset - lines).clamp(-maxScroll, 0);
    });
  }

  void _scrollDown(int lines) {
    final maxScroll = _terminal.buffer.lines.length - _terminal.viewHeight;
    setState(() {
      _scrollOffset = (_scrollOffset + lines).clamp(-maxScroll, 0);
    });
  }

  // PATCHED (garage): size propagation — see patch 8 in
  // tui/vendor/NOCTERM_VERSION. Upstream dropped tile→PTY sizing twice
  // over: the renderer reported a hardcoded 80×24 (so _updateSize never saw
  // a real size), and PtyController.resize silently no-ops before the
  // process is running (the embedding app starts controllers *after* first
  // layout), so the PTY stayed at the spawn default forever.
  //
  // The fix: _updateSize records the layout size and schedules the apply in
  // a microtask (it is called from build/layout, where Terminal.resize's
  // onResize → controller.resize → notifyListeners → setState chain must
  // not run); a resize that could not reach a non-running PTY stays dirty
  // and is re-applied on the controller's transition to running (covers the
  // deferred first attach AND restarts, which respawn at the start()
  // default size).
  bool _ptyResizeDirty = false;

  // Debug hook (env-gated, zero cost when unset): append resize-path events
  // to the file named by GARAGE_TUI_RESIZELOG.
  static final String? _resizeLogPath =
      Platform.environment['GARAGE_TUI_RESIZELOG'];
  void _rlog(String msg) {
    final path = _resizeLogPath;
    if (path == null) return;
    File(path).writeAsStringSync(
        '${DateTime.now().toIso8601String()} [$hashCode] $msg\n',
        mode: FileMode.append);
  }

  void _updateSize(int cols, int rows) {
    if (cols == _cols && rows == _rows) return;
    _rlog('updateSize $cols x $rows (was $_cols x $_rows), running=${component.controller.isRunning}');
    _cols = cols;
    _rows = rows;
    _ptyResizeDirty = true;
    scheduleMicrotask(_applyPendingResize);
  }

  void _applyPendingResize() {
    _rlog('applyPending dirty=$_ptyResizeDirty defunct=$_defunct running=${component.controller.isRunning} -> $_cols x $_rows');
    if (_defunct || !_ptyResizeDirty) return;
    // Terminal.resize fires onResize, which already forwards to a running
    // controller — but keep the explicit resize so the dirty flag only
    // clears when the PTY could actually take the size.
    _terminal.resize(_cols, _rows);
    if (component.controller.isRunning) {
      _ptyResizeDirty = false;
      component.controller.resize(_cols, _rows);
    }
    setState(() {}); // re-render with the resized buffer
  }

  @override
  void dispose() {
    // PATCHED (garage): output-callback leak — detach before unmount.
    _defunct = true;
    if (_outputCallback != null) {
      component.controller.removeOutputCallback(_outputCallback!);
    }
    if (_controllerListener != null) {
      component.controller.removeListener(_controllerListener!);
    }
    super.dispose();
  }

  @override
  void didUpdateComponent(TerminalXterm oldComponent) {
    super.didUpdateComponent(oldComponent);

    // Handle controller change
    if (oldComponent.controller != component.controller) {
      if (_controllerListener != null) {
        oldComponent.controller.removeListener(_controllerListener!);
      }
      // PATCHED (garage): output-callback leak — detach from the old
      // controller before registering on the new one.
      if (_outputCallback != null) {
        oldComponent.controller.removeOutputCallback(_outputCallback!);
      }
      _controllerListener = _onControllerChanged;
      component.controller.addListener(_controllerListener!);
      _setupControllerHandler();
    }
  }

  @override
  Component build(BuildContext context) {
    // PATCHED (garage): absolute scroll anchor (patch 6) — report the
    // active buffer geometry so the embedding app can maintain an absolute
    // anchor (this component rebuilds on every output chunk, so the report
    // tracks the buffer closely).
    final metrics = component.onBufferMetrics;
    if (metrics != null) {
      final lines = _terminal.buffer.lines;
      metrics(lines.length, lines.droppedLines, _terminal.viewHeight);
    }
    return Focusable(
      focused: component.focused,
      onKeyEvent: (event) {
        _handleKeyEvent(event);
        return true; // Consume all key events
      },
      child: _TerminalRenderer(
        terminal: _terminal,
        scrollOffset: _scrollOffset,
        // PATCHED (garage): absolute scroll anchor (patch 6).
        frozenTopLine: component.frozenTopLine,
        onSizeChange: _updateSize,
        // PATCHED (garage): engaged cursor (patch 9).
        showCursor: component.showCursor,
      ),
    );
  }
}

/// Terminal renderer that converts xterm buffer to TUI components
class _TerminalRenderer extends StatelessComponent {
  final xterm.Terminal terminal;
  final int scrollOffset;

  // PATCHED (garage): absolute scroll anchor (patch 6) — when non-null the
  // slice starts at exactly this buffer line (clamped), so new output can
  // never move the visible window, and the cursor is suppressed.
  final int? frozenTopLine;

  final void Function(int cols, int rows)? onSizeChange;

  // PATCHED (garage): engaged cursor (patch 9) — see the TerminalXterm
  // field doc. The renderer inverts the cell at the emulator's cursor
  // position when set (and DECTCEM allows, and the view is live).
  final bool showCursor;

  const _TerminalRenderer({
    required this.terminal,
    required this.scrollOffset,
    this.frozenTopLine,
    this.onSizeChange,
    this.showCursor = false,
  });

  @override
  Component build(BuildContext context) {
    // PATCHED (garage): size propagation (patch 8) — upstream rendered a
    // hardcoded 80×24 "for now"; the real size comes from the incoming
    // constraints, reported through onSizeChange so the embedding state can
    // resize the emulator and the PTY (deferred — see _updateSize).
    return LayoutBuilder(builder: (context, constraints) {
      final cols = constraints.maxWidth.isFinite
          ? math.max(1, constraints.maxWidth.floor())
          : 80;
      final rows = constraints.maxHeight.isFinite
          ? math.max(1, constraints.maxHeight.floor())
          : 24;

      // Notify size change
      onSizeChange?.call(cols, rows);

      return _buildLines(cols, rows);
    });
  }

  Component _buildLines(int cols, int rows) {
    final lines = <Component>[];

    // Calculate visible range considering scrollback
    final buffer = terminal.buffer;
    final totalLines = buffer.lines.length;
    final viewHeight = rows;

    // Determine which lines to display.
    // PATCHED (garage): absolute scroll anchor (patch 6) — a frozen view
    // slices from the given absolute line; upstream's relative math only
    // runs while live.
    final frozen = frozenTopLine;
    final startLine = frozen != null
        ? math.max(0, math.min(frozen, totalLines - viewHeight))
        : math.max(0, totalLines - viewHeight + scrollOffset);

    // PATCHED (garage): engaged cursor (patch 9) — paint the cursor only
    // when the embedder asks (showCursor), the view is live (not frozen —
    // patch 6 — and not relatively scrolled), and the application has not
    // hidden it via DECTCEM (`CSI ?25l`). Upstream compared lineIndex (an
    // absolute buffer index) to the view-relative cursorY, which is wrong
    // as soon as scrollback exists — absoluteCursorY is the buffer-space
    // cursor line.
    final cursorAllowed = showCursor &&
        frozen == null &&
        scrollOffset == 0 &&
        terminal.cursorVisibleMode;

    // Render terminal buffer lines
    for (int y = 0; y < viewHeight; y++) {
      final lineIndex = startLine + y;

      if (lineIndex >= 0 && lineIndex < totalLines) {
        final line = buffer.lines[lineIndex];
        final hasCursor =
            cursorAllowed && lineIndex == buffer.absoluteCursorY;
        lines.add(_renderLine(line, hasCursor, buffer.cursorX, cols));
      } else {
        // Empty line
        lines.add(Text(' ' * cols));
      }
    }

    return Column(
      crossAxisAlignment: CrossAxisAlignment.start,
      children: lines,
    );
  }

  Component _renderLine(
      xterm.BufferLine line, bool hasCursor, int cursorX, int cols) {
    final spans = <_StyledSpan>[];
    final lineLength = line.length;
    final cellData = xterm.CellData.empty();

    // PATCHED (garage): engaged cursor (patch 9) — the inverse-video style
    // for the cursor cell. Also applied when the cursor sits on an
    // empty/padding cell (the common case at a composer's end-of-input):
    // upstream only styled the cursor on occupied cells, and then dropped
    // the style anyway (see _convertCellStyle).
    const cursorStyle = TextStyle(reverse: true);

    // Merge consecutive spans with the same style.
    void addSpan(String char, TextStyle style) {
      if (spans.isNotEmpty && spans.last.style == style) {
        spans.last.text += char;
      } else {
        spans.add(_StyledSpan(char, style));
      }
    }

    // PATCHED (garage): size propagation (patch 8) — pad/iterate to the
    // laid-out width instead of the hardcoded 80.
    for (int x = 0; x < cols; x++) {
      final isCursor = hasCursor && x == cursorX;
      if (x < lineLength) {
        line.getCellData(x, cellData);
        final codePoint = cellData.content & xterm.CellContent.codepointMask;
        if (codePoint != 0) {
          addSpan(String.fromCharCode(codePoint),
              _convertCellStyle(cellData, isCursor));
        } else {
          // Empty cell
          addSpan(' ', isCursor ? cursorStyle : const TextStyle());
        }
      } else {
        // Padding
        addSpan(' ', isCursor ? cursorStyle : const TextStyle());
      }
    }

    // Combine all spans into a single text
    if (spans.isEmpty) {
      return Text(' ' * cols);
    } else if (spans.length == 1) {
      return Text(spans[0].text, style: spans[0].style);
    } else if (hasCursor) {
      // PATCHED (garage): engaged cursor (patch 9) — the upstream fallback
      // below flattens a multi-style line into one style, which would erase
      // the single inverted cursor cell. The cursor line renders its real
      // spans through RichText instead (per-span styles, no wrap). Other
      // lines keep the upstream collapse untouched.
      return RichText(
        text: TextSpan(children: [
          for (final span in spans)
            TextSpan(text: span.text, style: span.style),
        ]),
        softWrap: false,
      );
    } else {
      // For multiple styles, we need to combine them
      // For now, just use the first style for the whole line
      final buffer = StringBuffer();
      TextStyle? primaryStyle;

      for (final span in spans) {
        buffer.write(span.text);
        primaryStyle ??= span.style;
      }

      return Text(buffer.toString(), style: primaryStyle ?? const TextStyle());
    }
  }

  TextStyle _convertCellStyle(xterm.CellData cell, bool isCursor) {
    Color? fg;
    Color? bg;
    bool bold = false;
    bool dim = false;
    bool italic = false;
    bool underline = false;

    // Extract attributes from flags
    final flags = cell.flags;
    if (flags & xterm.CellAttr.bold != 0) bold = true;
    if (flags & xterm.CellAttr.faint != 0) dim = true;
    if (flags & xterm.CellAttr.italic != 0) italic = true;
    if (flags & xterm.CellAttr.underline != 0) underline = true;

    // Convert colors
    fg = _convertColor(cell.foreground);
    bg = _convertColor(cell.background);

    // PATCHED (garage): engaged cursor (patch 9) — nocterm's TextStyle DOES
    // support reverse (SGR 7; the upstream "not supported" note here was
    // stale), so the cursor cell paints as inverse video. Content-level
    // inverse (CellAttr.inverse) deliberately stays unpainted, exactly as
    // upstream: the whole-line style collapse in _renderLine would let one
    // inverse-attr cell flip an entire line.
    return TextStyle(
      color: fg,
      backgroundColor: bg,
      fontWeight: bold ? FontWeight.bold : (dim ? FontWeight.dim : null),
      fontStyle: italic ? FontStyle.italic : null,
      decoration: underline ? TextDecoration.underline : null,
      reverse: isCursor,
    );
  }

  Color? _convertColor(int color) {
    if (color == 0) return null;

    final colorType = color & xterm.CellColor.typeMask;
    final colorValue = color & xterm.CellColor.valueMask;

    if (colorType == xterm.CellColor.rgb) {
      // RGB color
      final r = (colorValue >> 16) & 0xFF;
      final g = (colorValue >> 8) & 0xFF;
      final b = colorValue & 0xFF;
      return Color.fromRGB(r, g, b);
    } else if (colorType == xterm.CellColor.palette ||
        colorType == xterm.CellColor.named) {
      // Palette color (0-255)
      return _getPaletteColor(colorValue);
    }

    return null;
  }

  Color _getPaletteColor(int index) {
    // ANSI 16 colors
    switch (index) {
      case 0:
        return Colors.black;
      case 1:
        return Colors.red;
      case 2:
        return Colors.green;
      case 3:
        return Colors.yellow;
      case 4:
        return Colors.blue;
      case 5:
        return Colors.magenta;
      case 6:
        return Colors.cyan;
      case 7:
        return Colors.white;
      case 8:
        return Colors.brightBlack;
      case 9:
        return Colors.brightRed;
      case 10:
        return Colors.brightGreen;
      case 11:
        return Colors.brightYellow;
      case 12:
        return Colors.brightBlue;
      case 13:
        return Colors.brightMagenta;
      case 14:
        return Colors.brightCyan;
      case 15:
        return Colors.brightWhite;
      default:
        // 256 color palette
        if (index < 232) {
          // 216 color cube
          final i = index - 16;
          final r = (i ~/ 36) * 51;
          final g = ((i ~/ 6) % 6) * 51;
          final b = (i % 6) * 51;
          return Color.fromRGB(r, g, b);
        } else {
          // Grayscale
          final gray = 8 + (index - 232) * 10;
          return Color.fromRGB(gray, gray, gray);
        }
    }
  }
}

/// Helper class for styled text spans
class _StyledSpan {
  String text;
  final TextStyle style;

  _StyledSpan(this.text, this.style);

  @override
  bool operator ==(Object other) {
    if (identical(this, other)) return true;
    return other is _StyledSpan && other.style == style;
  }

  @override
  int get hashCode => style.hashCode;
}
