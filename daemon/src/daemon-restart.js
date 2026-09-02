// p16-restart D3: daemon self-restart. POST /api/daemon/restart spawns a
// detached successor running this same daemon/src/index.js, replies 202
// with the successor's pid, then closes this daemon and exits — the
// successor takes the port once this process is gone (see index.js's
// boot-time waitForPredecessor call and D6's port-handoff-without-a-race
// requirement).
import { spawn } from "node:child_process";
import { mkdirSync, openSync } from "node:fs";
import os from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";

// Same daemon.log the launcher's startDetachedDaemon uses (bin/garage.js) —
// honors GARAGE_DIR (the p8.1 scratch-dir override) so a scratch-port test
// run never writes into the real ~/.garage.
function daemonLogPath() {
  const logDir = process.env.GARAGE_DIR ?? path.join(os.homedir(), ".garage");
  mkdirSync(logDir, { recursive: true });
  return path.join(logDir, "daemon.log");
}

// probe() answers true while the predecessor is still reachable on `port`,
// false once its /api/health stops answering (port free). A short per-probe
// timeout keeps a single failed connect from stretching the 200ms poll
// interval a caller passes below.
async function defaultProbe(port) {
  try {
    const res = await fetch(`http://127.0.0.1:${port}/api/health`, {
      signal: AbortSignal.timeout(500),
    });
    return res.ok;
  } catch {
    return false;
  }
}

// D3/D6: called at daemon boot when GARAGE_PREDECESSOR_PID is set (see
// index.js) — polls until the predecessor's health stops answering (the
// port is free, safe to bind) or `deadlineMs` passes, in which case it
// throws so the successor fails loud and exits non-zero rather than racing
// the predecessor for the port. `probe`/`now`/`sleep` are injectable so this
// is unit-testable without a real daemon or real time (daemon/test/).
export async function waitForPredecessor({
  port,
  pid,
  probe = () => defaultProbe(port),
  now = Date.now,
  sleep = (ms) => new Promise((r) => setTimeout(r, ms)),
  deadlineMs = 10000,
} = {}) {
  const start = now();
  while (now() - start < deadlineMs) {
    if (!(await probe())) return;
    await sleep(200);
  }
  throw new Error(
    `predecessor daemon (pid ${pid}) did not release port ${port} within ${deadlineMs}ms`
  );
}

export default async function daemonRestartRoutes(app) {
  app.post("/api/daemon/restart", async (req, reply) => {
    // daemon/src/daemon-restart.js -> daemon/src/index.js, same directory.
    const daemonEntry = path.resolve(
      path.dirname(fileURLToPath(import.meta.url)),
      "index.js"
    );
    const out = openSync(daemonLogPath(), "a");
    const child = spawn(process.execPath, [daemonEntry], {
      detached: true,
      stdio: ["ignore", out, out],
      // GARAGE_PREDECESSOR_PID tells the successor which pid to wait out
      // before it binds the port (see waitForPredecessor / index.js).
      env: { ...process.env, GARAGE_PREDECESSOR_PID: String(process.pid) },
    });
    child.unref();

    await reply.code(202).send({ pid: child.pid });
    // The reply must flush to the socket before this process tears itself
    // down — deferred to the next tick so Fastify's write actually reaches
    // the client first (D3).
    setImmediate(async () => {
      await app.close();
      process.exit(0);
    });
  });
}
