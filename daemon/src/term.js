import { WebSocketServer } from "ws";
import pty from "node-pty";
import { GARAGE_PREFIX } from "./tmux.js";
import { originAllowed } from "./security.js";

const SWEEP_MS = 30_000;

// ws → pty. The socket owns the pty: socket dies → pty dies (detach).
// The tmux session itself is never touched from here.
const bridges = new Map();

export function attachTermServer(app) {
  const wss = new WebSocketServer({ noServer: true });

  app.server.on("upgrade", (req, socket, head) => {
    // WebSockets bypass CORS — block browser pages from foreign origins.
    if (!originAllowed(req.headers.origin)) {
      socket.destroy();
      return;
    }
    const url = new URL(req.url, "http://127.0.0.1");
    if (!url.pathname.startsWith("/term/")) {
      socket.destroy();
      return;
    }
    const id = decodeURIComponent(url.pathname.slice("/term/".length));
    if (!id.startsWith(GARAGE_PREFIX)) {
      socket.destroy();
      return;
    }
    wss.handleUpgrade(req, socket, head, (ws) => bridge(app, ws, id));
  });

  const sweep = setInterval(() => {
    for (const [ws, term] of bridges) {
      if (ws.readyState === ws.CLOSED || ws.readyState === ws.CLOSING) {
        term.kill();
        bridges.delete(ws);
      }
    }
  }, SWEEP_MS);
  app.addHook("onClose", async () => {
    clearInterval(sweep);
    for (const term of bridges.values()) term.kill();
    bridges.clear();
  });
}

function bridge(app, ws, id) {
  let term;
  try {
    term = pty.spawn("tmux", ["attach", "-t", `=${id}`], {
      name: "xterm-256color",
      cols: 80,
      rows: 24,
      cwd: process.env.HOME,
      env: process.env,
    });
  } catch (err) {
    app.log.error({ err, id }, "pty spawn failed");
    ws.close(1011, "pty spawn failed");
    return;
  }
  bridges.set(ws, term);
  app.log.info({ id, pid: term.pid }, "term attached");

  term.onData((data) => {
    if (ws.readyState === ws.OPEN) ws.send(Buffer.from(data));
  });
  term.onExit(({ exitCode }) => {
    bridges.delete(ws);
    app.log.info({ id, exitCode }, "term detached");
    if (ws.readyState === ws.OPEN) ws.close(1000, "detached");
  });

  ws.on("message", (data, isBinary) => {
    if (isBinary) {
      term.write(data.toString("utf8"));
      return;
    }
    try {
      const msg = JSON.parse(data.toString("utf8"));
      if (msg.type === "resize" && msg.cols > 0 && msg.rows > 0) {
        term.resize(msg.cols, msg.rows);
      }
    } catch {
      app.log.warn({ id }, "ignoring malformed control frame");
    }
  });
  ws.on("close", () => {
    bridges.delete(ws);
    term.kill();
  });
}
