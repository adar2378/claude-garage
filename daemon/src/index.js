import { existsSync } from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import Fastify from "fastify";
import fastifyStatic from "@fastify/static";
import sessionRoutes from "./sessions.js";
import workspaceRoutes from "./workspaces.js";
import pickerRoutes from "./picker.js";
import hookRoutes from "./hooks.js";
import eventRoutes from "./events.js";
import notifyRoutes from "./notify.js";
import diffRoutes from "./diff.js";
import editorRoutes from "./editor.js";
import { attachTermServer } from "./term.js";
import { rejectForeignOrigins } from "./security.js";
import { startPoller } from "./poller.js";

const HOST = "127.0.0.1";
const PORT = Number(process.env.GARAGE_PORT ?? 4747);

// daemon/src/index.js -> repo root is two levels up.
const __dirname = path.dirname(fileURLToPath(import.meta.url));
const REPO_ROOT = path.resolve(__dirname, "..", "..");

const app = Fastify({ logger: { level: "info" } });

app.addHook("onRequest", rejectForeignOrigins);
app.get("/api/health", async () => ({ status: "ok" }));
app.register(sessionRoutes);
app.register(workspaceRoutes);
app.register(pickerRoutes);
app.register(hookRoutes);
app.register(eventRoutes);
app.register(notifyRoutes);
app.register(diffRoutes);
app.register(editorRoutes);
attachTermServer(app);

// D-packaging: flag-gated so dev mode (Vite on :5173 proxying to this
// daemon) never double-registers a static root — registered last, after
// every /api/* route above, so those routes win over static's own wildcard.
if (process.env.GARAGE_SERVE_UI === "1") {
  const uiDist = path.join(REPO_ROOT, "ui", "dist");
  if (existsSync(uiDist)) {
    app.register(fastifyStatic, { root: uiDist, prefix: "/" });
  } else {
    app.log.error(
      `GARAGE_SERVE_UI is set but ${uiDist} does not exist — run "npm run build --workspace ui" first`
    );
  }
}

// Poller feeds StatusStore (busy/idle baseline); hooks.js and notify.js
// subscribe to the same store — see status.js for the single write path.
const stopPoller = startPoller(app);
app.addHook("onClose", async () => {
  stopPoller();
});

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

export { app };
