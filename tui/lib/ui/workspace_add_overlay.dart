/// The `w` add-workspace overlay (spec tui-key-routing "p8.1 session
/// lifecycle bindings"): a centered modal with one nocterm TextField for a
/// directory path. Enter submits (the app expands `~`, validates the dir
/// exists, derives the name web-UI-style and PUTs /api/workspaces); Esc
/// cancels. While it is open the root Focusable yields, so the TextField
/// owns the keys — the overlay intercepts only Esc via onKeyEvent.
///
/// Click routing mirrors the triage overlay: the modal box absorbs inside
/// clicks (no row semantics — clicks inside do nothing), the full-screen
/// barrier dismisses on an outside click and stops clicks reaching the
/// wall underneath.
library;

import 'package:nocterm/nocterm.dart';

import 'click_region.dart';
import 'theme.dart';

class WorkspaceAddOverlay extends StatefulComponent {
  const WorkspaceAddOverlay({
    super.key,
    required this.onSubmit,
    required this.onDismiss,
    this.error,
    this.busy = false,
  });

  /// The typed path, raw (untrimmed/unexpanded — the app owns validation
  /// so it stays testable as a pure function).
  final void Function(String path) onSubmit;
  final void Function() onDismiss;

  /// Validation/daemon error to show inside the modal; the overlay stays
  /// open so the path can be corrected.
  final String? error;

  /// True while the PUT is in flight — suppresses re-submits.
  final bool busy;

  @override
  State<WorkspaceAddOverlay> createState() => _WorkspaceAddOverlayState();
}

class _WorkspaceAddOverlayState extends State<WorkspaceAddOverlay> {
  final TextEditingController _controller = TextEditingController();

  @override
  void dispose() {
    _controller.dispose();
    super.dispose();
  }

  bool _onKeyEvent(KeyboardEvent event) {
    if (event.logicalKey == LogicalKey.escape) {
      component.onDismiss();
      return true;
    }
    return false; // everything else is the TextField's (typing, Enter)
  }

  void _submit(String text) {
    if (component.busy) return;
    component.onSubmit(text);
  }

  @override
  Component build(BuildContext context) {
    return LayoutBuilder(builder: (context, constraints) {
      final boxWidth = (constraints.maxWidth.floor() - 8).clamp(30, 64);
      final fieldWidth = boxWidth - 6;

      Component modal = Container(
        width: boxWidth.toDouble(),
        decoration: BoxDecoration(
          color: Colors.black,
          border: BoxBorder.all(
              color: GarageColors.dim, style: BoxBorderStyle.rounded),
          title: const BorderTitle(
            text: ' add workspace ',
            style: TextStyle(color: GarageColors.fg),
          ),
        ),
        padding: const EdgeInsets.symmetric(horizontal: 2, vertical: 1),
        child: Column(
          mainAxisSize: MainAxisSize.min,
          crossAxisAlignment: CrossAxisAlignment.start,
          children: [
            const Text('project directory',
                style: TextStyle(color: GarageColors.dim)),
            TextField(
              controller: _controller,
              focused: true,
              width: fieldWidth.toDouble(),
              placeholder: '~/dev/my-project',
              placeholderStyle: const TextStyle(color: GarageColors.faint),
              style: const TextStyle(color: GarageColors.fg),
              onKeyEvent: _onKeyEvent,
              onSubmitted: _submit,
            ),
            // Red, not amber — amber is reserved for needs-input.
            if (component.error != null)
              Text(component.error!,
                  style: const TextStyle(color: Colors.red)),
            const Text(''),
            Text(
              component.busy
                  ? 'registering…'
                  : 'Enter add · Esc cancel · name derives from the folder',
              style: const TextStyle(color: GarageColors.faint),
            ),
          ],
        ),
      );

      // Inner region absorbs clicks on the modal (they do nothing); the
      // barrier dismisses on true outside clicks, same as Esc.
      final absorber = ClickAbsorber();
      modal = ClickRegion(
        absorbs: absorber,
        onClick: (_, __) {},
        child: modal,
      );
      return ClickRegion(
        yieldsTo: absorber,
        onClick: (_, __) => component.onDismiss(),
        child: Center(child: modal),
      );
    });
  }
}
