// p8.2 — /api/health carries `version` (the package.json version, read
// once at daemon start) and `pid` (process.pid) so the launcher
// (bin/garage.js) can detect a stale pre-upgrade daemon and stop it by
// pid — never by process-name matching. A pre-upgrade daemon reports
// neither field, which the launcher also treats as stale.
import { test, before, after } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const ROOT = path.resolve(__dirname, "..", "..");

let app;

before(async () => {
  const { default: Fastify } = await import("fastify");
  const { healthPayload } = await import("../src/health.js");
  app = Fastify();
  // Same registration shape as daemon/src/index.js.
  app.get("/api/health", async () => healthPayload());
});

after(async () => {
  await app?.close();
});

test("health reports ok + the package version + the daemon pid", async () => {
  const res = await app.inject({ method: "GET", url: "/api/health" });
  assert.equal(res.statusCode, 200);
  const body = res.json();
  assert.equal(body.status, "ok");

  const pkgVersion = JSON.parse(
    readFileSync(path.join(ROOT, "package.json"), "utf8")
  ).version;
  assert.equal(body.version, pkgVersion, "version must match package.json");
  assert.equal(body.pid, process.pid, "pid must be the serving process");
});

test("version and pid are stable across calls (read once, no drift)", async () => {
  const a = (await app.inject({ method: "GET", url: "/api/health" })).json();
  const b = (await app.inject({ method: "GET", url: "/api/health" })).json();
  assert.deepEqual(a, b);
});
