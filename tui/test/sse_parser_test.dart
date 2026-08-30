// SSE line parser (tui/lib/api/sse.dart) against the daemon's exact wire
// shape (daemon/src/events.js): named `status`/`sessions` events, keepalive
// comment lines, '\n' line endings — plus the framing edge cases a chunked
// socket produces (partial lines, multiple events per chunk).
import 'package:garage_tui/api/sse.dart';
import 'package:test/test.dart';

void main() {
  test('parses a named event with JSON data (daemon `status` shape)', () {
    final parser = SseParser();
    final events = parser.addChunk(
      'event: status\ndata: {"id":"garage/a/x","status":"needs-input","since":123}\n\n',
    );
    expect(events, hasLength(1));
    expect(events.single.event, 'status');
    expect(
      events.single.data,
      '{"id":"garage/a/x","status":"needs-input","since":123}',
    );
  });

  test('keepalive and connected comments produce no events', () {
    final parser = SseParser();
    expect(parser.addChunk(': connected\n\n'), isEmpty);
    expect(parser.addChunk(': keepalive\n\n'), isEmpty);
    // A comment between fields of a real event does not disturb it.
    expect(
      parser.addChunk('event: sessions\n: keepalive\ndata: {}\n\n').single.event,
      'sessions',
    );
  });

  test('partial lines across chunks are reassembled', () {
    final parser = SseParser();
    expect(parser.addChunk('event: sta'), isEmpty);
    expect(parser.addChunk('tus\ndata: {"id"'), isEmpty);
    expect(parser.addChunk(':1}\n'), isEmpty);
    final events = parser.addChunk('\n');
    expect(events.single.event, 'status');
    expect(events.single.data, '{"id":1}');
  });

  test('one chunk carrying two events yields both, in order', () {
    final parser = SseParser();
    final events = parser.addChunk(
      'event: status\ndata: {"n":1}\n\nevent: sessions\ndata: {}\n\n',
    );
    expect(events.map((e) => e.event), ['status', 'sessions']);
    expect(events.first.data, '{"n":1}');
  });

  test('an event with no event: field dispatches as "message"', () {
    final parser = SseParser();
    final events = parser.addChunk('data: hello\n\n');
    expect(events.single.event, 'message');
    expect(events.single.data, 'hello');
  });

  test('multiple data: lines join with newline; only one leading space is '
      'stripped', () {
    final parser = SseParser();
    final events = parser.addChunk('data: line1\ndata:  spaced\ndata:\n\n');
    expect(events.single.data, 'line1\n spaced\n');
  });

  test('a blank line with no accumulated data dispatches nothing and resets '
      'the event name', () {
    final parser = SseParser();
    expect(parser.addChunk('event: status\n\n'), isEmpty);
    // The dangling name must not leak into the next event.
    expect(parser.addChunk('data: x\n\n').single.event, 'message');
  });

  test('CRLF line endings are tolerated', () {
    final parser = SseParser();
    final events = parser.addChunk('event: sessions\r\ndata: {}\r\n\r\n');
    expect(events.single.event, 'sessions');
    expect(events.single.data, '{}');
  });
}
