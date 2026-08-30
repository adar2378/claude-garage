/// Left-click catcher (spec tui-key-routing "Engage and disengage":
/// "Pressing Enter (or clicking a tile) ... SHALL engage"; spec tui-triage
/// "Pressing `A` (or clicking the strip badge) SHALL open" the queue).
///
/// nocterm routes every parsed MouseEvent through a render-tree hit test
/// (`_routeMouseEvent` in the vendored terminal_binding.dart); the
/// [MouseTracker] then dispatches enter/exit/hover callbacks to every
/// render object on the hit path that mixes in
/// [MouseTrackerAnnotationProvider]. GestureDetector rides the same path,
/// but its tap recognizer fires on release after an up/down pairing and
/// reports only global coordinates — while mapping a click to a tile index
/// or rail row needs coordinates LOCAL to the clicked surface. So, exactly
/// like [WheelRegion] does for wheel events, [ClickRegion] records its
/// paint offset and re-derives local cell coords, firing [onClick] on the
/// left-button press transition (press-fire: terminal UIs act on
/// mouse-down; there is no drag semantics on the wall).
///
/// [MouseTracker] dispatches to EVERY annotation on the hit path — a click
/// inside a modal would also fire a full-screen barrier region behind it.
/// [ClickAbsorber] fixes that: the inner region `absorbs` the (per-dispatch
/// unique) event instance, the outer region `yieldsTo` the same token and
/// skips it. Hit-path order guarantees inner-before-outer: annotations are
/// collected child-first (each region adds itself only after its child),
/// and the tracker's LinkedHashSets preserve that order.
library;

import 'dart:math' as math;

import 'package:nocterm/nocterm.dart';
// TerminalCanvas, MouseHitTestResult and the MouseTracker annotation types
// are not re-exported by package:nocterm/nocterm.dart — the fork is
// vendored, so the src imports are stable (same pattern as wheel_region).
// ignore: implementation_imports
import 'package:nocterm/src/framework/terminal_canvas.dart';
// ignore: implementation_imports
import 'package:nocterm/src/rendering/mouse_hit_test.dart';
// ignore: implementation_imports
import 'package:nocterm/src/rendering/mouse_tracker.dart';

/// `col`/`row` are 0-based cell coordinates local to the region.
typedef ClickCallback = void Function(int col, int row);

/// Shared token letting an inner [ClickRegion] swallow a click before an
/// outer barrier acts on it. Identity-based: the tracker hands the same
/// enriched MouseEvent instance to every annotation of one dispatch.
class ClickAbsorber {
  MouseEvent? _absorbed;

  void _absorb(MouseEvent event) => _absorbed = event;

  bool _wasAbsorbed(MouseEvent event) => identical(_absorbed, event);
}

class ClickRegion extends SingleChildRenderObjectComponent {
  const ClickRegion({
    super.key,
    required this.onClick,
    this.absorbs,
    this.yieldsTo,
    required super.child,
  });

  final ClickCallback onClick;

  /// Mark this token on every click inside the region (inner/modal role).
  final ClickAbsorber? absorbs;

  /// Skip a click another region already marked on this token (outer/barrier
  /// role).
  final ClickAbsorber? yieldsTo;

  @override
  RenderObject createRenderObject(BuildContext context) => RenderClickRegion(
      onClick: onClick, absorbs: absorbs, yieldsTo: yieldsTo);

  @override
  void updateRenderObject(
      BuildContext context, covariant RenderClickRegion renderObject) {
    renderObject
      ..onClick = onClick
      ..absorbs = absorbs
      ..yieldsTo = yieldsTo;
  }
}

class RenderClickRegion extends RenderObject
    with
        RenderObjectWithChildMixin<RenderObject>,
        MouseTrackerAnnotationProvider {
  RenderClickRegion({required this.onClick, this.absorbs, this.yieldsTo}) {
    // One annotation for the render object's lifetime. MouseTrackerAnnotation
    // equality is by renderObject, so hover continuity would survive
    // recreation too — but a stable instance keeps the tracker's bookkeeping
    // trivially correct.
    _annotation = MouseTrackerAnnotation(
      onEnter: _handleMouse,
      onExit: _handleExit,
      onHover: _handleMouse,
      renderObject: this,
    );
  }

  ClickCallback onClick;
  ClickAbsorber? absorbs;
  ClickAbsorber? yieldsTo;

  late final MouseTrackerAnnotation _annotation;

  @override
  MouseTrackerAnnotation? get annotation => _annotation;

  /// Global offset recorded at paint time — the only reliable way to turn
  /// the MouseEvent's absolute terminal coordinates into region-local ones
  /// (nocterm render objects don't carry a global transform).
  Offset _paintOffset = Offset.zero;

  /// Left-button state, tracked so one press event — which the tracker may
  /// deliver as BOTH onEnter and onHover — fires exactly one click, and a
  /// held button entering the region doesn't re-fire.
  bool _leftDown = false;

  void _handleMouse(MouseEvent event) {
    if (event.button != MouseButton.left) return;
    // On release ('m') the tracker removes `left` from the pressed set
    // before enriching, so `down` correctly drops to false.
    final down = event.pressed || event.isPrimaryButtonDown;
    if (down && !_leftDown) {
      _leftDown = true;
      if (event.isMotion) return; // drag entering the region is not a click
      if (yieldsTo?._wasAbsorbed(event) ?? false) return;
      absorbs?._absorb(event);
      final maxCol = math.max(0, size.width.round() - 1);
      final maxRow = math.max(0, size.height.round() - 1);
      final col = (event.x - _paintOffset.dx).round().clamp(0, maxCol);
      final row = (event.y - _paintOffset.dy).round().clamp(0, maxRow);
      onClick(col, row);
    } else if (!down && _leftDown) {
      _leftDown = false;
    }
  }

  void _handleExit(MouseEvent event) {
    // Reset so a press that ends elsewhere can't leave a stuck state.
    _leftDown = false;
  }

  @override
  void attach(PipelineOwner owner) {
    super.attach(owner);
    _annotation.validForMouseTracker = true;
  }

  @override
  void detach() {
    // Prevent callbacks on a disposed render object mid-dispatch (same
    // contract as the vendored RenderMouseRegion).
    _annotation.validForMouseTracker = false;
    super.detach();
  }

  @override
  void setupParentData(RenderObject child) {
    if (child.parentData is! BoxParentData) {
      child.parentData = BoxParentData();
    }
  }

  @override
  void performLayout() {
    if (child != null) {
      child!.layout(constraints, parentUsesSize: true);
      size = child!.size;
    } else {
      size = constraints.constrain(Size.zero);
    }
  }

  @override
  void paint(TerminalCanvas canvas, Offset offset) {
    super.paint(canvas, offset);
    _paintOffset = offset;
    if (child != null) {
      final childParentData = child!.parentData as BoxParentData;
      child!.paint(canvas, offset + childParentData.offset);
    }
  }

  @override
  bool hitTest(HitTestResult result, {required Offset position}) {
    if (!Rect.fromLTWH(0, 0, size.width, size.height).contains(position)) {
      return false;
    }
    // Children first, so an inner region's annotation lands in the result
    // before this one (the ClickAbsorber ordering contract).
    if (child != null) {
      final childParentData = child!.parentData as BoxParentData;
      child!.hitTest(result, position: position - childParentData.offset);
    }
    // Opaque within bounds: the region is a click target for its whole
    // area (tile borders, blank grid cells) and — as an overlay barrier —
    // must stop the hit test from reaching the wall underneath (Stack
    // stops at the first child whose hitTest returns true).
    if (result is MouseHitTestResult) {
      result.addWithPosition(target: this, localPosition: position);
    }
    return true;
  }
}
