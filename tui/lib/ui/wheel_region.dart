/// Mouse-wheel catcher for a tile (spec tui-scrollback "Mouse wheel
/// scrolling").
///
/// nocterm routes wheel MouseEvents (SGR mouse mode is enabled on the host
/// terminal by TerminalBinding) to the first render object mixing in
/// [ScrollableRenderObjectMixin] whose accumulated bounds contain the mouse
/// position — see `_dispatchMouseWheelAtPosition` in the vendored
/// terminal_binding.dart. Wrapping each tile's terminal area in a
/// [WheelRegion] therefore gives the tile per-tile wheel events; the render
/// object records its paint offset so the callback receives coordinates
/// LOCAL to the region (0-based cells), which the tile re-emits 1-based in
/// the SGR sequence it forwards to the PTY when engaged.
library;

import 'dart:math' as math;

import 'package:nocterm/nocterm.dart';
// TerminalCanvas and ScrollableRenderObjectMixin are not re-exported by
// package:nocterm/nocterm.dart — the fork is vendored, so the src imports
// are stable.
// ignore: implementation_imports
import 'package:nocterm/src/framework/terminal_canvas.dart';
// ignore: implementation_imports
import 'package:nocterm/src/rendering/scrollable_render_object.dart';

/// `up` is wheel-up; `col`/`row` are 0-based cell coordinates local to the
/// region.
typedef WheelCallback = void Function(bool up, int col, int row);

class WheelRegion extends SingleChildRenderObjectComponent {
  const WheelRegion({super.key, required this.onWheel, required super.child});

  final WheelCallback onWheel;

  @override
  RenderObject createRenderObject(BuildContext context) =>
      RenderWheelRegion(onWheel: onWheel);

  @override
  void updateRenderObject(
      BuildContext context, covariant RenderWheelRegion renderObject) {
    renderObject.onWheel = onWheel;
  }
}

class RenderWheelRegion extends RenderObject
    with RenderObjectWithChildMixin<RenderObject>, ScrollableRenderObjectMixin {
  RenderWheelRegion({required this.onWheel});

  WheelCallback onWheel;

  /// Global offset recorded at paint time — the only reliable way to turn
  /// the MouseEvent's absolute terminal coordinates into region-local ones
  /// (nocterm render objects don't carry a global transform).
  Offset _paintOffset = Offset.zero;

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
    if (child != null) {
      final childParentData = child!.parentData as BoxParentData;
      return child!.hitTest(result,
          position: position - childParentData.offset);
    }
    return false;
  }

  @override
  bool handleMouseWheel(MouseEvent event) {
    if (event.button != MouseButton.wheelUp &&
        event.button != MouseButton.wheelDown) {
      return false;
    }
    final maxCol = math.max(0, size.width.round() - 1);
    final maxRow = math.max(0, size.height.round() - 1);
    final col = (event.x - _paintOffset.dx).round().clamp(0, maxCol);
    final row = (event.y - _paintOffset.dy).round().clamp(0, maxRow);
    onWheel(event.button == MouseButton.wheelUp, col, row);
    return true;
  }
}
