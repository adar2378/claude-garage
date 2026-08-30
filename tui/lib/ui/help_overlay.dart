/// The `?` help overlay: a centered key legend on the overlay layer
/// (spec tui-key-routing: overlays never stack; `?` toggles). A click
/// anywhere dismisses it (same as Esc); the full-screen ClickRegion also
/// stops clicks from reaching the wall underneath while it is open.
library;

import 'package:nocterm/nocterm.dart';

import 'click_region.dart';
import 'theme.dart';

const List<(String, String)> _bindings = [
  ('1-9', 'focus workspace'),
  ('[ ]', 'cycle focused tile'),
  ('Enter', 'engage focused tile (restore it when restorable)'),
  ('Ctrl+G', 'disengage (while engaged)'),
  ('m', 'maximize / restore focused tile'),
  ('a', 'jump to longest-waiting blocked session'),
  ('A', 'triage queue'),
  ('n / N', 'spawn session / worktree session'),
  ('R', 'restore all restorable sessions in workspace'),
  ('x x', 'close focused session (press twice)'),
  ('X X', 'remove focused workspace (sessions keep running)'),
  ('X K', 'remove focused workspace AND kill its sessions'),
  ('w', 'add workspace'),
  ('?', 'toggle this help'),
  ('q', 'quit (tmux sessions keep running)'),
];

class HelpOverlay extends StatelessComponent {
  const HelpOverlay({super.key, this.onDismiss});

  /// Any click dismisses (matches Esc). Null renders a click-inert legend.
  final void Function()? onDismiss;

  @override
  Component build(BuildContext context) {
    final legend = _legend();
    final onDismiss = this.onDismiss;
    if (onDismiss == null) return legend;
    return ClickRegion(onClick: (_, __) => onDismiss(), child: legend);
  }

  Component _legend() {
    return Center(
      child: Container(
        decoration: BoxDecoration(
          color: Colors.black,
          border: BoxBorder.all(
              color: GarageColors.dim, style: BoxBorderStyle.rounded),
          title: const BorderTitle(
            text: ' keys ',
            style: TextStyle(color: GarageColors.fg),
          ),
        ),
        padding: const EdgeInsets.symmetric(horizontal: 2, vertical: 1),
        child: Column(
          mainAxisSize: MainAxisSize.min,
          crossAxisAlignment: CrossAxisAlignment.start,
          children: [
            for (final (key, what) in _bindings)
              RichText(
                maxLines: 1,
                softWrap: false,
                text: TextSpan(children: [
                  TextSpan(
                      text: key.padRight(8),
                      style: const TextStyle(color: GarageColors.fg)),
                  TextSpan(text: what,
                      style: const TextStyle(color: GarageColors.dim)),
                ]),
              ),
          ],
        ),
      ),
    );
  }
}
