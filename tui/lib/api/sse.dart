/// SSE subscription to the daemon's `GET /api/events`.
///
/// The daemon (daemon/src/events.js) sends exactly two named events —
/// `status` with `{id, status, since}` and `sessions` with `{}` — plus
/// `: connected` / `: keepalive` comment lines every 15s.
///
/// Three pieces, separated so the logic is unit-testable without sockets:
///  - [SseParser]: pure line-based framing (partial lines across chunks,
///    comments ignored, multi-`data:` joining).
///  - [SseReconnectMachine]: pure reconnect/backoff/poll-fallback state
///    (500ms → 8s exponential; the 5s sessions-poll fallback is active only
///    while disconnected — design: "Poll fallback").
///  - [SseClient]: the thin dart:io shell that wires the two to a socket.
library;

import 'dart:async';
import 'dart:convert';
import 'dart:io';

import 'client.dart' show defaultDaemonBaseUrl;

/// One dispatched server-sent event.
class SseEvent {
  const SseEvent(this.event, this.data);

  /// Event name (`status` / `sessions` here); `message` when the server sent
  /// no `event:` field, per the SSE spec.
  final String event;

  /// Raw data payload (JSON text for the daemon's events).
  final String data;

  @override
  String toString() => 'SseEvent($event, $data)';
}

/// Incremental line-based SSE parser. Feed decoded chunks in arrival order;
/// each call returns the events completed by that chunk.
class SseParser {
  final StringBuffer _partialLine = StringBuffer();
  String _eventName = '';
  final List<String> _dataLines = [];

  List<SseEvent> addChunk(String chunk) {
    final events = <SseEvent>[];
    var start = 0;
    for (var i = 0; i < chunk.length; i++) {
      if (chunk.codeUnitAt(i) != 0x0a) continue; // '\n'
      _partialLine.write(chunk.substring(start, i));
      start = i + 1;
      var line = _partialLine.toString();
      _partialLine.clear();
      // The daemon terminates lines with '\n'; tolerate CRLF too.
      if (line.endsWith('\r')) line = line.substring(0, line.length - 1);
      final event = _processLine(line);
      if (event != null) events.add(event);
    }
    _partialLine.write(chunk.substring(start));
    return events;
  }

  SseEvent? _processLine(String line) {
    if (line.isEmpty) {
      // Blank line = dispatch. Per spec: no accumulated data → no event
      // (this is how keepalive comments stay invisible).
      if (_dataLines.isEmpty) {
        _eventName = '';
        return null;
      }
      final event = SseEvent(
        _eventName.isEmpty ? 'message' : _eventName,
        _dataLines.join('\n'),
      );
      _eventName = '';
      _dataLines.clear();
      return event;
    }
    if (line.startsWith(':')) return null; // comment (keepalive)

    final colon = line.indexOf(':');
    final field = colon < 0 ? line : line.substring(0, colon);
    var value = colon < 0 ? '' : line.substring(colon + 1);
    if (value.startsWith(' ')) value = value.substring(1);

    switch (field) {
      case 'event':
        _eventName = value;
      case 'data':
        _dataLines.add(value);
      default:
        // id/retry/unknown fields — ignored, the daemon sends neither.
        break;
    }
    return null;
  }
}

/// Pure reconnect state: exponential backoff 500ms → 8s (doubling, capped),
/// reset on a successful connect; the sessions-poll fallback is active
/// exactly while started-but-not-connected.
class SseReconnectMachine {
  SseReconnectMachine({
    this.initialDelay = const Duration(milliseconds: 500),
    this.maxDelay = const Duration(seconds: 8),
  });

  final Duration initialDelay;
  final Duration maxDelay;

  bool _started = false;
  bool _connected = false;
  int _attempt = 0;

  bool get connected => _connected;

  /// True exactly while the client should poll `/api/sessions` every 5s
  /// instead of trusting SSE (spec tui-wall: state comes from the daemon
  /// even when the stream is down).
  bool get pollFallbackActive => _started && !_connected;

  void start() => _started = true;

  void onConnected() {
    _connected = true;
    _attempt = 0;
  }

  /// Records a drop and returns how long to wait before the next attempt:
  /// 500ms, 1s, 2s, 4s, 8s, 8s, ...
  Duration onDisconnected() {
    _connected = false;
    // Cap the shift so _attempt can't overflow on very long outages.
    final shift = _attempt > 30 ? 30 : _attempt;
    _attempt++;
    final delayMs = initialDelay.inMilliseconds << shift;
    return delayMs >= maxDelay.inMilliseconds
        ? maxDelay
        : Duration(milliseconds: delayMs);
  }
}

/// Interval of the sessions-poll fallback while the stream is down.
const Duration ssePollFallbackInterval = Duration(seconds: 5);

/// Long-lived subscription to `/api/events`. Owns its own [HttpClient] so a
/// wedged stream can be torn down with `close(force: true)` without killing
/// the request/response client.
class SseClient {
  SseClient({
    String? baseUrl,
    required this.onEvent,
    this.onPollFallback,
    SseReconnectMachine? machine,
  })  : baseUrl = baseUrl ?? defaultDaemonBaseUrl,
        machine = machine ?? SseReconnectMachine();

  final String baseUrl;
  final void Function(SseEvent event) onEvent;

  /// Fires every [ssePollFallbackInterval] while disconnected — wire this to
  /// a sessions refetch.
  final void Function()? onPollFallback;
  final SseReconnectMachine machine;

  HttpClient? _http;
  Timer? _pollTimer;
  bool _stopped = false;

  void start() {
    if (machine.pollFallbackActive || machine.connected) return; // running
    machine.start();
    _startPollTimer();
    unawaited(_run());
  }

  Future<void> _run() async {
    while (!_stopped) {
      try {
        final http = _http = HttpClient();
        final request =
            await http.getUrl(Uri.parse('$baseUrl/api/events'));
        request.headers.set(HttpHeaders.acceptHeader, 'text/event-stream');
        final response = await request.close();
        if (response.statusCode != 200) {
          await response.drain<void>();
          throw HttpException('SSE endpoint returned ${response.statusCode}');
        }
        machine.onConnected();
        _stopPollTimer();
        final parser = SseParser();
        await for (final chunk in response.transform(utf8.decoder)) {
          for (final event in parser.addChunk(chunk)) {
            if (_stopped) return;
            onEvent(event);
          }
        }
      } on Object {
        // Connect failure or mid-stream error — fall through to reconnect.
      } finally {
        _http?.close(force: true);
        _http = null;
      }
      if (_stopped) return;
      final delay = machine.onDisconnected();
      _startPollTimer();
      await Future<void>.delayed(delay);
    }
  }

  void _startPollTimer() {
    if (_pollTimer != null || onPollFallback == null) return;
    _pollTimer = Timer.periodic(ssePollFallbackInterval, (_) {
      if (machine.pollFallbackActive) onPollFallback!();
    });
  }

  void _stopPollTimer() {
    _pollTimer?.cancel();
    _pollTimer = null;
  }

  void stop() {
    _stopped = true;
    _stopPollTimer();
    _http?.close(force: true);
    _http = null;
  }
}
