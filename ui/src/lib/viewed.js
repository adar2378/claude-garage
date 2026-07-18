// Client-side "viewed" state for review mode (design D-viewed). Ephemeral
// per-user UI progress, not daemon state: one localStorage key per
// workspace holding {[path]: contentHash}. A file counts as viewed iff its
// stored hash matches the hash of its *current* diff text — an edit since
// last review (different diff content) therefore auto-resets it to unviewed.

const KEY_PREFIX = "garage-viewed:";

// djb2 — fast, non-cryptographic. Collisions are irrelevant here: this is a
// UI convenience (did-the-content-change marker), not a security boundary.
export function hashContent(text) {
  let hash = 5381;
  const str = text ?? "";
  for (let i = 0; i < str.length; i++) {
    hash = (hash * 33) ^ str.charCodeAt(i);
  }
  return (hash >>> 0).toString(36);
}

function storageKey(workspace) {
  return `${KEY_PREFIX}${workspace}`;
}

export function loadViewedMap(workspace) {
  if (!workspace) return {};
  try {
    const raw = localStorage.getItem(storageKey(workspace));
    return raw ? JSON.parse(raw) : {};
  } catch {
    return {};
  }
}

function saveViewedMap(workspace, map) {
  try {
    localStorage.setItem(storageKey(workspace), JSON.stringify(map));
  } catch {
    // best effort — localStorage being unavailable just means viewed state
    // doesn't persist, not a hard failure.
  }
}

export function markViewed(workspace, path, contentHash) {
  if (!workspace) return {};
  const map = loadViewedMap(workspace);
  map[path] = contentHash;
  saveViewedMap(workspace, map);
  return map;
}

// Reconcile stored keys against the current file list on each fetch — a
// file that stopped changing (reverted, committed elsewhere) drops its
// stale viewed entry rather than accumulating forever.
export function pruneViewed(workspace, currentPaths) {
  if (!workspace) return;
  const map = loadViewedMap(workspace);
  const keep = new Set(currentPaths);
  let changed = false;
  for (const path of Object.keys(map)) {
    if (!keep.has(path)) {
      delete map[path];
      changed = true;
    }
  }
  if (changed) saveViewedMap(workspace, map);
}

export function isFileViewed(viewedMap, file) {
  if (!file || !viewedMap) return false;
  return viewedMap[file.path] === hashContent(file.diff ?? "");
}
