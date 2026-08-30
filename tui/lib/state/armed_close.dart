/// The `x` close arming (spec tui-key-routing "p8.1 session lifecycle
/// bindings") is one instance of the generic [ArmedAction] double-press
/// machine — p8.3's `X` workspace removal reuses the same machine with
/// workspace-name keys, so the semantics live in armed_action.dart and this
/// alias keeps the original name (and its tests) intact.
library;

import 'armed_action.dart';

export 'armed_action.dart';

typedef ArmedClose = ArmedAction;
