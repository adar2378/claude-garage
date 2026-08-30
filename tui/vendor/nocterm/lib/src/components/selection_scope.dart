import '../framework/framework.dart';
import 'selection_state.dart';

/// Provides selection drag state to descendant widgets.
///
/// This InheritedComponent is used by [SelectionArea] to communicate
/// its selection state to descendants like [ListView], which need to
/// know when a selection drag is active to avoid cleaning up children
/// that are part of the selection range.
///
/// Example:
/// ```dart
/// SelectionArea(
///   child: ListView.builder(
///     itemCount: 100,
///     itemBuilder: (context, index) => Text('Item $index'),
///   ),
/// )
/// ```
///
/// In the above example, [ListView] will automatically use [SelectionScope]
/// to determine when to keep selection-range items built during a drag.
class SelectionScope extends InheritedComponent {
  /// Creates a selection scope.
  const SelectionScope({
    super.key,
    required this.isActive,
    required this.rangeFor,
    required this.updateRange,
    required super.child,
  });

  /// Whether a selection drag is currently active in this scope.
  final bool isActive;

  /// Returns the selection range for the given context object (typically a
  /// render object like [RenderListViewport]), or null if no range is set.
  final SelectionRange? Function(Object context) rangeFor;

  /// Updates the selection range for the given context object.
  ///
  /// This is called by [SelectionArea] during a drag to track which
  /// list indices are part of the current selection.
  final void Function(Object context, int minIndex, int maxIndex) updateRange;

  /// Returns the [SelectionScope] from the closest ancestor, or null if
  /// no [SelectionScope] ancestor exists.
  ///
  /// This method registers the calling component as a dependent of the
  /// [SelectionScope], so the component will rebuild when the selection
  /// state changes.
  static SelectionScope? maybeOf(BuildContext context) {
    return context.dependOnInheritedComponentOfExactType<SelectionScope>();
  }

  /// Returns the [SelectionScope] from the closest ancestor.
  ///
  /// Throws if no [SelectionScope] ancestor exists.
  static SelectionScope of(BuildContext context) {
    final scope = maybeOf(context);
    assert(scope != null, 'No SelectionScope found in context');
    return scope!;
  }

  @override
  bool updateShouldNotify(SelectionScope oldComponent) {
    return isActive != oldComponent.isActive;
  }
}
