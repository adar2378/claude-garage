// p8 grid-views (LATER.md design, built 2026-07-19): group a workspace's
// sessions into named views — each view its own arrangement, ONE view on
// screen at a time. Views are pure client UI (like layouts): assignments
// live in localStorage keyed per workspace; sessions stay plain tmux
// sessions and the daemon knows nothing about any of this.
//
// Model: every session implicitly belongs to the "main" view unless
// `assignments[sessionId]` names another view. Views are DERIVED from
// (assignments ∩ live session list) on every compute — a view with no
// remaining sessions simply stops existing, so there is no cleanup
// bookkeeping. The default single-view workspace renders exactly as
// before (no strip, no rail tier).
//
// Caveat (documented, accepted): assignments are keyed by full session id
// (`garage/<ws>/<label>`), so renaming a workspace resets its views — the
// ids all change. Same caveat layouts already have.

const KEY_PREFIX = "garage-views:";

export const MAIN_VIEW = "main";

export function loadViews(workspace) {
  try {
    const raw = localStorage.getItem(`${KEY_PREFIX}${workspace}`);
    const parsed = raw ? JSON.parse(raw) : null;
    return {
      assignments: parsed?.assignments ?? {},
      focused: parsed?.focused ?? MAIN_VIEW,
    };
  } catch {
    return { assignments: {}, focused: MAIN_VIEW };
  }
}

export function saveViews(workspace, state) {
  try {
    localStorage.setItem(`${KEY_PREFIX}${workspace}`, JSON.stringify(state));
  } catch {
    // best-effort — losing view assignments degrades to the single main view
  }
}

/**
 * Derive the ordered view list for a group's sessions:
 * [{name, sessions, needsCount}], "main" first, detached views in
 * first-appearance order. Views with no live members vanish naturally.
 */
export function computeViews(sessions, assignments) {
  const byName = new Map();
  byName.set(MAIN_VIEW, []);
  for (const s of sessions) {
    const view = assignments[s.id] ?? MAIN_VIEW;
    if (!byName.has(view)) byName.set(view, []);
    byName.get(view).push(s);
  }
  const views = [];
  for (const [name, members] of byName) {
    if (members.length === 0) continue;
    views.push({
      name,
      sessions: members,
      needsCount: members.filter((s) => s.status === "needs-input").length,
    });
  }
  return views;
}

/** The view a session belongs to (main when unassigned). */
export function viewOf(assignments, sessionId) {
  return assignments[sessionId] ?? MAIN_VIEW;
}

/**
 * A free view name derived from a session label — "test", then "test-2"…
 * against the currently existing view names.
 */
export function deriveViewName(label, existingNames) {
  const names = new Set(existingNames);
  const base = label || "view";
  if (base !== MAIN_VIEW && !names.has(base)) return base;
  let n = 2;
  while (names.has(`${base}-${n}`) || `${base}-${n}` === MAIN_VIEW) n++;
  return `${base}-${n}`;
}
