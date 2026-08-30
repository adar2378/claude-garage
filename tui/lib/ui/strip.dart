/// The single-line bottom strip (spec tui-wall / tui-key-routing):
/// workspace tabs `1-9` with amber needs-input dots, the total blocked
/// count, a transient notice slot, and the always-visible keys-target chip.
///
/// The blocked badge is its own ClickRegion (spec tui-triage: "Pressing
/// `A` (or clicking the strip badge) SHALL open" the queue) — split into a
/// separate RichText so its bounds come from layout, not mirrored text
/// math. Clicks anywhere else in the strip are a no-op (no region).
library;

import 'package:nocterm/nocterm.dart';

import '../state/wall_state.dart';
import 'click_region.dart';
import 'theme.dart';

class Strip extends StatelessComponent {
  const Strip({super.key, required this.state, this.notice, this.onBadgeClick});

  final WallState state;

  /// Transient message (spawn errors, "nothing waiting" for the a-jump).
  final String? notice;

  /// The blocked badge was clicked. Null renders a click-inert badge.
  final void Function()? onBadgeClick;

  @override
  Component build(BuildContext context) {
    final tabs = <InlineSpan>[];
    for (var i = 0; i < state.groups.length && i < 9; i++) {
      final group = state.groups[i];
      final focused = group.name == state.focusedWorkspace;
      tabs.add(TextSpan(
        text: ' ${i + 1}:${group.name}',
        style: TextStyle(
          color: focused ? GarageColors.fg : GarageColors.dim,
          fontWeight: focused ? FontWeight.bold : null,
        ),
      ));
      if (group.hasNeedsInput) {
        tabs.add(const TextSpan(
            text: '●', style: TextStyle(color: GarageColors.amber)));
      }
    }

    final blocked = state.blockedCount;
    final chipEngaged = state.layer == KeyLayer.engaged;

    // Adjacent RichTexts render identically to the former concatenated
    // spans; the split exists so the badge is an independent hit target.
    Component badge = RichText(
      maxLines: 1,
      softWrap: false,
      text: TextSpan(
          text: ' ● $blocked blocked ',
          style: const TextStyle(color: GarageColors.amber)),
    );
    final onBadgeClick = this.onBadgeClick;
    if (onBadgeClick != null) {
      badge = ClickRegion(onClick: (_, __) => onBadgeClick(), child: badge);
    }

    return Container(
      color: Colors.black,
      child: Row(children: [
        RichText(maxLines: 1, softWrap: false, text: TextSpan(children: tabs)),
        const Spacer(),
        if (notice != null)
          RichText(
            maxLines: 1,
            softWrap: false,
            text: TextSpan(text: ' $notice ',
                style: const TextStyle(color: GarageColors.fg)),
          ),
        if (blocked > 0) badge,
        RichText(
          maxLines: 1,
          softWrap: false,
          text: TextSpan(
            text: ' ${state.keysTargetChip} ',
            style: TextStyle(
              color: chipEngaged ? Colors.black : GarageColors.dim,
              backgroundColor: chipEngaged ? GarageColors.amber : null,
            ),
          ),
        ),
      ]),
    );
  }
}
