import { execFile } from "node:child_process";
import { statusEvents } from "./status.js";

const PORT = Number(process.env.GARAGE_PORT ?? 4747);
const UI_URL = `http://127.0.0.1:${PORT}`;

// D-notify: notification on transition INTO needs-input, edge-triggered
// (the store only emits "transition" on an actual state change, so this
// handler naturally fires once per transition, not once per poll tick).
// Darwin only; a no-op elsewhere. Never throws — a notification failure
// must not affect the daemon.
//
// Clickability: osascript's `display notification` cannot carry a click
// action — it is a dead-end toast. When terminal-notifier is installed
// (brew install terminal-notifier), use it instead: its -open flag makes
// clicking the notification open the pit wall. Checked once, lazily.
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
          ["-title", "claude-garage", "-message", message, "-open", UI_URL],
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

// Visibility contract: the UI posts here periodically while a pit-wall
// page is in the foreground. "a visible page exists" = any client entry
// visible && seen within TTL_MS. Called from the browser, so it stays
// behind the normal Origin allowlist (unlike /api/hooks/claude).
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
