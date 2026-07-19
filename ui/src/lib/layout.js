// dockview layout persistence, one layout per workspace (design D-dock).
// Serialized to localStorage under `garage-layout:<workspace>`.
//
// `reconcile`/`buildDefault` operate on a *live* DockviewApi rather than on
// raw serialized JSON: dockview's `fromJSON`/`toJSON` round-trip the
// current panel set, but reconciling a *persisted* view against a *live*
// session list — dropping panels for sessions that vanished, splitting new
// ones in next to the biggest existing panel — is far more robustly
// expressed against `api.addPanel`/`api.removePanel` (which know how to
// keep the grid tree consistent) than by hand-walking and mutating the
// serialized grid tree ourselves. TerminalGrid.jsx is the only caller.

const KEY_PREFIX = "garage-layout:";
const COMPONENT = "terminal";

export function storageKey(workspace) {
  return `${KEY_PREFIX}${workspace}`;
}

export function loadLayout(workspace) {
  try {
    const raw = localStorage.getItem(storageKey(workspace));
    return raw ? JSON.parse(raw) : null;
  } catch {
    return null;
  }
}

export function saveLayout(workspace, api) {
  try {
    localStorage.setItem(storageKey(workspace), JSON.stringify(api.toJSON()));
  } catch {
    // best-effort — a full/blocked localStorage just means the next load
    // falls back to the default stack (see loadOrBuildLayout), which is a
    // safe degradation.
  }
}

export function clearLayout(workspace) {
  try {
    localStorage.removeItem(storageKey(workspace));
  } catch {
    // ignore
  }
}

// One-shot placement hints (p7 D-grid-controls): an explicit split control
// knows exactly where the joining session's panel should dock (which cell,
// which direction), but the panel is only created later, by reconcile(),
// after the spawn's sessions refetch lands. The control registers a hint
// keyed by the id it is about to create; reconcile consumes it ahead of
// the longest-axis heuristic. Entries expire after 10s (a failed spawn
// must not misplace an unrelated later join) and are dropped if their
// reference panel no longer exists.
const HINT_TTL_MS = 10_000;
const placementHints = new Map(); // id -> { referencePanel, direction, at }

export function registerPlacementHint(id, referencePanel, direction) {
  placementHints.set(id, { referencePanel, direction, at: Date.now() });
}

function takePlacementHint(api, id) {
  const hint = placementHints.get(id);
  if (!hint) return null;
  placementHints.delete(id);
  if (Date.now() - hint.at > HINT_TTL_MS) return null;
  if (!api.getPanel(hint.referencePanel)) return null;
  return hint;
}

// The panel a newly-joining session splits into (p7 D-grid-join). "Largest"
// is measured by rendered area since that's what's actually visually
// biggest to the user, not by proportion-of-tree.
function largestPanel(api) {
  let best = null;
  let bestArea = -1;
  for (const panel of api.panels) {
    const area = (panel.api.width || 0) * (panel.api.height || 0);
    if (area > bestArea) {
      bestArea = area;
      best = panel;
    }
  }
  return best;
}

// The zero-config default (p7 D-grid-default): a balanced near-square
// grid — ceil(sqrt(n)) columns, filled row-major in `sessionIds` order —
// instead of the old full-width vertical stack. Expressed through
// addPanel reference/direction math so dockview keeps the grid tree
// consistent: panel i docks "right" of its row neighbor, or "below" the
// panel one row up when it starts a new row. 4 sessions -> 2×2, 5-6 ->
// 3 columns, etc. Assumes `api` is already empty (callers clear first —
// see loadOrBuildLayout/resetLayout).
export function buildDefault(api, sessionIds) {
  const cols = Math.ceil(Math.sqrt(sessionIds.length));
  sessionIds.forEach((id, i) => {
    let position;
    if (i > 0 && i < cols) {
      // First row: dock right of the previous panel — this establishes
      // the columns.
      position = { referencePanel: sessionIds[i - 1], direction: "right" };
    } else if (i >= cols) {
      // Every later row: dock below the COLUMN neighbor (i - cols), never
      // "right of the previous" — a right-split of a second-row panel
      // would subdivide that panel's own cell instead of aligning under
      // the next column (verified against dockview's split semantics
      // during p7 e2e).
      position = { referencePanel: sessionIds[i - cols], direction: "below" };
    }
    api.addPanel({ id, component: COMPONENT, position });
  });
}

// Reconciles the live layout against the current session-id set: drops
// panels for ids no longer present, adds panels for ids that are new
// (split below the largest existing panel), leaves everything else's
// arrangement untouched. Safe to call redundantly (e.g. right after a
// fresh build) — it's a no-op when the panel set already matches.
export function reconcile(api, sessionIds) {
  const wanted = new Set(sessionIds);

  for (const panel of [...api.panels]) {
    if (!wanted.has(panel.id)) {
      // Contained: dockview can throw "resource already disposed" when a
      // removal races its own internal cleanup (e.g. removal initiated
      // from inside the panel's tab). The panel is gone either way; one
      // bad dispose must not abort reconciling the rest.
      try {
        api.removePanel(panel);
      } catch {
        /* already disposed */
      }
    }
  }

  const present = new Set(api.panels.map((p) => p.id));
  for (const id of sessionIds) {
    if (present.has(id)) continue;
    // Explicit split hint wins; otherwise split the largest panel along
    // its longer axis (p7 D-grid-join) — "right" when wider than tall —
    // the same heuristic VS Code uses to keep terminal splits balanced,
    // replacing the old always-"below" that degenerated into ever-thinner
    // full-width rows.
    const hint = takePlacementHint(api, id);
    let position;
    if (hint) {
      position = { referencePanel: hint.referencePanel, direction: hint.direction };
    } else {
      const target = largestPanel(api);
      if (target) {
        const wide = (target.api.width || 0) >= (target.api.height || 0);
        position = { referencePanel: target.id, direction: wide ? "right" : "below" };
      }
    }
    api.addPanel({ id, component: COMPONENT, position });
  }
}

// Loads `workspace`'s persisted layout into `api` if there is one and it
// survives a `fromJSON` round-trip; otherwise falls back to the default
// vertical stack (design: "layout invalid/empty -> default vertical
// stack"). Either way finishes with a reconcile pass so a layout captured
// before a session died/joined self-heals immediately (design: "reconciled
// on every session-set change").
export function loadOrBuildLayout(api, workspace, sessionIds) {
  const persisted = loadLayout(workspace);
  if (persisted) {
    try {
      api.fromJSON(persisted);
      reconcile(api, sessionIds);
      return;
    } catch {
      // corrupt/incompatible snapshot — fall through and treat the same as
      // "no layout".
    }
  }
  api.clear();
  buildDefault(api, sessionIds);
}

// The reset-layout control's action: discard the persisted layout and
// rebuild the default stack from the current session set.
export function resetLayout(api, workspace, sessionIds) {
  clearLayout(workspace);
  api.clear();
  buildDefault(api, sessionIds);
}
