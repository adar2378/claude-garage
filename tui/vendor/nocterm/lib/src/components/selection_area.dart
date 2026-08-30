import 'dart:async';
import 'dart:math' as math;

import 'package:meta/meta.dart';
import 'package:nocterm/nocterm.dart' hide TextAlign;

import '../rendering/mouse_region.dart';
import '../rendering/mouse_tracker.dart';
import '../text/selection_utils.dart' as selection_utils;
import 'selection_state.dart';

/// A widget that enables mouse text selection across all descendant [Text]
/// widgets (or any render object that mixes in [Selectable]).
///
/// Wrap a subtree with [SelectionArea] to allow click-and-drag selection:
///
/// ```dart
/// SelectionArea(
///   child: Column(children: [
///     Text('First paragraph'),
///     Text('Second paragraph'),
///   ]),
/// )
/// ```
///
/// Selection is driven from the top: the [SelectionArea] captures pointer
/// events, walks its render subtree to discover [Selectable] render objects,
/// and tells each one which character range to highlight.
class SelectionArea extends StatefulComponent {
  const SelectionArea({
    super.key,
    required this.child,
    this.selectionColor,
    this.onSelectionChanged,
    this.onSelectionCompleted,
  });

  /// The child widget tree containing [Text] widgets to make selectable.
  final Component child;

  /// The color used to highlight selected text.
  /// If null, defaults to [TuiThemeData.selectionColor].
  final Color? selectionColor;

  /// Called when the selection changes. The callback receives the currently
  /// selected text (possibly spanning multiple widgets), or an empty string
  /// when the selection is cleared.
  final ValueChanged<String>? onSelectionChanged;

  /// Called when a drag selection completes (mouse up).
  /// Receives the selected text (or empty string if nothing is selected).
  final ValueChanged<String>? onSelectionCompleted;

  @override
  State<SelectionArea> createState() => _SelectionAreaState();
}

class _SelectionAreaState extends State<SelectionArea> {
  bool _isActive = false;
  final Map<Object, SelectionRange> _ranges = {};

  void _onDragStarted() {
    if (!_isActive) {
      setState(() {
        _isActive = true;
      });
    }
    // Also update global state for backwards compatibility
    SelectionDragState.begin();
  }

  void _onDragEnded() {
    SelectionDragState.end();
    if (_isActive) {
      setState(() {
        _isActive = false;
        _ranges.clear();
      });
    }
  }

  void _updateRange(Object context, int minIndex, int maxIndex) {
    if (minIndex > maxIndex) return;
    _ranges[context] = SelectionRange(minIndex, maxIndex);
    // Also update global state for backwards compatibility
    SelectionDragState.updateRange(context, minIndex, maxIndex);
  }

  SelectionRange? _rangeFor(Object context) {
    return _ranges[context];
  }

  @override
  void dispose() {
    if (_isActive) {
      // Ensure global drag state is cleaned up even if the widget unmounts
      // while a drag is in progress.
      SelectionDragState.end();
      _isActive = false;
      _ranges.clear();
    }
    super.dispose();
  }

  @override
  Component build(BuildContext context) {
    final theme = TuiTheme.of(context);
    final effectiveColor = component.selectionColor ?? theme.selectionColor;

    return SelectionScope(
      isActive: _isActive,
      rangeFor: _rangeFor,
      updateRange: _updateRange,
      child: _SelectionAreaWidget(
        selectionColor: effectiveColor,
        onSelectionChanged: component.onSelectionChanged,
        onSelectionCompleted: component.onSelectionCompleted,
        onDragStarted: _onDragStarted,
        onDragEnded: _onDragEnded,
        onRangeUpdated: _updateRange,
        child: component.child,
      ),
    );
  }
}

/// Internal single-child render object component.
class _SelectionAreaWidget extends SingleChildRenderObjectComponent {
  const _SelectionAreaWidget({
    required super.child,
    required this.selectionColor,
    this.onSelectionChanged,
    this.onSelectionCompleted,
    this.onDragStarted,
    this.onDragEnded,
    this.onRangeUpdated,
  });

  final Color selectionColor;
  final ValueChanged<String>? onSelectionChanged;
  final ValueChanged<String>? onSelectionCompleted;
  final VoidCallback? onDragStarted;
  final VoidCallback? onDragEnded;
  final void Function(Object context, int minIndex, int maxIndex)?
      onRangeUpdated;

  @override
  RenderObject createRenderObject(BuildContext context) {
    return RenderSelectionArea(
      selectionColor: selectionColor,
      onSelectionChanged: onSelectionChanged,
      onSelectionCompleted: onSelectionCompleted,
      onDragStarted: onDragStarted,
      onDragEnded: onDragEnded,
      onRangeUpdated: onRangeUpdated,
    );
  }

  @override
  void updateRenderObject(
      BuildContext context, covariant RenderSelectionArea renderObject) {
    renderObject
      ..selectionColor = selectionColor
      ..onSelectionChanged = onSelectionChanged
      ..onSelectionCompleted = onSelectionCompleted
      ..onDragStarted = onDragStarted
      ..onDragEnded = onDragEnded
      ..onRangeUpdated = onRangeUpdated;
  }
}

/// Render object that drives cross-widget text selection.
///
/// It extends [RenderMouseRegion] so it can capture pointer down/move/up
/// events, and it walks its render subtree to discover [Selectable] children.
class RenderSelectionArea extends RenderMouseRegion {
  RenderSelectionArea({
    required Color selectionColor,
    this.onSelectionChanged,
    this.onSelectionCompleted,
    this.onDragStarted,
    this.onDragEnded,
    this.onRangeUpdated,
  }) : _selectionColor = selectionColor;

  Color _selectionColor;
  Color get selectionColor => _selectionColor;
  set selectionColor(Color value) {
    if (_selectionColor == value) return;
    _selectionColor = value;
    // Push color to all currently-selected selectables
    for (final s in _cachedSelectables) {
      s.selectionColor = value;
    }
    markNeedsPaint();
  }

  ValueChanged<String>? onSelectionChanged;
  ValueChanged<String>? onSelectionCompleted;
  VoidCallback? onDragStarted;
  VoidCallback? onDragEnded;
  void Function(Object context, int minIndex, int maxIndex)? onRangeUpdated;

  // Selection state
  bool _isDragging = false;
  bool _pendingPostFrameSelection = false;

  /// Sorted list of selectables discovered on pointer-down.
  List<Selectable> _cachedSelectables = [];
  Map<Object, List<_SelectableEntry>> _contextLists = {};

  // Selectables cache to avoid walking the tree on every mouse move
  bool _selectablesCacheValid = false;
  List<Selectable>? _selectablesCache;
  Map<Object, List<_SelectableEntry>>? _contextListsCache;

  /// The position where the pointer was initially pressed.
  Offset _pressPosition = Offset.zero;

  /// Anchor (start of drag) selectable and char index.
  _SelectionPosition? _anchor;

  /// Focus (current drag position) selectable and char index.
  _SelectionPosition? _focus;

  // Edge auto-scroll while dragging near a scroll viewport boundary.
  static const double _edgeScrollThreshold = 1.0;
  static const double _edgeScrollDelta = 1.0;
  bool _edgeAutoScrolling = false;
  Offset? _edgeScrollLocalPosition;

  /// Tracks pointer motion after the cursor leaves this region mid-drag.
  StreamSubscription<MouseEvent>? _externalDragSubscription;

  // Cache invalidation

  void _invalidateSelectablesCache() {
    _selectablesCacheValid = false;
    _selectablesCache = null;
    _contextListsCache = null;
  }

  @override
  void markNeedsLayout() {
    super.markNeedsLayout();
    _invalidateSelectablesCache();
  }

  @override
  void adoptChild(RenderObject child) {
    super.adoptChild(child);
    _invalidateSelectablesCache();
  }

  @override
  void dropChild(RenderObject child) {
    super.dropChild(child);
    _invalidateSelectablesCache();
  }

  // Gesture handling

  @override
  void attach(PipelineOwner owner) {
    super.attach(owner);
    _updateSelectionAnnotation();
  }

  @override
  void detach() {
    _stopExternalDragTracking();
    _stopEdgeAutoScroll();
    super.detach();
  }

  MouseTrackerAnnotation? _selectionAnnotation;

  @override
  MouseTrackerAnnotation? get annotation =>
      _selectionAnnotation ?? super.annotation;

  /// Whether the left mouse button is currently pressed inside this region.
  bool _isLeftButtonPressed = false;

  void _updateSelectionAnnotation() {
    _selectionAnnotation = MouseTrackerAnnotation(
      onEnter: (event) {
        _stopExternalDragTracking();
        if (event.button == MouseButton.left) {
          final leftDown = event.pressed || event.isPrimaryButtonDown;
          if (leftDown && !_isLeftButtonPressed) {
            _isLeftButtonPressed = true;
            _handlePointerDown(event);
          } else if (!leftDown) {
            _isLeftButtonPressed = false;
          }
        }
      },
      onExit: (event) {
        if (_isDragging && event.isPrimaryButtonDown) {
          final clamped = _clampPointerPosition(
            Offset(event.x.toDouble(), event.y.toDouble()),
          );
          _edgeScrollLocalPosition = clamped;
          _updateDragSelectionAt(clamped);
          _updateEdgeAutoScroll(clamped);
          _startExternalDragTracking();
          return;
        }
        if (_isDragging) {
          _isLeftButtonPressed = false;
          _handlePointerUp(event);
          return;
        }
        _isLeftButtonPressed = false;
      },
      onHover: (event) {
        if (event.button == MouseButton.wheelUp ||
            event.button == MouseButton.wheelDown) {
          if (_isDragging || event.isPrimaryButtonDown) {
            _handlePointerMove(event);
            _schedulePostFrameSelectionUpdate(event);
          }
          return;
        }

        if (event.button == MouseButton.left) {
          final leftDown = event.pressed || event.isPrimaryButtonDown;
          if (leftDown && !_isLeftButtonPressed) {
            _isLeftButtonPressed = true;
            _handlePointerDown(event);
          } else if (!leftDown && _isLeftButtonPressed) {
            _isLeftButtonPressed = false;
            _handlePointerUp(event);
          } else if (leftDown && _isLeftButtonPressed) {
            // Dragging
            _handlePointerMove(event);
          }
        }
      },
      renderObject: this,
    );
  }

  // Pointer event handlers
  void _handlePointerDown(MouseEvent event) {
    final localPos = Offset(event.x.toDouble(), event.y.toDouble());
    _pressPosition = localPos;
    if (!_isDragging) {
      onDragStarted?.call();
    }

    // Collect and sort selectables
    _cachedSelectables = _collectSelectables(this);
    _contextLists = _buildContextLists(_cachedSelectables);

    // Clear existing selections
    for (final s in _cachedSelectables) {
      s.clearSelection();
    }

    // Hit-test to find which selectable contains the pointer
    final hit = _hitTestSelectables(localPos);
    if (hit != null) {
      final localOffset = _globalToSelectableLocal(localPos, hit);
      final charIndex = hit.getCharacterIndexAtLocalPosition(localOffset);
      final contextKey = _contextKeyForSelectable(hit);
      final id = _selectionIdForSelectable(hit);
      final entries = _contextLists[contextKey];
      final entry = entries == null ? null : _entryForId(entries, id);

      _anchor = _SelectionPosition(
        context: contextKey,
        selectableId: id,
        offset: charIndex,
        orderIndex: entry == null ? null : entries!.indexOf(entry),
      );
      _focus = _SelectionPosition(
        context: contextKey,
        selectableId: id,
        offset: charIndex,
        orderIndex: entry == null ? null : entries!.indexOf(entry),
      );
      if (entries != null) {
        _updateSelectionRangeForViewport(
            entries, contextKey, _anchor!, _focus!);
      }
    } else {
      _anchor = null;
      _focus = null;
      // Don't set anchor/focus — will be set on first move
    }

    _isDragging = true; // Always start drag, even from whitespace
    _notifySelectionChanged();
  }

  void _handlePointerMove(MouseEvent event) {
    if (!_isDragging) return;

    final localPos = Offset(event.x.toDouble(), event.y.toDouble());
    _updateDragSelectionAt(localPos);
    _updateEdgeAutoScroll(localPos);
  }

  void _updateDragSelectionAt(Offset localPos) {
    _cachedSelectables = _collectSelectables(this);
    if (_cachedSelectables.isEmpty) return;
    _contextLists = _buildContextLists(_cachedSelectables);

    // Late anchor: first selectable the pointer touches becomes the anchor.
    if (_anchor == null) {
      final hit = _hitTestSelectables(localPos);
      if (hit != null) {
        final contextKey = _contextKeyForSelectable(hit);
        final id = _selectionIdForSelectable(hit);
        final localOffset = _globalToSelectableLocal(localPos, hit);
        final focusIndex = _focusIndexFromPress(hit, localPos, localOffset);
        final anchorIndex = _anchorIndexFromPress(hit);
        final entries = _contextLists[contextKey];
        final entry = entries == null ? null : _entryForId(entries, id);
        _anchor = _SelectionPosition(
          context: contextKey,
          selectableId: id,
          offset: anchorIndex,
          orderIndex: entry == null ? null : entries!.indexOf(entry),
        );
        _focus = _SelectionPosition(
          context: contextKey,
          selectableId: id,
          offset: focusIndex,
          orderIndex: entry == null ? null : entries!.indexOf(entry),
        );
      }
      _notifySelectionChanged();
      return;
    }

    final contextKey = _anchor!.context;
    final entries = _contextLists[contextKey];
    if (entries == null || entries.isEmpty) {
      _anchor = null;
      _focus = null;
      _notifySelectionChanged();
      return;
    }

    final anchorEntry = _entryForId(entries, _anchor!.selectableId);
    Selectable? anchorSelectable = anchorEntry?.selectable;
    if (anchorSelectable == null) {
      final reanchorEntry =
          _entryByClampedIndex(entries, _anchor!.orderIndex) ??
              _entryForId(
                entries,
                _anchor!.selectableId,
              );
      final reanchor = reanchorEntry?.selectable ??
          _hitTestSelectablesInContext(_pressPosition, entries) ??
          _nearestSelectableInContext(_pressPosition, entries);
      if (reanchor == null) {
        _anchor = null;
        _focus = null;
        _notifySelectionChanged();
        return;
      }
      final reanchorId = _selectionIdForSelectable(reanchor);
      final reanchorEntryResolved = _entryForId(entries, reanchorId);
      anchorSelectable = reanchor;
      _anchor = _SelectionPosition(
        context: contextKey,
        selectableId: reanchorId,
        offset: _anchorIndexFromPress(reanchor),
        orderIndex: reanchorEntryResolved == null
            ? null
            : entries.indexOf(reanchorEntryResolved),
      );
    }
    if (_anchor!.orderIndex == null && anchorEntry != null) {
      _anchor!.orderIndex = entries.indexOf(anchorEntry);
    }

    final hit = _hitTestSelectablesInContext(localPos, entries);
    final focusSelectable =
        hit ?? _nearestSelectableInContext(localPos, entries);
    if (focusSelectable != null) {
      final localOffset = _globalToSelectableLocal(localPos, focusSelectable);
      final focusId = _selectionIdForSelectable(focusSelectable);
      final focusEntry = _entryForId(entries, focusId);
      _focus = _SelectionPosition(
        context: contextKey,
        selectableId: focusId,
        offset: _focusIndexFromPress(focusSelectable, localPos, localOffset),
        orderIndex: focusEntry == null ? null : entries.indexOf(focusEntry),
      );

      _updateSelectionRanges(
        entries,
        _anchor!,
        _focus!,
      );

      _updateSelectionRangeForViewport(entries, contextKey, _anchor!, _focus!);
    }

    final overscrollDirection = _edgeScrollDirectionAt(localPos);
    if (overscrollDirection != 0) {
      _extendFocusForOverscroll(localPos, overscrollDirection);
    }

    _notifySelectionChanged();
  }

  void _updateEdgeAutoScroll(Offset localPos) {
    _edgeScrollLocalPosition = localPos;
    if (!_isDragging) {
      _stopEdgeAutoScroll();
      return;
    }

    if (_edgeScrollDirectionAt(localPos) == 0) {
      _stopEdgeAutoScroll();
      return;
    }

    if (!_edgeAutoScrolling) {
      _edgeAutoScrolling = true;
      _edgeScrollTick();
    }
  }

  void _edgeScrollTick() {
    if (!_edgeAutoScrolling || !_isDragging) {
      _stopEdgeAutoScroll();
      return;
    }

    final localPos = _edgeScrollLocalPosition;
    if (localPos == null) {
      _stopEdgeAutoScroll();
      return;
    }

    final direction = _edgeScrollDirectionAt(localPos);
    if (direction == 0) {
      _stopEdgeAutoScroll();
      return;
    }

    final viewport = _primaryScrollViewport();
    if (viewport == null || !_scrollViewport(viewport, direction)) {
      _stopEdgeAutoScroll();
      return;
    }

    _invalidateSelectablesCache();
    _updateDragSelectionAt(localPos);
    _extendFocusForOverscroll(localPos, direction);
    _schedulePostFrameSelectionUpdate(_mouseEventAt(localPos));

    SchedulerBinding.instance.scheduleFrameCallback((_) => _edgeScrollTick());
  }

  /// When the pointer has overscrolled past the viewport edge, snap focus to
  /// the boundary row so selection continues to grow as content scrolls in.
  void _extendFocusForOverscroll(Offset terminalPos, int direction) {
    if (_anchor == null || _focus == null) return;

    final viewport = _primaryScrollViewport();
    if (viewport is! RenderListViewport) return;

    final contextKey = _anchor!.context;
    if (contextKey != viewport) return;

    final entries = _contextLists[contextKey];
    if (entries == null || entries.isEmpty) return;

    final vpOffset = _globalPaintOffsetOf(viewport);
    final vpTop = vpOffset.dy;
    final vpBottom = vpOffset.dy + viewport.size.height;
    final overscrolledAbove = terminalPos.dy < vpTop;
    final overscrolledBelow = terminalPos.dy >= vpBottom;

    if (direction < 0 && overscrolledAbove) {
      final first = entries.first;
      _focus = _SelectionPosition(
        context: contextKey,
        selectableId: first.id,
        offset: 0,
        orderIndex: entries.indexOf(first),
      );
    } else if (direction > 0 && overscrolledBelow) {
      final last = entries.last;
      _focus = _SelectionPosition(
        context: contextKey,
        selectableId: last.id,
        offset: last.selectable.selectableText.length,
        orderIndex: entries.indexOf(last),
      );
    } else {
      return;
    }

    _updateSelectionRanges(entries, _anchor!, _focus!);
    _updateSelectionRangeForViewport(entries, contextKey, _anchor!, _focus!);
    _notifySelectionChanged();
  }

  void _stopEdgeAutoScroll() {
    _edgeAutoScrolling = false;
    _edgeScrollLocalPosition = null;
  }

  void _startExternalDragTracking() {
    if (_externalDragSubscription != null) return;
    final binding = NoctermBinding.instance;
    if (binding is! TerminalBinding) return;
    _externalDragSubscription =
        binding.mouseEvents.listen(_handleExternalMouseEvent);
  }

  void _stopExternalDragTracking() {
    _externalDragSubscription?.cancel();
    _externalDragSubscription = null;
  }

  void _handleExternalMouseEvent(MouseEvent event) {
    if (!_isDragging) {
      _stopExternalDragTracking();
      return;
    }

    if (event.button == MouseButton.wheelUp ||
        event.button == MouseButton.wheelDown) {
      if (event.isPrimaryButtonDown) {
        final clamped = _clampPointerPosition(
          Offset(event.x.toDouble(), event.y.toDouble()),
        );
        _updateDragSelectionAt(clamped);
        _schedulePostFrameSelectionUpdate(event);
      }
      return;
    }

    if (event.button != MouseButton.left) return;

    if (!event.isMotion && !event.pressed) {
      _isLeftButtonPressed = false;
      final clamped = _clampPointerPosition(
        Offset(event.x.toDouble(), event.y.toDouble()),
      );
      _handlePointerUp(_mouseEventAt(clamped));
      return;
    }

    if (event.isPrimaryButtonDown) {
      _isLeftButtonPressed = true;
      final clamped = _clampPointerPosition(
        Offset(event.x.toDouble(), event.y.toDouble()),
      );
      _edgeScrollLocalPosition = clamped;
      _updateDragSelectionAt(clamped);
      _updateEdgeAutoScroll(clamped);
    }
  }

  Offset _clampPointerPosition(Offset pos) {
    final bounds = _selectionAreaBounds();
    final right = bounds.right <= bounds.left ? bounds.left : bounds.right - 1;
    final bottom = bounds.bottom <= bounds.top ? bounds.top : bounds.bottom - 1;
    return Offset(
      pos.dx.clamp(bounds.left, right),
      pos.dy.clamp(bounds.top, bottom),
    );
  }

  MouseEvent _mouseEventAt(Offset localPos) {
    return MouseEvent(
      button: MouseButton.left,
      x: localPos.dx.round(),
      y: localPos.dy.round(),
      pressed: true,
      isMotion: true,
      buttons: const {MouseButton.left},
    );
  }

  /// Mouse event coordinates are terminal-global, not local to this region.
  RenderObject? _primaryScrollViewport() {
    RenderObject? found;
    void visit(RenderObject node) {
      node.visitChildren(visit);
      if (node is RenderListViewport || node is RenderSingleChildViewport) {
        found = node;
      }
    }

    visitChildren(visit);
    return found;
  }

  int _edgeScrollDirectionAt(Offset terminalPos) {
    final viewport = _primaryScrollViewport();
    if (viewport == null || !viewport.hasSize) return 0;

    final direction = _edgeScrollDirectionFor(terminalPos, viewport);
    if (direction != 0) return direction;

    // Pointer overscrolled outside the viewport (e.g. into padding/header)
    // but drag is still active — keep scrolling from the selection bounds.
    final area = _selectionAreaBounds();
    if (terminalPos.dy <= area.top + _edgeScrollThreshold) {
      return -1;
    }
    if (terminalPos.dy >= area.bottom - _edgeScrollThreshold - 1) {
      return 1;
    }
    return 0;
  }

  int _edgeScrollDirectionFor(Offset terminalPos, RenderObject viewport) {
    if (!viewport.hasSize) return 0;

    final offset = _globalPaintOffsetOf(viewport);
    final top = offset.dy;
    final bottom = offset.dy + viewport.size.height;

    if (terminalPos.dy <= top + _edgeScrollThreshold) {
      return -1;
    }
    if (terminalPos.dy >= bottom - _edgeScrollThreshold - 1) {
      return 1;
    }
    return 0;
  }

  bool _scrollViewport(RenderObject viewport, int direction) {
    final ScrollController controller;
    final bool reverse;
    final Axis scrollDirection;

    if (viewport is RenderListViewport) {
      controller = viewport.controller;
      reverse = viewport.reverse;
      scrollDirection = viewport.scrollDirection;
    } else if (viewport is RenderSingleChildViewport) {
      controller = viewport.controller;
      reverse = false;
      scrollDirection = viewport.scrollDirection;
    } else {
      return false;
    }

    if (scrollDirection != Axis.vertical) return false;

    if (direction < 0) {
      if (reverse) {
        if (controller.offset >= controller.maxScrollExtent) return false;
        controller.scrollDown(_edgeScrollDelta);
      } else {
        if (controller.offset <= controller.minScrollExtent) return false;
        controller.scrollUp(_edgeScrollDelta);
      }
    } else {
      if (reverse) {
        if (controller.offset <= controller.minScrollExtent) return false;
        controller.scrollUp(_edgeScrollDelta);
      } else {
        if (controller.offset >= controller.maxScrollExtent) return false;
        controller.scrollDown(_edgeScrollDelta);
      }
    }
    return true;
  }

  void _updateSelectionRangeForViewport(
    List<_SelectableEntry> entries,
    Object contextKey,
    _SelectionPosition anchor,
    _SelectionPosition focus,
  ) {
    if (contextKey is! RenderListViewport) return;
    final anchorEntry = _entryForId(entries, anchor.selectableId);
    final focusEntry = _entryForId(entries, focus.selectableId);
    final anchorIndex = anchorEntry?.listIndex;
    final focusIndex = focusEntry?.listIndex;
    if (anchorIndex == null || focusIndex == null) return;
    final minIndex = math.min(anchorIndex, focusIndex);
    final maxIndex = math.max(anchorIndex, focusIndex);
    onRangeUpdated?.call(contextKey, minIndex, maxIndex);
  }

  void _handlePointerUp(MouseEvent event) {
    _stopExternalDragTracking();
    _stopEdgeAutoScroll();
    if (_isDragging) {
      _handlePointerMove(event);
      onDragEnded?.call();
      markNeedsLayout();
    }
    _isDragging = false;
    // Keep selection visible (don't clear on mouse up)
    if (onSelectionCompleted == null) return;
    onSelectionCompleted!(_collectSelectedText());
  }

  /// Schedules a post-frame callback to re-apply selection after layout.
  ///
  /// When a wheel event fires during a drag, the scroll offset changes before
  /// layout runs. [_handlePointerMove] applies selection to the items currently
  /// in the tree, but the subsequent layout pass may remove and rebuild items
  /// (via [_forceBuildSelectionRange]). Those newly built items are fresh
  /// instances with no selection set, causing gaps. This callback re-applies
  /// selection after layout has completed.
  void _schedulePostFrameSelectionUpdate(MouseEvent event) {
    if (_pendingPostFrameSelection) return;
    _pendingPostFrameSelection = true;
    SchedulerBinding.instance.addPostFrameCallback((_) {
      _pendingPostFrameSelection = false;
      if (!_isDragging) return;
      if (_anchor == null || _focus == null) return;

      _cachedSelectables = _collectSelectables(this);
      if (_cachedSelectables.isEmpty) return;
      _contextLists = _buildContextLists(_cachedSelectables);

      final contextKey = _anchor!.context;
      final entries = _contextLists[contextKey];
      if (entries == null || entries.isEmpty) return;

      // Re-resolve anchor and focus entries in the new entry list.
      final anchorEntry = _entryForId(entries, _anchor!.selectableId);
      final focusEntry = _entryForId(entries, _focus!.selectableId);
      if (anchorEntry != null) {
        _anchor!.orderIndex = entries.indexOf(anchorEntry);
      }
      if (focusEntry != null) {
        _focus!.orderIndex = entries.indexOf(focusEntry);
      }

      _updateSelectionRanges(entries, _anchor!, _focus!);
      _updateSelectionRangeForViewport(entries, contextKey, _anchor!, _focus!);
      _notifySelectionChanged();
    });
  }

  // Subtree walking
  /// Walks the render subtree rooted at [root] and collects all [Selectable]
  /// render objects, sorted in reading order (top-to-bottom, left-to-right).
  ///
  /// Results are cached to avoid walking the entire tree on every mouse move
  /// during drag selection. The cache is invalidated when layout changes.
  List<Selectable> _collectSelectables(RenderObject root) {
    if (_selectablesCacheValid && _selectablesCache != null) {
      return _selectablesCache!;
    }

    final result = <Selectable>[];
    void visit(RenderObject node) {
      if (node is Selectable) {
        result.add(node);
      }
      node.visitChildren(visit);
    }

    root.visitChildren(visit);

    // Sort by global paint position (y first, then x)
    result.sort((a, b) {
      final aPos = a.globalPaintOffset;
      final bPos = b.globalPaintOffset;
      final yCompare = aPos.dy.compareTo(bPos.dy);
      if (yCompare != 0) return yCompare;
      return aPos.dx.compareTo(bPos.dx);
    });

    _selectablesCache = result;
    _selectablesCacheValid = true;
    return result;
  }

  Object _selectionIdForSelectable(Selectable selectable) {
    final ro = selectable as RenderObject;
    return ro.selectionId ?? ro;
  }

  Object _contextKeyForSelectable(Selectable selectable) {
    final ro = selectable as RenderObject;
    return _nearestScrollViewportAncestor(ro) ?? this;
  }

  int? _listIndexForSelectable(Selectable selectable) {
    RenderObject? node = selectable as RenderObject;
    while (node != null && node != this) {
      final parent = node.parent;
      if (parent is RenderListViewport) {
        final pd = node.parentData;
        if (pd is ListViewParentData) {
          return pd.index;
        }
      }
      node = parent;
    }
    return null;
  }

  /// Builds context lists from selectables, using cache when available.
  Map<Object, List<_SelectableEntry>> _buildContextLists(
      List<Selectable> selectables) {
    if (_selectablesCacheValid && _contextListsCache != null) {
      return _contextListsCache!;
    }

    final lists = <Object, List<_SelectableEntry>>{};
    for (final selectable in selectables) {
      final ro = selectable as RenderObject;
      final origin = selectable.globalPaintOffset;
      final bounds = Rect.fromLTWH(
        origin.dx,
        origin.dy,
        ro.hasSize ? ro.size.width : 0,
        ro.hasSize ? ro.size.height : 0,
      );
      final contextKey = _contextKeyForSelectable(selectable);
      final entry = _SelectableEntry(
        selectable: selectable,
        id: _selectionIdForSelectable(selectable),
        contextKey: contextKey,
        bounds: bounds,
        listIndex: _listIndexForSelectable(selectable),
      );
      lists.putIfAbsent(contextKey, () => []).add(entry);
    }

    for (final list in lists.values) {
      list.sort(_compareByPosition);
    }

    _contextListsCache = lists;
    return lists;
  }

  int _compareByPosition(_SelectableEntry a, _SelectableEntry b) {
    final yCompare = a.bounds.top.compareTo(b.bounds.top);
    if (yCompare != 0) return yCompare;
    return a.bounds.left.compareTo(b.bounds.left);
  }

  _SelectableEntry? _entryForId(
      List<_SelectableEntry> entries, Object selectableId) {
    for (final entry in entries) {
      if (entry.id == selectableId) return entry;
    }
    return null;
  }

  _SelectableEntry? _entryByClampedIndex(
      List<_SelectableEntry> entries, int? index) {
    if (entries.isEmpty || index == null) return null;
    final clamped = index.clamp(0, entries.length - 1);
    return entries[clamped];
  }

  // Hit testing selectables
  /// Returns the [Selectable] whose bounds contain [globalPos], or null.
  Selectable? _hitTestSelectables(Offset globalPos) {
    final baseBounds = _selectionAreaBounds();
    for (final s in _cachedSelectables) {
      final origin = s.globalPaintOffset;
      final ro = s as RenderObject;
      if (!ro.hasSize) continue;
      final sBounds = Rect.fromLTWH(
        origin.dx,
        origin.dy,
        ro.size.width,
        ro.size.height,
      );
      final clipBounds = _clipBoundsForSelectable(s, baseBounds);
      if (clipBounds == null) continue;
      final visibleBounds = _intersectRect(sBounds, clipBounds);
      if (visibleBounds == null) continue;
      if (globalPos.dx >= visibleBounds.left &&
          globalPos.dx < visibleBounds.right &&
          globalPos.dy >= visibleBounds.top &&
          globalPos.dy < visibleBounds.bottom) {
        return s;
      }
    }
    return null;
  }

  /// Returns the nearest [Selectable] to [globalPos] when the pointer is
  /// outside all selectables (e.g. dragging past the last line).
  Selectable? _hitTestSelectablesInContext(
    Offset globalPos,
    List<_SelectableEntry> entries,
  ) {
    final baseBounds = _selectionAreaBounds();
    for (final entry in entries) {
      final s = entry.selectable;
      final ro = s as RenderObject;
      if (!ro.hasSize) continue;
      final sBounds = entry.bounds;
      final clipBounds = _clipBoundsForSelectable(s, baseBounds);
      if (clipBounds == null) continue;
      final visibleBounds = _intersectRect(sBounds, clipBounds);
      if (visibleBounds == null) continue;
      if (globalPos.dx >= visibleBounds.left &&
          globalPos.dx < visibleBounds.right &&
          globalPos.dy >= visibleBounds.top &&
          globalPos.dy < visibleBounds.bottom) {
        return s;
      }
    }
    return null;
  }

  Selectable? _nearestSelectableInContext(
    Offset globalPos,
    List<_SelectableEntry> entries,
  ) {
    if (entries.isEmpty) return null;

    final baseBounds = _selectionAreaBounds();
    Selectable? nearest;
    double minDy = double.infinity;
    double minDx = double.infinity;

    for (final entry in entries) {
      final s = entry.selectable;
      final ro = s as RenderObject;
      if (!ro.hasSize) continue;
      final sBounds = entry.bounds;
      final clipBounds = _clipBoundsForSelectable(s, baseBounds);
      if (clipBounds == null) continue;
      final visibleBounds = _intersectRect(sBounds, clipBounds);
      if (visibleBounds == null) continue;
      // Prefer vertical proximity first, then horizontal proximity.
      final dy = globalPos.dy < visibleBounds.top
          ? visibleBounds.top - globalPos.dy
          : (globalPos.dy >= visibleBounds.bottom
              ? globalPos.dy - (visibleBounds.bottom - 1)
              : 0.0);
      final dx = globalPos.dx < visibleBounds.left
          ? visibleBounds.left - globalPos.dx
          : (globalPos.dx >= visibleBounds.right
              ? globalPos.dx - (visibleBounds.right - 1)
              : 0.0);
      if (dy < minDy || (dy == minDy && dx < minDx)) {
        minDy = dy;
        minDx = dx;
        nearest = s;
      }
    }

    return nearest;
  }

  int _anchorIndexFromPress(Selectable selectable) {
    final ro = selectable as RenderObject;
    if (!ro.hasSize) {
      final localOffset = _globalToSelectableLocal(_pressPosition, selectable);
      return selectable.getCharacterIndexAtLocalPosition(localOffset);
    }
    final origin = selectable.globalPaintOffset;
    final top = origin.dy;
    final bottom = origin.dy + ro.size.height;
    if (_pressPosition.dy < top) {
      return 0;
    }
    if (_pressPosition.dy >= bottom) {
      return selectable.selectableText.length;
    }
    final localOffset = _globalToSelectableLocal(_pressPosition, selectable);
    return selectable.getCharacterIndexAtLocalPosition(localOffset);
  }

  int _focusIndexFromPress(
    Selectable selectable,
    Offset globalPos,
    Offset localOffset,
  ) {
    final ro = selectable as RenderObject;
    if (!ro.hasSize) {
      return selectable.getCharacterIndexAtLocalPosition(localOffset);
    }
    final origin = selectable.globalPaintOffset;
    final top = origin.dy;
    final bottom = origin.dy + ro.size.height;
    final isInside = globalPos.dy >= top && globalPos.dy < bottom;
    if (globalPos.dy < top) {
      return 0;
    }
    if (globalPos.dy >= bottom) {
      return selectable.selectableText.length;
    }
    // If press started below this selectable and we're dragging upward into it,
    // snap focus to end-of-line for initial row entry.
    if (!isInside &&
        _pressPosition.dy >= bottom &&
        globalPos.dy < _pressPosition.dy) {
      return selectable.selectableText.length;
    }
    return selectable.getCharacterIndexAtLocalPosition(localOffset);
  }

  /// Converts a position in the SelectionArea's coordinate space to a
  /// position local to the given [Selectable].
  Offset _globalToSelectableLocal(Offset globalPos, Selectable selectable) {
    final origin = selectable.globalPaintOffset;
    return Offset(globalPos.dx - origin.dx, globalPos.dy - origin.dy);
  }

  // Cross-widget selection logic
  /// Updates the per-widget selection ranges based on the current anchor and
  /// focus positions.
  void _updateSelectionRanges(
    List<_SelectableEntry> entries,
    _SelectionPosition anchor,
    _SelectionPosition focus,
  ) {
    final anchorEntry = _entryForId(entries, anchor.selectableId);
    final focusEntry = _entryForId(entries, focus.selectableId);
    if (anchorEntry == null || focusEntry == null) return;

    final anchorIdx = entries.indexOf(anchorEntry);
    final focusIdx = entries.indexOf(focusEntry);
    if (anchorIdx < 0 || focusIdx < 0) return;

    final forward = anchorIdx < focusIdx ||
        (anchorIdx == focusIdx && anchor.offset <= focus.offset);

    int startIdx = math.min(anchorIdx, focusIdx);
    int endIdx = math.max(anchorIdx, focusIdx);

    for (int i = 0; i < entries.length; i++) {
      final s = entries[i].selectable;
      s.selectionColor = _selectionColor;

      if (i < startIdx || i > endIdx) {
        // Outside the selection span
        s.clearSelection();
      } else if (anchorIdx == focusIdx) {
        // Single widget selection
        final len = s.selectableText.length;
        final start = anchor.offset.clamp(0, len);
        final end = focus.offset.clamp(0, len);
        s.setSelectionRange(start, end);
      } else if (i == anchorIdx) {
        // Anchor widget
        if (forward) {
          final len = s.selectableText.length;
          final start = anchor.offset.clamp(0, len);
          s.setSelectionRange(start, len);
        } else {
          final len = s.selectableText.length;
          final end = anchor.offset.clamp(0, len);
          s.setSelectionRange(0, end);
        }
      } else if (i == focusIdx) {
        // Focus widget
        if (forward) {
          final len = s.selectableText.length;
          final end = focus.offset.clamp(0, len);
          s.setSelectionRange(0, end);
        } else {
          final len = s.selectableText.length;
          final start = focus.offset.clamp(0, len);
          s.setSelectionRange(start, len);
        }
      } else {
        // Widgets between anchor and focus: fully selected
        s.setSelectionRange(0, s.selectableText.length);
      }
    }
  }

  /// Notifies the callback with the currently selected text.
  void _notifySelectionChanged() {
    if (onSelectionChanged == null) return;
    onSelectionChanged!(_collectSelectedText());
  }

  String _collectSelectedText() {
    return _collectSelectedTextFromVisible();
  }

  String _collectSelectedTextFromVisible() {
    final buf = StringBuffer();
    int? lastBottomRow;

    for (final s in _cachedSelectables) {
      if (!s.hasSelection) continue;
      if (!_appendSelectedTextFromSelectable(buf, s, lastBottomRow)) {
        continue;
      }
      lastBottomRow = _lastBottomRow;
    }
    return buf.toString();
  }

  @visibleForTesting
  String debugAppendSelectedText({
    required String initial,
    required String text,
    required List<String> lines,
    required int start,
    required int end,
    required int topRow,
    required int height,
    int? lastBottomRow,
  }) {
    final buf = StringBuffer(initial);
    _appendSelectedText(
      buf,
      text: text,
      lines: lines,
      start: start,
      end: end,
      topRow: topRow,
      height: height,
      lastBottomRow: lastBottomRow,
    );
    return buf.toString();
  }

  int? _lastBottomRow;

  bool _appendSelectedTextFromSelectable(
    StringBuffer buf,
    Selectable s,
    int? lastBottomRow,
  ) {
    final text = s.selectableText;
    final len = text.length;
    final rawStart = math.min(s.selectionStart!, s.selectionEnd!);
    final rawEnd = math.max(s.selectionStart!, s.selectionEnd!);
    final start = rawStart.clamp(0, len);
    final end = rawEnd.clamp(0, len);
    if (start >= end) return false;

    final ro = s as RenderObject;
    final origin = s.globalPaintOffset;
    final height = ro.hasSize ? ro.size.height.ceil() : 1;
    final layout = s.selectableLayout;
    final lines = layout?.lines;
    _appendSelectedText(
      buf,
      text: text,
      lines: lines,
      start: start,
      end: end,
      topRow: origin.dy.floor(),
      height: height,
      lastBottomRow: lastBottomRow,
    );
    return true;
  }

  void _appendSelectedText(
    StringBuffer buf, {
    required String text,
    required List<String>? lines,
    required int start,
    required int end,
    required int topRow,
    required int height,
    required int? lastBottomRow,
  }) {
    if (start >= end) return;

    if (lines != null && lines.length > 1) {
      // Multi-line: iterate lines, only include visible ones
      final lineStarts = selection_utils.lineStartOffsets(text, lines);
      for (int li = 0; li < lines.length; li++) {
        final lineRow = topRow + li;

        final lineStart = lineStarts[li];
        final lineEnd = lineStart + lines[li].length;
        final selStart = math.max(start, lineStart);
        final selEnd = math.min(end, lineEnd);
        if (selStart >= selEnd) continue;

        if (buf.isNotEmpty) {
          if (lastBottomRow != null && lineRow > lastBottomRow) {
            buf.write('\n');
          } else {
            buf.write(' ');
          }
        }
        lastBottomRow = lineRow;
        buf.write(text.substring(selStart, selEnd));
      }
      _lastBottomRow = lastBottomRow;
      return;
    }

    // Single-line (or no layout)
    final bottomRow = topRow + height - 1;

    if (buf.isNotEmpty) {
      if (lastBottomRow != null && topRow > lastBottomRow) {
        buf.write('\n');
      } else {
        buf.write(' ');
      }
    }
    _lastBottomRow = bottomRow;
    buf.write(text.substring(start, end));
  }

  /// Global paint offset of this render object.
  Offset _globalPaintOffset() => _globalPaintOffsetOf(this);

  Rect _selectionAreaBounds() {
    final baseOffset = _globalPaintOffset();
    return Rect.fromLTWH(
      baseOffset.dx,
      baseOffset.dy,
      hasSize ? size.width : double.infinity,
      hasSize ? size.height : double.infinity,
    );
  }

  /// Global paint offset of any render object.
  static Offset _globalPaintOffsetOf(RenderObject node) {
    double x = 0;
    double y = 0;
    RenderObject? current = node;
    while (current != null) {
      if (current.parentData is BoxParentData) {
        final pd = current.parentData as BoxParentData;
        x += pd.offset.dx;
        y += pd.offset.dy;
      }
      current = current.parent;
    }
    return Offset(x, y);
  }

  /// Walk the parent chain of [selectable] up to this SelectionArea,
  /// intersecting with any scroll viewport bounds encountered.
  Rect? _clipBoundsForSelectable(Selectable selectable, Rect baseBounds) {
    Rect? clip = baseBounds;
    RenderObject? node = (selectable as RenderObject).parent;
    while (node != null && node != this) {
      if (node is RenderListViewport || node is RenderSingleChildViewport) {
        final vpOffset = _globalPaintOffsetOf(node);
        final vpBounds = Rect.fromLTWH(
          vpOffset.dx,
          vpOffset.dy,
          node.hasSize ? node.size.width : double.infinity,
          node.hasSize ? node.size.height : double.infinity,
        );
        clip = clip == null ? vpBounds : _intersectRect(clip, vpBounds);
        if (clip == null) return null; // fully clipped
      }
      node = node.parent;
    }
    return clip;
  }

  /// Intersect two rects, returning null if the result is empty.
  static Rect? _intersectRect(Rect a, Rect b) {
    final left = math.max(a.left, b.left);
    final top = math.max(a.top, b.top);
    final right = math.min(a.right, b.right);
    final bottom = math.min(a.bottom, b.bottom);
    if (left >= right || top >= bottom) return null;
    return Rect.fromLTRB(left, top, right, bottom);
  }

  RenderObject? _nearestScrollViewportAncestor(RenderObject ro) {
    RenderObject? node = ro.parent;
    while (node != null && node != this) {
      if (node is RenderListViewport || node is RenderSingleChildViewport) {
        return node;
      }
      node = node.parent;
    }
    return null;
  }

  @override
  bool hitTestSelf(Offset position) => true;
}

class _SelectionPosition {
  _SelectionPosition({
    required this.context,
    required this.selectableId,
    required this.offset,
    this.orderIndex,
  });

  final Object context;
  final Object selectableId;
  final int offset;
  int? orderIndex;
}

class _SelectableEntry {
  _SelectableEntry({
    required this.selectable,
    required this.id,
    required this.contextKey,
    required this.bounds,
    required this.listIndex,
  });

  final Selectable selectable;
  final Object id;
  final Object contextKey;
  final Rect bounds;
  final int? listIndex;
}
