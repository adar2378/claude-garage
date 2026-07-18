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
