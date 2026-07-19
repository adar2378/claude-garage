// p6-drag-resize: persistence + drag mechanics for the resizable
// rail/pane columns (App.jsx) and the changes-pane list/diff split
// (ChangesPane.jsx). One storage key, one shape, following the same
// best-effort try/catch pattern as lib/layout.js.

const STORAGE_KEY = "garage-pane-sizes";

const DEFAULTS = { railW: 240, paneW: 360, paneSplit: 0.4 };

export function loadPaneSizes() {
  try {
    const raw = localStorage.getItem(STORAGE_KEY);
    if (!raw) return { ...DEFAULTS };
    const parsed = JSON.parse(raw);
    return { ...DEFAULTS, ...parsed };
  } catch {
    return { ...DEFAULTS };
  }
}

// Merges `partial` into whatever is currently persisted (not whatever the
// caller's in-memory state happens to be) so a caller that only owns one
// field of the shape (e.g. ChangesPane only ever touches `paneSplit`) can't
// clobber a sibling field it never read.
export function savePaneSizes(partial) {
  try {
    const next = { ...loadPaneSizes(), ...partial };
    localStorage.setItem(STORAGE_KEY, JSON.stringify(next));
  } catch {
    // best-effort — a full/blocked localStorage just means the size resets
    // to defaults next load, which is a safe degradation.
  }
}

// Generic drag-to-resize helper: attaches window-level mousemove/mouseup
// listeners starting from a mousedown event, reporting the cumulative
// pointer delta (relative to the mousedown point, not incremental) to
// `onMove(dx, dy)` on every move, then `onEnd()` once on mouseup. Callers
// compute their own clamped size from `startSize + dx` (or `- dx`) inside
// onMove — this helper owns no sizing knowledge.
//
// Disables text selection for the duration of the drag (dragging a 4px
// divider fast enough otherwise selects surrounding text) and restores
// whatever `body.style.userSelect` was beforehand.
export function startDrag(e, { onMove, onEnd } = {}) {
  e.preventDefault();
  const startX = e.clientX;
  const startY = e.clientY;
  const previousUserSelect = document.body.style.userSelect;
  document.body.style.userSelect = "none";

  function handleMove(moveEvent) {
    const dx = moveEvent.clientX - startX;
    const dy = moveEvent.clientY - startY;
    onMove?.(dx, dy);
  }

  function handleUp() {
    window.removeEventListener("mousemove", handleMove);
    window.removeEventListener("mouseup", handleUp);
    document.body.style.userSelect = previousUserSelect;
    onEnd?.();
  }

  window.addEventListener("mousemove", handleMove);
  window.addEventListener("mouseup", handleUp);
}
