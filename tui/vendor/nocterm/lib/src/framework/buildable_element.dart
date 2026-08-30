part of 'framework.dart';

/// An element that has a build method.
abstract class BuildableElement extends Element {
  BuildableElement(super.component);

  Element? _child;

  bool _debugDoingBuild = false;
  @override
  bool get debugDoingBuild => _debugDoingBuild;

  void _firstBuild() {
    rebuild();
  }

  @override
  void mount(Element? parent, dynamic newSlot) {
    super.mount(parent, newSlot);
    assert(_child == null);
    _firstBuild();
  }

  @override
  void performRebuild() {
    assert(() {
      _debugDoingBuild = true;
      return true;
    }());

    Component? built;
    try {
      built = build();
    } catch (e, stack) {
      // Handle build errors
      _debugDoingBuild = false;
      built = ErrorComponent(error: e, stackTrace: stack);
      NoctermError.reportError(NoctermErrorDetails(
        exception: e,
        stack: stack,
        library: 'nocterm framework',
        context: 'while building $runtimeType',
      ));
    } finally {
      _dirty = false;
      assert(() {
        _debugDoingBuild = false;
        return true;
      }());
    }

    _child = updateChild(_child, built, slot);
  }

  @protected
  Component build();

  @override
  void visitChildren(ElementVisitor visitor) {
    if (_child != null) {
      visitor(_child!);
    }
  }

  @override
  void forgetChild(Element child) {
    assert(_child == child);
    _child = null;
  }
}

/// Component shown when there's an error during build.
///
/// Uses [RenderTUIErrorBox] to display a red bordered box with the error
/// message and stack trace, matching the visual style of layout/paint errors.
class ErrorComponent extends SingleChildRenderObjectComponent {
  const ErrorComponent({
    required this.error,
    required this.stackTrace,
  });

  final Object error;
  final StackTrace stackTrace;

  @override
  RenderObject createRenderObject(BuildContext context) {
    return RenderTUIErrorBox(
      message: 'Build error',
      error: error,
      stackTrace: stackTrace,
    );
  }

  @override
  void updateRenderObject(
      BuildContext context, RenderTUIErrorBox renderObject) {
    // RenderTUIErrorBox is immutable after creation
  }
}
