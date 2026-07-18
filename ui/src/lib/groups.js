// Derives the rail/grid's ordered workspace groups from the two raw API
// collections. Pure + side-effect-free so it is trivial to reason about and
// to recompute on every sessions/workspaces change.
//
// Ordering rules (spec: pit-wall-ui "Needs-you-first ordering"):
//  - workspaces containing >=1 needs-input session sort before the rest,
//    stable otherwise (registration/discovery order preserved within each
//    bucket).
//  - within a workspace, needs-input sessions sort before the rest, stable
//    otherwise.
//
// Sessions whose workspace isn't in the registry still get a synthesized
// group (registered: false) so nothing is invisible (spec: 3.1).

export function buildGroups(workspaces, sessions) {
  const byName = new Map();

  for (const ws of workspaces ?? []) {
    byName.set(ws.name, {
      name: ws.name,
      dir: ws.dir,
      registered: true,
      sessions: [],
    });
  }

  for (const s of sessions ?? []) {
    let group = byName.get(s.workspace);
    if (!group) {
      group = {
        name: s.workspace,
        dir: s.dir,
        registered: false,
        sessions: [],
      };
      byName.set(s.workspace, group);
    }
    group.sessions.push(s);
  }

  const groups = [...byName.values()];

  // Array#sort is stable per spec (ES2019+); we rely on that for the
  // "stable otherwise" clause instead of tracking original index by hand.
  groups.sort((a, b) => rank(hasNeedsInput(a)) - rank(hasNeedsInput(b)));

  return groups.map((g) => ({
    ...g,
    sessions: [...g.sessions].sort(
      (a, b) => rank(a.status === "needs-input") - rank(b.status === "needs-input")
    ),
  }));
}

function hasNeedsInput(group) {
  return group.sessions.some((s) => s.status === "needs-input");
}

function rank(isBlocked) {
  return isBlocked ? 0 : 1;
}
