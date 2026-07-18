// Browser pages are confused deputies: any website can fire requests at
// 127.0.0.1, and WebSockets are exempt from CORS. Browsers always send an
// Origin header, so we allowlist the UI's origin. Requests WITHOUT an Origin
// (curl, scripts) are allowed — a local process needs no browser to reach
// tmux and already runs with the user's privileges.
const DEFAULT_ORIGINS = ["http://127.0.0.1:5173", "http://localhost:5173"];

export const ALLOWED_ORIGINS = new Set(
  process.env.GARAGE_UI_ORIGINS?.split(",").map((s) => s.trim()) ??
    DEFAULT_ORIGINS
);

export function originAllowed(origin) {
  return origin === undefined || ALLOWED_ORIGINS.has(origin);
}

const STATE_CHANGING = new Set(["POST", "PUT", "PATCH", "DELETE"]);

export async function rejectForeignOrigins(req, reply) {
  if (STATE_CHANGING.has(req.method) && !originAllowed(req.headers.origin)) {
    return reply.code(403).send({ error: "origin not allowed" });
  }
}
