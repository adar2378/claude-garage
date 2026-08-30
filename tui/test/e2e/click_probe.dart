/// Framework-level live probe for ClickRegion (post-review fix smoke).
///
/// A miniature of the wall's layout — rail-like bordered Column, grid-like
/// Stack under a LayoutBuilder, strip Row with a badge, and a toggleable
/// modal-over-barrier overlay — where every ClickRegion writes what it
/// resolved into the strip (`LAST=[...]`). Driven by
/// run_click_probe_smoke.sh: SGR press/release sequences via
/// `tmux send-keys -H`, assertions via `capture-pane`.
///
/// Verifies against a real terminal exactly the parts unit tests cannot:
/// nocterm's mouse routing reaches annotations through the real render
/// tree, paint-offset local coords are right for every surface, the
/// absorber suppresses the barrier for modal clicks, the overlay blocks
/// the wall underneath, and the triage modal's border+padding row offset
/// (hit_targets.triageModalRowOffset) matches real layout.
library;

import 'package:garage_tui/ui/click_region.dart';
import 'package:nocterm/nocterm.dart';

void main() async {
  await runApp(const ProbeApp());
}

class ProbeApp extends StatefulComponent {
  const ProbeApp({super.key});

  @override
  State<ProbeApp> createState() => _ProbeAppState();
}

class _ProbeAppState extends State<ProbeApp> {
  String last = 'none';
  bool overlay = false;

  void _set(String value) => setState(() => last = value);

  bool _onKey(KeyboardEvent e) {
    if (e.character == 'o') setState(() => overlay = !overlay);
    if (e.character == 'q') shutdownApp();
    return true;
  }

  @override
  Component build(BuildContext context) {
    final main = Column(children: [
      Expanded(
        child: Row(children: [
          // Rail analogue: bordered fixed-width Container, region INSIDE so
          // local row == rendered row (border insets all sides by 1).
          Container(
            width: 28,
            decoration: const BoxDecoration(
              border: BoxBorder(right: BorderSide(color: Colors.grey)),
            ),
            child: ClickRegion(
              onClick: (col, row) => _set('rail:$row'),
              child: const Column(
                crossAxisAlignment: CrossAxisAlignment.start,
                children: [
                  Text('ws header'),
                  Text('  row one'),
                  Text('  row two'),
                ],
              ),
            ),
          ),
          // Grid analogue: one region over a Stack of Positioned tiles.
          Expanded(
            child: LayoutBuilder(builder: (context, constraints) {
              final w = constraints.maxWidth.floor();
              final h = constraints.maxHeight.floor();
              return ClickRegion(
                onClick: (col, row) => _set('grid:$col,$row/${w}x$h'),
                child: Stack(children: [
                  Positioned(
                    left: 0,
                    top: 0,
                    width: (w ~/ 2).toDouble(),
                    height: h.toDouble(),
                    child: Container(
                      decoration: BoxDecoration(
                          border: BoxBorder.all(color: Colors.grey)),
                      child: const Text('tile one'),
                    ),
                  ),
                  Positioned(
                    left: (w ~/ 2).toDouble(),
                    top: 0,
                    width: (w - w ~/ 2).toDouble(),
                    height: h.toDouble(),
                    child: Container(
                      decoration: BoxDecoration(
                          border: BoxBorder.all(color: Colors.grey)),
                      child: const Text('tile two'),
                    ),
                  ),
                ]),
              );
            }),
          ),
        ]),
      ),
      // Strip analogue: result line + badge region.
      Container(
        color: Colors.black,
        child: Row(children: [
          Text('LAST=[$last]'),
          const Spacer(),
          ClickRegion(
            onClick: (_, __) => _set('badge'),
            child: RichText(
              maxLines: 1,
              softWrap: false,
              text: const TextSpan(
                  text: ' * 1 blocked ',
                  style: TextStyle(color: Colors.yellow)),
            ),
          ),
          const Text(' chip '),
        ]),
      ),
    ]);

    Component body = main;
    if (overlay) {
      // Triage analogue: barrier + absorbing modal, same structure and the
      // same border(1)+vertical padding(1) row offset as the real overlay.
      final absorber = ClickAbsorber();
      final modal = ClickRegion(
        absorbs: absorber,
        onClick: (col, row) => _set('modal:$row'),
        child: Container(
          width: 40,
          decoration: BoxDecoration(
            color: Colors.black,
            border: BoxBorder.all(color: Colors.grey),
          ),
          padding: const EdgeInsets.symmetric(horizontal: 2, vertical: 1),
          child: const Column(
            mainAxisSize: MainAxisSize.min,
            crossAxisAlignment: CrossAxisAlignment.start,
            children: [Text('queue row A'), Text('queue row B')],
          ),
        ),
      );
      body = Stack(children: [
        Positioned(left: 0, top: 0, right: 0, bottom: 0, child: main),
        Positioned(
          left: 0,
          top: 0,
          right: 0,
          bottom: 0,
          child: ClickRegion(
            yieldsTo: absorber,
            onClick: (_, __) => _set('barrier'),
            child: Center(child: modal),
          ),
        ),
      ]);
    }

    return Focusable(focused: true, onKeyEvent: _onKey, child: body);
  }
}
