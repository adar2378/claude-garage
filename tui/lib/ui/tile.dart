/// One grid tile: an embedded terminal attached to a tmux session, with a
/// border-title status bar, the engaged-layer key path, and the frozen
/// local-history view (spec tui-scrollback).
///
/// Key routing (spec tui-key-routing): while engaged, EVERY key event is
/// consumed here — Ctrl+G disengages, synthetic Ctrl+V recovers a paste,
/// Shift+PageUp/Down drives the frozen scrollback, and everything else
/// re-encodes through [encodeKey] into the PTY. Nothing ever falls through
/// to TerminalXterm's lossy default handling (it drops modifiers).
///
/// Scrollback (spec tui-scrollback): the history view is LOCAL — an
/// absolute anchor over the xterm buffer ([ScrollAnchor], rendered via the
/// vendored TerminalXterm's `frozenTopLine`, patch 6), never tmux
/// copy-mode, never TerminalXterm's relative scroll. Any key forwarded to
/// the PTY snaps back to live first; End snaps too. Wheel-up over an
/// unengaged tile opens the same frozen view without engaging (no PTY
/// bytes, layer untouched); wheel over an engaged LIVE tile is forwarded to
/// the application as an SGR mouse sequence (Claude Code enables mouse
/// reporting and scrolls its own alternate screen); wheel while frozen
/// pages the frozen view, and wheel-down at the tail returns to live.
///
/// Salience (mockup contract): amber border+title EXCLUSIVELY for
/// needs-input; engaged gets a bold highlight border; focused gets an
/// underlined title; working/idle stay neutral; done is green, fading after
/// 2 minutes. While frozen, the title shows the back-to-live affordance
/// `↓ live · +N` (lines arrived since freezing).
library;

import 'package:nocterm/nocterm.dart';

import '../input/encode_key.dart';
import '../input/paste.dart';
import '../state/wall_state.dart';
import 'pty_writer.dart';
import 'scroll_anchor.dart';
import 'theme.dart';
import 'wheel_region.dart';

class Tile extends StatefulComponent {
  const Tile({
    super.key,
    required this.session,
    required this.controller,
    required this.writer,
    required this.pasteForwarder,
    required this.onDisengage,
    required this.nowMs,
    this.deadReason,
    this.focused = false,
    this.engaged = false,
    this.restoring = false,
    this.obscured = false,
    this.seedText,
  });

  final WallSession session;

  /// Null while no PTY exists for this tile (restorable placeholder, or
  /// reconciliation hasn't created it yet).
  final PtyController? controller;

  /// Flush-safe write path for [controller] (null exactly when it is) —
  /// direct controller.write drops keys, see pty_writer.dart.
  final PtyWriter? writer;
  final PasteForwarder pasteForwarder;

  /// The Ctrl+G chord — the tile never mutates layer state itself.
  final void Function() onDisengage;

  /// Render-time clock (epoch ms), injected so elapsed text and done-fade
  /// stay pure functions of the build.
  final int nowMs;

  /// Non-null once the reattach budget is exhausted; renders the dead
  /// placeholder naming the reason.
  final String? deadReason;
  final bool focused;
  final bool engaged;

  /// A restore call is in flight for this session (optimistic placeholder
  /// state — spec tui-key-routing "p8.1 session lifecycle bindings").
  final bool restoring;

  /// The tile is fully covered by a maximized sibling: keep it mounted (its
  /// terminal buffer must survive the maximize round-trip) but drop its
  /// WheelRegion so wheel events can never route into an invisible tile.
  final bool obscured;

  /// Optional pre-attach history (tmux capture-pane output, CRLF line
  /// endings), written into the terminal once before any PTY output — see
  /// TilePtyRegistry.seedFor (flag-gated, off by default).
  final String? seedText;

  @override
  State<Tile> createState() => _TileState();
}

class _TileState extends State<Tile> {
  /// Per-tile scroll state — survives rebuilds because tiles are keyed by
  /// session id in the grid Stack.
  final ScrollAnchor _anchor = ScrollAnchor();

  /// Latest buffer geometry, reported by TerminalXterm during ITS build
  /// (so always one frame fresh under streaming output). Never triggers a
  /// setState — reading it next frame is the contract.
  BufferMetrics _metrics = BufferMetrics.zero;

  /// While frozen the title's `+N` counter must track output the tile
  /// itself doesn't rebuild for (TerminalXterm's setState only rebuilds its
  /// own subtree), so listen to the PTY output stream and rebuild — only
  /// while frozen, to keep the live path free of extra rebuilds.
  void Function(String)? _outputCallback;
  PtyController? _listenedController;

  @override
  void initState() {
    super.initState();
    _listen(component.controller);
  }

  @override
  void didUpdateComponent(Tile oldComponent) {
    super.didUpdateComponent(oldComponent);
    if (oldComponent.controller != component.controller) {
      _unlisten();
      _listen(component.controller);
    }
  }

  @override
  void dispose() {
    _unlisten();
    super.dispose();
  }

  void _listen(PtyController? c) {
    if (c == null) return;
    _listenedController = c;
    _outputCallback = (_) {
      if (mounted && _anchor.frozen) setState(() {});
    };
    c.addOutputCallback(_outputCallback!);
  }

  void _unlisten() {
    if (_listenedController != null && _outputCallback != null) {
      _listenedController!.removeOutputCallback(_outputCallback!);
    }
    _listenedController = null;
    _outputCallback = null;
  }

  void _onBufferMetrics(int totalLines, int droppedLines, int viewHeight) {
    // Called during TerminalXterm's build — store only, no setState.
    _metrics = BufferMetrics(
      totalLines: totalLines,
      droppedLines: droppedLines,
      viewHeight: viewHeight,
    );
  }

  int get _pageLines => ScrollAnchor.pageLines(_metrics.viewHeight);

  /// The engaged key path. Returning true always: never fall through.
  bool _onKeyEvent(KeyboardEvent e) {
    // Disengage chord (Ctrl+G; the framework's debug intercept is disabled
    // at startup via TerminalBinding.debugKeyEnabled = false, patch 4).
    // Deliberately does NOT snap to live — a frozen peek may outlive
    // engagement, same as the unengaged wheel peek.
    if (e.matches(LogicalKey.keyG, ctrl: true)) {
      component.onDisengage();
      return true;
    }
    // Frozen scrollback (spec tui-scrollback): Shift+PageUp freezes at an
    // absolute anchor and pages up; Shift+PageDown pages toward live and
    // snaps back when the window reaches the anchor tail. Never forwarded
    // to the app, never handed to TerminalXterm's relative scroll.
    if (e.modifiers.shift && e.logicalKey == LogicalKey.pageUp) {
      setState(() => _anchor.scrollUp(_pageLines, _metrics));
      return true;
    }
    if (e.modifiers.shift && e.logicalKey == LogicalKey.pageDown) {
      setState(() => _anchor.scrollDown(_pageLines));
      return true;
    }
    // End while frozen: snap back to live, consumed (a live End still
    // reaches the app through encodeKey below).
    if (_anchor.frozen && e.logicalKey == LogicalKey.end) {
      setState(_anchor.snapLive);
      return true;
    }
    // Synthetic Ctrl+V paste recovery: coalesced input/bracketed paste is
    // parked in ClipboardManager by the framework; forward it as one
    // bracketed paste. An empty buffer is a real Ctrl+V and falls through
    // to encodeKey (raw 0x16).
    final paste = component.pasteForwarder.recover(e);
    if (paste != null) {
      _snapLiveBeforePtyWrite();
      _write(paste);
      return true;
    }
    final bytes = encodeKey(e);
    if (bytes != null) {
      // Any key that reaches the PTY snaps the tile back to live first
      // (spec tui-scrollback "Typing snaps to live").
      _snapLiveBeforePtyWrite();
      _write(bytes);
    }
    return true;
  }

  void _snapLiveBeforePtyWrite() {
    if (_anchor.frozen) setState(_anchor.snapLive);
  }

  /// Wheel over this tile's terminal area (region-local 0-based coords).
  void _onWheel(bool up, int col, int row) {
    // A frozen view owns the wheel regardless of engagement — that is how
    // "wheel-down at the tail returns to live" works for the peek case, and
    // forwarding into an invisible live screen would be meaningless.
    if (_anchor.frozen) {
      setState(() {
        if (up) {
          _anchor.scrollUp(ScrollAnchor.wheelLines, _metrics);
        } else {
          _anchor.scrollDown(ScrollAnchor.wheelLines);
        }
      });
      return;
    }
    if (component.engaged) {
      // Live + engaged: forward to the application (spec tui-scrollback
      // "Mouse wheel scrolling") as the SGR wheel sequence with 1-based
      // coords local to the tile; Claude Code enables mouse reporting and
      // handles its own alternate-screen scrolling.
      _write('\x1b[<${up ? 64 : 65};${col + 1};${row + 1}M');
      return;
    }
    // Unengaged live tile: wheel-up opens the frozen peek WITHOUT engaging
    // — layer unchanged, no PTY bytes. Wheel-down at the tail is a no-op.
    if (up) {
      setState(() => _anchor.scrollUp(ScrollAnchor.wheelLines, _metrics));
    }
  }

  void _write(String data) => component.writer?.write(data);

  @override
  Component build(BuildContext context) {
    final session = component.session;
    final engaged = component.engaged;
    final focused = component.focused;
    final nowMs = component.nowMs;

    final blocked = session.needsInput;
    final glyphColor =
        statusColor(session.status, sinceMs: session.since, nowMs: nowMs);

    // Border salience: amber is reserved for needs-input and wins even over
    // the engaged highlight (a blocked tile must never stop being amber);
    // engaged is the bright bold frame, focused a lighter neutral.
    final borderColor = blocked
        ? GarageColors.amber
        : engaged
            ? GarageColors.fg
            : focused
                ? GarageColors.dim
                : GarageColors.faint;
    final borderStyle = engaged ? BoxBorderStyle.bold : BoxBorderStyle.rounded;

    final labelColor = blocked
        ? GarageColors.amber
        : (engaged || focused)
            ? GarageColors.fg
            : GarageColors.dim;
    final elapsed = elapsedFor(session.status, session.since, nowMs);

    final title = BorderTitle.rich(
      textSpan: TextSpan(children: [
        TextSpan(text: ' ${glyphFor(session.status)} ',
            style: TextStyle(color: glyphColor)),
        TextSpan(
          text: session.label,
          style: TextStyle(
            color: labelColor,
            fontWeight: engaged ? FontWeight.bold : null,
            // "Underline-ish" focus treatment from the mockup.
            decoration: focused ? TextDecoration.underline : null,
          ),
        ),
        if (session.branch != null)
          TextSpan(
              text: ' ⎇ ${session.branch}',
              style: const TextStyle(color: GarageColors.faint)),
        if (elapsed != null)
          TextSpan(
              text: ' $elapsed',
              style: TextStyle(
                  color: blocked ? GarageColors.amber : GarageColors.dim)),
        // Back-to-live affordance (spec tui-scrollback "Return to live"):
        // the not-live marker plus lines arrived since freezing, updated
        // every rebuild (output while frozen triggers one via the PTY
        // output listener).
        if (_anchor.frozen)
          TextSpan(
            text: ' ↓ live · +${_anchor.newLines(_metrics)} ',
            style: const TextStyle(
                color: GarageColors.fg, fontWeight: FontWeight.bold),
          ),
        const TextSpan(text: ' '),
      ]),
    );

    return Container(
      decoration: BoxDecoration(
        border: BoxBorder.all(color: borderColor, style: borderStyle),
        title: title,
      ),
      child: _body(),
    );
  }

  Component _body() {
    final session = component.session;
    if (!session.live) {
      // Restorable: no PTY is ever spawned (spec tui-wall "Restorable
      // placeholder"). While a restore call is in flight the placeholder
      // says so (optimistic state; the refetch after the response settles
      // the real status).
      if (component.restoring) {
        return _placeholder('⟳ restoring…', 'resuming the conversation');
      }
      return _placeholder(
          '⟳ restorable', 'press Enter (or click) to restore · x x to discard');
    }
    if (component.deadReason != null) {
      return _placeholder('✕ attach dead', component.deadReason!);
    }
    final c = component.controller;
    if (c == null) {
      return _placeholder('… attaching', '');
    }
    final terminal = TerminalXterm(
        controller: c,
        focused: component.engaged,
        autoStart: false, // the registry owns start/restart (staggered attach)
        onKeyEvent: _onKeyEvent,
        // Frozen history: absolute slice start (null = live tail follow).
        frozenTopLine: _anchor.topLine(_metrics),
        // Inner cursor (vendored patch 9): solid inverse-video block at the
        // emulator's cursor cell, ONLY while engaged and live — the
        // renderer additionally gates on DECTCEM (apps that hide the
        // cursor keep it hidden) and suppresses it in a frozen history
        // view. Unengaged tiles show none: a wall of six carets is noise.
        showCursor: component.engaged && !_anchor.frozen,
        onBufferMetrics: _onBufferMetrics,
        initialContent: component.seedText,
        // tmux attach parks its client in the alt screen; ignoring the
        // switch keeps everything in the main buffer so scrolled output
        // accumulates as local history (spec tui-scrollback "History
        // depth" — the emulator-side smcup@ trick, patch 6).
        ignoreAltBuffer: true,
        // suppressSizeReports stays OFF: the attach PTY is pipe-based
        // (script wrapper), so tmux learns the client size through the
        // in-band `CSI 18 t` → `CSI 8;rows;cols t` handshake the emulator
        // answers — suppressing it leaves the client sizeless and kills
        // the attach (patch 8).
      );
    // An obscured tile (covered by a maximized sibling) keeps its element
    // and buffer but must not catch wheel events.
    if (component.obscured) return terminal;
    return WheelRegion(onWheel: _onWheel, child: terminal);
  }

  Component _placeholder(String heading, String detail) => Center(
        child: Column(
          mainAxisSize: MainAxisSize.min,
          children: [
            Text(heading, style: const TextStyle(color: GarageColors.dim)),
            if (detail.isNotEmpty)
              Text(detail, style: const TextStyle(color: GarageColors.faint)),
          ],
        ),
      );
}
