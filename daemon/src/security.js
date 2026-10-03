// Browser pages are confused deputies: any website can fire requests at
// 127.0.0.1. Browsers always send an Origin header; the TUI, claude's hook
// posts, curl and scripts send none. So requests WITHOUT an Origin are
// allowed (a local process needs no browser to reach tmux and already runs
// with the user's privileges), and any Origin is refused unless listed.
//
// p17-tui-only: garage serves no web UI any more, so the default allowlist
// is empty. GARAGE_UI_ORIGINS (comma-separated) can still opt an origin in.
const DEFAULT_ORIGINS = [];

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
