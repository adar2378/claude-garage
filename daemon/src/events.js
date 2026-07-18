import { statusEvents } from "./status.js";
import { pollerEvents } from "./poller.js";

const KEEPALIVE_MS = 15_000;

// D-push: GET /api/events as SSE. One-directional daemon -> UI, so SSE over
// a plain reply.raw loop — no new dependency, EventSource auto-reconnects.
const activeStreams = new Set();

export default async function eventRoutes(app) {
  app.get("/api/events", async (req, reply) => {
    reply.hijack();
    const res = reply.raw;
    res.writeHead(200, {
      "Content-Type": "text/event-stream",
      "Cache-Control": "no-cache",
      Connection: "keep-alive",
    });
    res.write(": connected\n\n");
    activeStreams.add(res);

    const send = (event, data) => {
      res.write(`event: ${event}\ndata: ${JSON.stringify(data)}\n\n`);
    };

    const onTransition = ({ id, to }) => send("status", { id, status: to });
    const onSessionsChanged = () => send("sessions", {});

    statusEvents.on("transition", onTransition);
    pollerEvents.on("sessions-changed", onSessionsChanged);

    const keepalive = setInterval(() => {
      res.write(": keepalive\n\n");
    }, KEEPALIVE_MS);
    keepalive.unref?.();

    const cleanup = () => {
      clearInterval(keepalive);
      statusEvents.off("transition", onTransition);
      pollerEvents.off("sessions-changed", onSessionsChanged);
      activeStreams.delete(res);
    };

    req.raw.on("close", cleanup);
    res.on("error", cleanup);
  });

  app.addHook("onClose", async () => {
    for (const res of activeStreams) {
      res.end();
    }
    activeStreams.clear();
  });
}
