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

// The panel a newly-joining session splits below (design D-dock: "new
// session -> added as a bottom split of the largest panel"). "Largest" is
// measured by rendered area since that's what's actually visually biggest
// to the user, not by proportion-of-tree.
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

// The zero-config default: every session stacked vertically, top to
// bottom, in `sessionIds` order. Assumes `api` is already empty (callers
// clear first — see loadOrBuildLayout/resetLayout).
export function buildDefault(api, sessionIds) {
  let previousId = null;
  for (const id of sessionIds) {
    api.addPanel({
      id,
      component: COMPONENT,
      position: previousId ? { referencePanel: previousId, direction: "below" } : undefined,
    });
    previousId = id;
  }
}

// Reconciles the live layout against the current session-id set: drops
// panels for ids no longer present, adds panels for ids that are new
// (split below the largest existing panel), leaves everything else's
// arrangement untouched. Safe to call redundantly (e.g. right after a
// fresh build) — it's a no-op when the panel set already matches.
export function reconcile(api, sessionIds) {
  const wanted = new Set(sessionIds);

  for (const panel of [...api.panels]) {
    if (!wanted.has(panel.id)) api.removePanel(panel);
  }

  const present = new Set(api.panels.map((p) => p.id));
  for (const id of sessionIds) {
    if (present.has(id)) continue;
    const target = largestPanel(api);
    api.addPanel({
      id,
      component: COMPONENT,
      position: target ? { referencePanel: target.id, direction: "below" } : undefined,
    });
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
