/// Generic armed double-press state machine (spec tui-key-routing "p8.1
/// session lifecycle bindings" for the `x` close, "p8.3 workspace removal"
/// for the `X` remove): the first press on a key arms it, the second press
/// on the SAME key within [timeout] confirms; any other keyboard key — or a
/// press aimed at a different target — disarms/re-arms without confirming.
/// Pure (time injected) so it unit-tests without timers; the caller owns
/// the strip notice and its TTL. Keys are opaque target identifiers (a
/// session id for `x`, a workspace name for `X`) — one machine instance per
/// binding, so arming one never confirms the other.
library;

class ArmedAction {
  ArmedAction({this.timeout = const Duration(seconds: 3)});

  final Duration timeout;

  String? _armedId;
  int _armedAtMs = 0;

  /// The target id currently armed, or null. (The arm may have expired —
  /// [press] checks the clock, this getter is for rendering only.)
  String? get armedId => _armedId;

  /// A press aimed at [id] at [nowMs]. Returns true when this press
  /// CONFIRMS the action (same id, within the window) — the caller then
  /// performs the effect. False means the press (re-)armed: show the
  /// "press again" notice.
  bool press(String id, int nowMs) {
    final confirmed = _armedId == id &&
        nowMs - _armedAtMs <= timeout.inMilliseconds;
    if (confirmed) {
      _armedId = null;
      return true;
    }
    _armedId = id;
    _armedAtMs = nowMs;
    return false;
  }

  /// Any other key disarms (spec: "any other key disarms").
  void disarm() => _armedId = null;
}
