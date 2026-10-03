import { execFile } from "node:child_process";
import { statusEvents } from "./status.js";

// D-notify: notification on transition INTO needs-input, edge-triggered
// (the store only emits "transition" on an actual state change, so this
// handler naturally fires once per transition, not once per poll tick).
// Darwin only; a no-op elsewhere. Never throws — a notification failure
// must not affect the daemon.
//
// terminal-notifier is used when installed (brew install terminal-notifier),
// osascript's `display notification` otherwise. Neither opens anything on
// click (p17-tui-only: there is no web wall to open). Checked once, lazily.
let hasTerminalNotifier = null; // null = not yet probed
function probeTerminalNotifier(cb) {
  if (hasTerminalNotifier !== null) return cb(hasTerminalNotifier);
  execFile("which", ["terminal-notifier"], (err) => {
    hasTerminalNotifier = !err;
    cb(hasTerminalNotifier);
  });
}

function notifyDarwin(id) {
  if (process.platform !== "darwin") return;
  const message = `${id} needs input`;
  try {
    probeTerminalNotifier((available) => {
      if (available) {
        execFile(
          "terminal-notifier",
          ["-title", "claude-garage", "-message", message],
          () => {}
        );
      } else {
        const script = `display notification ${JSON.stringify(message)} with title "claude-garage"`;
        execFile("osascript", ["-e", script], () => {});
      }
    });
  } catch {
    // never throw
  }
}

// Visibility contract: the TUI posts here periodically while it is in the
// foreground. "a visible client exists" = any client entry visible && seen
// within TTL_MS. Rides the normal Origin allowlist (the TUI sends no
// Origin), unlike /api/hooks/claude's token scheme.
const TTL_MS = 45_000;
const clients = new Map(); // clientId -> { visible, lastSeen }

function anyVisiblePage() {
  const now = Date.now();
  for (const { visible, lastSeen } of clients.values()) {
    if (visible && now - lastSeen <= TTL_MS) return true;
  }
  return false;
}

export default async function notifyRoutes(app) {
  app.post("/api/ui/visibility", async (req, reply) => {
    const { clientId, visible } = req.body ?? {};
    if (typeof clientId !== "string" || !clientId || typeof visible !== "boolean") {
      return reply
        .code(400)
        .send({ error: "clientId (string) and visible (boolean) are required" });
    }
    clients.set(clientId, { visible, lastSeen: Date.now() });
    return reply.code(200).send({ ok: true });
  });

  statusEvents.on("transition", ({ id, to }) => {
    if (to === "needs-input" && !anyVisiblePage()) {
      app.log.info({ id }, "needs-input notification fired");
      notifyDarwin(id);
    }
  });
}
