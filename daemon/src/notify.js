import { execFile } from "node:child_process";
import { statusEvents } from "./status.js";

// D-notify: notification on transition INTO needs-input, edge-triggered
// (the store only emits "transition" on an actual state change, so this
// handler naturally fires once per transition, not once per poll tick).
// macOS (terminal-notifier / osascript) and Windows (toast); a no-op elsewhere. Never throws — a notification failure
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

// Windows: a toast through the WinRT notification API, sent via the stock
// PowerShell's own app id (no extra modules). The message is built from a
// session id, but single quotes are doubled anyway.
function notifyWindows(id) {
  const message = `${id} needs input`.replaceAll("'", "''");
  const script =
    "[Windows.UI.Notifications.ToastNotificationManager, Windows.UI.Notifications, ContentType = WindowsRuntime] > $null; " +
    "$t = [Windows.UI.Notifications.ToastNotificationManager]::GetTemplateContent([Windows.UI.Notifications.ToastTemplateType]::ToastText02); " +
    "$n = $t.GetElementsByTagName('text'); " +
    "$n.Item(0).AppendChild($t.CreateTextNode('claude-garage')) > $null; " +
    `$n.Item(1).AppendChild($t.CreateTextNode('${message}')) > $null; ` +
    "$app = '{1AC14E77-02E7-4E5D-B744-2EB1AE5198B7}\\WindowsPowerShell\\v1.0\\powershell.exe'; " +
    "[Windows.UI.Notifications.ToastNotificationManager]::CreateToastNotifier($app).Show([Windows.UI.Notifications.ToastNotification]::new($t))";
  try {
    execFile(
      "powershell.exe",
      ["-NoProfile", "-NonInteractive", "-WindowStyle", "Hidden", "-Command", script],
      { windowsHide: true },
      () => {}
    );
  } catch {
    // never throw
  }
}

function notify(id) {
  if (process.platform === "win32") notifyWindows(id);
  else notifyDarwin(id);
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
      notify(id);
    }
  });
}
