// Browser pages are confused deputies: any website can fire requests at
// 127.0.0.1, and WebSockets are exempt from CORS. Browsers always send an
// Origin header, so we allowlist the UI's origin. Requests WITHOUT an Origin
// (curl, scripts) are allowed — a local process needs no browser to reach
// tmux and already runs with the user's privileges.
//
// D-packaging: in same-process serving mode (GARAGE_SERVE_UI=1) the UI is
// served from the daemon's own origin instead of Vite's :5173, so that
// origin (following GARAGE_PORT, same as index.js) must be allowlisted too —
// otherwise the served UI's own fetch/EventSource calls would be rejected as
// foreign.
const PORT = Number(process.env.GARAGE_PORT ?? 4747);
const DEFAULT_ORIGINS = [
  "http://127.0.0.1:5173",
  "http://localhost:5173",
  `http://127.0.0.1:${PORT}`,
  `http://localhost:${PORT}`,
];

export const ALLOWED_ORIGINS = new Set(
  process.env.GARAGE_UI_ORIGINS?.split(",").map((s) => s.trim()) ??
    DEFAULT_ORIGINS
);

export function originAllowed(origin) {
  return origin === undefined || ALLOWED_ORIGINS.has(origin);
}

const STATE_CHANGING = new Set(["POST", "PUT", "PATCH", "DELETE"]);

// No exemptions: claude's hook posts carry no Origin header, so they pass
// the allowlist naturally (header-less = ordinary local process). The hook
// endpoint additionally requires a per-install token — see hooks.js.
export async function rejectForeignOrigins(req, reply) {
  if (!STATE_CHANGING.has(req.method)) return;
  if (!originAllowed(req.headers.origin)) {
    return reply.code(403).send({ error: "origin not allowed" });
  }
}
