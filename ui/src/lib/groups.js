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

// p4-layout-focus-workspace-ux / design D-nesting: derives a containment
// tree from buildGroups()'s flat output — the registry itself stays flat
// (no parent links stored, no drift possible; see design's "Alternative
// rejected"). Nesting is purely presentational and re-derived on every
// call, same as buildGroups() itself.
//
// Returns both:
//  - tree: groups nested under their parent, each node additionally
//    carrying `depth` (0 = top-level) and `children` (possibly empty array).
//  - flattenedGroups: the tree walked pre-order (parent immediately
//    followed by its children) — this IS the rail's render order, so it is
//    also the order the "1"-"9" keybindings must index into. Callers that
//    want indexable/keyboard-matching order should use this array, not
//    walk `tree` themselves.
//
// buildGroups()'s existing signature/behavior is untouched above — this is
// an additive export for callers that want nesting.
export function buildGroupTree(workspaces, sessions) {
  const groups = buildGroups(workspaces, sessions);
  const byName = new Map(groups.map((g) => [g.name, g]));

  // parent = deepest OTHER group whose dir strictly contains this group's
  // dir (path-prefix with separator guard, so "foo-bar" is never treated
  // as nested under "foo"). "Deepest" wins so a 3-level containment chain
  // nests each workspace directly under its immediate parent, not the root.
  const parentOf = new Map();
  for (const g of groups) {
    if (!g.dir) continue;
    let parent = null;
    for (const candidate of groups) {
      if (candidate === g || !candidate.dir) continue;
      if (!isStrictlyInside(g.dir, candidate.dir)) continue;
      if (!parent || candidate.dir.length > parent.dir.length) parent = candidate;
    }
    if (parent) parentOf.set(g.name, parent.name);
  }

  const childrenOf = new Map();
  for (const g of groups) {
    const parentName = parentOf.get(g.name);
    if (!parentName) continue;
    if (!childrenOf.has(parentName)) childrenOf.set(parentName, []);
    childrenOf.get(parentName).push(g);
  }

  // Needs-you-first ordering, but bubbled through the whole subtree: a
  // needs-input session nested three levels deep still pulls its top-level
  // ancestor (and every ancestor in between) to the front of its siblings.
  function subtreeNeedsInput(name) {
    const g = byName.get(name);
    if (g && hasNeedsInput(g)) return true;
    return (childrenOf.get(name) ?? []).some((c) => subtreeNeedsInput(c.name));
  }

  function buildNode(g, depth) {
    const kids = [...(childrenOf.get(g.name) ?? [])].sort(
      (a, b) => rank(subtreeNeedsInput(a.name)) - rank(subtreeNeedsInput(b.name))
    );
    return { ...g, depth, children: kids.map((k) => buildNode(k, depth + 1)) };
  }

  const topLevel = groups
    .filter((g) => !parentOf.has(g.name))
    .sort((a, b) => rank(subtreeNeedsInput(a.name)) - rank(subtreeNeedsInput(b.name)));

  const tree = topLevel.map((g) => buildNode(g, 0));

  const flattenedGroups = [];
  (function flatten(nodes) {
    for (const node of nodes) {
      flattenedGroups.push(node);
      flatten(node.children);
    }
  })(tree);

  return { tree, flattenedGroups };
}

function isStrictlyInside(childDir, parentDir) {
  if (!childDir || !parentDir || childDir === parentDir) return false;
  const base = parentDir.endsWith("/") ? parentDir : `${parentDir}/`;
  return childDir.startsWith(base);
}
