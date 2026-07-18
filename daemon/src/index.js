import Fastify from "fastify";
import sessionRoutes from "./sessions.js";
import workspaceRoutes from "./workspaces.js";
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

const app = Fastify({ logger: { level: "info" } });

app.addHook("onRequest", rejectForeignOrigins);
app.get("/api/health", async () => ({ status: "ok" }));
app.register(sessionRoutes);
app.register(workspaceRoutes);
app.register(hookRoutes);
app.register(eventRoutes);
app.register(notifyRoutes);
app.register(diffRoutes);
app.register(editorRoutes);
attachTermServer(app);

// Poller feeds StatusStore (busy/idle baseline); hooks.js and notify.js
// subscribe to the same store — see status.js for the single write path.
const stopPoller = startPoller(app);
app.addHook("onClose", async () => {
  stopPoller();
});

app.listen({ host: HOST, port: PORT }).catch((err) => {
  app.log.error(err);
  process.exit(1);
});
