import Fastify from "fastify";
import sessionRoutes from "./sessions.js";
import { attachTermServer } from "./term.js";
import { rejectForeignOrigins } from "./security.js";

const HOST = "127.0.0.1";
const PORT = Number(process.env.GARAGE_PORT ?? 4747);

const app = Fastify({ logger: { level: "info" } });

app.addHook("onRequest", rejectForeignOrigins);
app.get("/api/health", async () => ({ status: "ok" }));
app.register(sessionRoutes);
attachTermServer(app);

app.listen({ host: HOST, port: PORT }).catch((err) => {
  app.log.error(err);
  process.exit(1);
});
