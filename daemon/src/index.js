import Fastify from "fastify";
import sessionRoutes from "./sessions.js";
import workspaceRoutes from "./workspaces.js";
import pickerRoutes from "./picker.js";
import hookRoutes from "./hooks.js";
import statuslineRoutes from "./statusline.js";
import eventRoutes from "./events.js";
import notifyRoutes from "./notify.js";
import diffRoutes from "./diff.js";
import editorRoutes from "./editor.js";
import worktreeRoutes from "./worktrees.js";
import daemonRestartRoutes, { waitForPredecessor } from "./daemon-restart.js";
import { rejectForeignOrigins } from "./security.js";
import { startPoller } from "./poller.js";
import { healthPayload } from "./health.js";
import { ensureExtendedKeys } from "./tmux.js";

const HOST = "127.0.0.1";
const PORT = Number(process.env.GARAGE_PORT ?? 4747);

// forceCloseConnections: close() actively terminates keep-alive and
// hijacked connections (SSE streams) instead of waiting for them to
// drain — they never would; see daemon-restart.js.
const app = Fastify({ logger: { level: "info" }, forceCloseConnections: true });

app.addHook("onRequest", rejectForeignOrigins);
// version + pid ride along for the launcher's stale-daemon gate
// (bin/garage.js, p8.2) — see health.js.
app.get("/api/health", async () => healthPayload());
app.register(sessionRoutes);
app.register(workspaceRoutes);
app.register(pickerRoutes);
app.register(hookRoutes);
app.register(statuslineRoutes);
app.register(eventRoutes);
app.register(notifyRoutes);
app.register(diffRoutes);
app.register(editorRoutes);
app.register(worktreeRoutes);
app.register(daemonRestartRoutes);

// Poller feeds StatusStore (busy/idle baseline); hooks.js and notify.js
// subscribe to the same store — see status.js for the single write path.
const stopPoller = startPoller(app);
// p14: Shift+Enter needs the tmux server's extended-keys config — apply it
// to an already-running server at boot (createSession re-applies later).
ensureExtendedKeys();
app.addHook("onClose", async () => {
  stopPoller();
});

// p16-restart D3/D6: a successor spawned by POST /api/daemon/restart carries
// GARAGE_PREDECESSOR_PID — wait for that pid's /api/health to stop answering
// (port free) before binding it, so the handoff never races two daemons for
// one port. Fails loud (non-zero exit, listen() never called) if the
// predecessor doesn't let go within 10s, naming the port and pid — see
// daemon-restart.js. `readyToListen` (rather than exiting straight from the
// catch below) keeps this a single, unambiguous control-flow path down to
// exactly one of listen() or process.exit(1) — never both.
let readyToListen = true;
if (process.env.GARAGE_PREDECESSOR_PID) {
  await waitForPredecessor({
    port: PORT,
    pid: Number(process.env.GARAGE_PREDECESSOR_PID),
  }).catch((err) => {
    app.log.error(err);
    readyToListen = false;
  });
}

if (readyToListen) {
  app.listen({ host: HOST, port: PORT }).catch((err) => {
    if (err.code === "EADDRINUSE") {
      app.log.error(
        `port ${PORT} is already in use — set GARAGE_PORT to choose a different port`
      );
    } else {
      app.log.error(err);
    }
    process.exit(1);
  });
} else {
  process.exit(1);
}

export { app };
