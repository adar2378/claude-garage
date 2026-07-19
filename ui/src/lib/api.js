// Thin fetch wrappers around the daemon's HTTP API.
// Kept deliberately dumb: no caching, no retry logic here — callers own
// resilience (App.jsx retries the initial load; SSE drives live updates).

const JSON_HEADERS = { "content-type": "application/json" };

async function parseJsonSafe(res) {
  try {
    return await res.json();
  } catch {
    return {};
  }
}

export async function fetchWorkspaces() {
  const res = await fetch("/api/workspaces");
  if (!res.ok) throw new Error(`GET /api/workspaces failed (${res.status})`);
  return res.json();
}

export async function fetchSessions() {
  const res = await fetch("/api/sessions");
  if (!res.ok) throw new Error(`GET /api/sessions failed (${res.status})`);
  return res.json();
}

export async function putWorkspace(name, dir) {
  const res = await fetch("/api/workspaces", {
    method: "PUT",
    headers: JSON_HEADERS,
    body: JSON.stringify({ name, dir }),
  });
  const body = await parseJsonSafe(res);
  if (!res.ok) {
    throw new Error(body.error || `could not register workspace (${res.status})`);
  }
  return body;
}

// {worktree: true} (design D-wt-ui) spawns the session in an isolated git
// worktree instead of the workspace's registered directory. Third param is
// optional and defaults to {} so every pre-existing call site (just
// workspace + label) keeps working unchanged. 400 (non-git repo — worktree
// requested against a directory that isn't a git repo) surfaces via the
// thrown Error's message same as any other body.error, for inline display
// next to the control that caused it.
export async function createSession(workspace, label, { worktree } = {}) {
  const res = await fetch("/api/sessions", {
    method: "POST",
    headers: JSON_HEADERS,
    body: JSON.stringify(worktree ? { workspace, label, worktree: true } : { workspace, label }),
  });
  const body = await parseJsonSafe(res);
  if (!res.ok) {
    if (res.status === 404) {
      throw new Error(`unknown workspace "${workspace}"`);
    }
    if (res.status === 409) {
      throw new Error(`session "${label}" already exists in ${workspace}`);
    }
    throw new Error(body.error || `could not create session (${res.status})`);
  }
  return body;
}

// {sessionId} (design D-wt-diff) overrides the diff root to the named
// session's worktree/live cwd when it sits outside the registered
// workspace dir — the response then carries {root, branch} alongside the
// usual {files, truncated} shape; callers show `⎇ <branch>` next to the
// workspace name when `branch` is present.
export async function fetchDiff(workspace, sessionId) {
  const qs = sessionId ? `?sessionId=${encodeURIComponent(sessionId)}` : "";
  const res = await fetch(`/api/diff/${encodeURIComponent(workspace)}${qs}`);
  if (!res.ok) throw new Error(`GET /api/diff/${workspace} failed (${res.status})`);
  return res.json();
}

// {workspace} opens the workspace root; {workspace, file, line} opens a
// specific file at a line. 501 (editor CLI missing) and 400 (path
// traversal) surface via the thrown Error's message — callers show it
// inline (design D-editor).
export async function openEditor(workspace, file, line) {
  const body = file ? { workspace, file, line: line ?? 1 } : { workspace };
  const res = await fetch("/api/open-editor", {
    method: "POST",
    headers: JSON_HEADERS,
    body: JSON.stringify(body),
  });
  const resBody = await parseJsonSafe(res);
  if (!res.ok) {
    throw new Error(resBody.error || `could not open editor (${res.status})`);
  }
  return resBody;
}

// {id} restores a single restorable session; {all: true} restores every
// currently-restorable one in a single daemon-side call. Per-workspace
// "restore all" in the rail deliberately does NOT use {all:true} — it
// issues one restoreSession({id}) per session in that workspace so a
// failure in one workspace's deck can't be conflated with another's (see
// WorkspaceRail). 409 (workspace/dir missing, name collision) surfaces via
// the thrown Error's message.
export async function restoreSession(payload) {
  const res = await fetch("/api/sessions/restore", {
    method: "POST",
    headers: JSON_HEADERS,
    body: JSON.stringify(payload),
  });
  const body = await parseJsonSafe(res);
  if (!res.ok) {
    throw new Error(body.error || body.reason || `could not restore session (${res.status})`);
  }
  return body;
}

// Daemon-side native folder picker (design D-picker — browsers never
// expose absolute filesystem paths, so this can't be done client-side).
// Resolves {dir} on a choice or {cancelled:true} on user-cancel (both 200
// per contract); 501 (non-darwin) surfaces via the thrown Error's message
// so callers can fall back to manual path entry.
export async function pickDirectory() {
  const res = await fetch("/api/pick-directory", { method: "POST" });
  const body = await parseJsonSafe(res);
  if (!res.ok) {
    throw new Error(body.error || `directory picker unavailable (${res.status})`);
  }
  return body;
}

// Renames a workspace and everything that embeds its name (live tmux
// sessions, resume metadata) — design D-rename. Resolves
// {name, dir, renamedSessions, failedSessions?}. 404 (unknown workspace)
// and 409 (name taken) surface via the thrown Error's message for inline
// display next to the rename control.
export async function renameWorkspace(oldName, newName) {
  const res = await fetch(`/api/workspaces/${encodeURIComponent(oldName)}`, {
    method: "PATCH",
    headers: JSON_HEADERS,
    body: JSON.stringify({ name: newName }),
  });
  const body = await parseJsonSafe(res);
  if (!res.ok) {
    if (res.status === 404) {
      throw new Error(body.error || `unknown workspace "${oldName}"`);
    }
    if (res.status === 409) {
      throw new Error(body.error || `workspace name already taken: ${newName}`);
    }
    throw new Error(body.error || `could not rename workspace (${res.status})`);
  }
  return body;
}

export function reportVisibility(clientId, visible) {
  fetch("/api/ui/visibility", {
    method: "POST",
    headers: JSON_HEADERS,
    body: JSON.stringify({ clientId, visible }),
    keepalive: true,
  }).catch(() => {
    // best-effort only — losing a visibility ping is not fatal
  });
}

export async function deleteWorkspace(name) {
  const res = await fetch(`/api/workspaces/${encodeURIComponent(name)}`, {
    method: "DELETE",
  });
  if (res.status === 404) throw new Error(`unknown workspace: ${name}`);
  if (!res.ok) throw new Error(`delete failed (${res.status})`);
}

// Kills the REAL tmux session backing {id} and drops its resume metadata
// (design: terminal-cell close control — a deliberate close, not a hide,
// so there's nothing left to restore afterward; see daemon/src/sessions.js
// DELETE /api/sessions/*). 403 (id isn't a garage-managed session) and 404
// (no live tmux session behind this id — e.g. a restorable-only entry)
// surface via the thrown Error's message. Success is 200 with a body of
// {deleted: true, worktree: {path, branch, repoDir} | null} (design
// D-wt-meta) — the worktree record, when present, is what drives the
// finish (merge/discard/keep) prompt after the kill.
export async function deleteSession(id) {
  const res = await fetch(`/api/sessions/${encodeURIComponent(id)}`, {
    method: "DELETE",
  });
  const body = await parseJsonSafe(res);
  if (!res.ok) {
    if (res.status === 403) {
      throw new Error(body.error || "refusing to close a non-garage session");
    }
    if (res.status === 404) {
      throw new Error(body.error || `no such session: ${id}`);
    }
    throw new Error(body.error || `could not close session (${res.status})`);
  }
  return body;
}

// Resolves a dead worktree session's branch (design D-wt-finish) —
// {worktree} is the exact {path, branch, repoDir} record returned by
// deleteSession, since the session (and its metadata) are already gone by
// the time this runs. `action` is "merge" | "discard" | "keep". 409 (merge
// conflict) surfaces the git stderr via the thrown Error's message with
// the worktree and branch left intact so the caller can retry, discard, or
// keep.
export async function finishWorktree(worktree, action) {
  const res = await fetch("/api/worktrees/finish", {
    method: "POST",
    headers: JSON_HEADERS,
    body: JSON.stringify({ worktree, action }),
  });
  const body = await parseJsonSafe(res);
  if (!res.ok) {
    if (res.status === 409) {
      throw new Error(body.error || "merge conflict — resolve manually");
    }
    throw new Error(body.error || `could not finish worktree (${res.status})`);
  }
  return body;
}
