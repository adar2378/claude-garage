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

export async function createSession(workspace, label) {
  const res = await fetch("/api/sessions", {
    method: "POST",
    headers: JSON_HEADERS,
    body: JSON.stringify({ workspace, label }),
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

export async function fetchDiff(workspace) {
  const res = await fetch(`/api/diff/${encodeURIComponent(workspace)}`);
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
