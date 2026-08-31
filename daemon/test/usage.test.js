// p11 — GET /api/usage: account-wide rate limits from the most recent
// statusline post, null until one arrives. Lives in its own file (its own
// `node --test` process) so statusline.js's module-level rate-limits
// singleton starts clean — sessions.context.test.js and statusline.test.js
// each get their own equally-clean copy the same way.
import { test, before, after } from "node:test";
import assert from "node:assert/strict";
import { mkdtemp, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import path from "node:path";

let scratch;
let app;
let statusline;

before(async () => {
  scratch = await mkdtemp(path.join(tmpdir(), "garage-usage-"));
  process.env.GARAGE_DIR = scratch;

  statusline = await import("../src/statusline.js");
  const { default: Fastify } = await import("fastify");
  const { default: sessionRoutes } = await import("../src/sessions.js");
  app = Fastify();
  await app.register(sessionRoutes);
});

after(async () => {
  await app?.close();
  await rm(scratch, { recursive: true, force: true });
  delete process.env.GARAGE_DIR;
});

test("GET /api/usage returns nulls before any statusline post", async () => {
  const res = await app.inject({ method: "GET", url: "/api/usage" });
  assert.equal(res.statusCode, 200);
  assert.deepEqual(res.json(), { fiveHour: null, sevenDay: null });
});

test("GET /api/usage reflects a five_hour-only post, sevenDay stays null", async () => {
  statusline.setRateLimits({ five_hour: { used_percentage: 23.5, resets_at: "2026-08-30T18:00:00Z" } });
  const res = await app.inject({ method: "GET", url: "/api/usage" });
  const body = res.json();
  assert.deepEqual(body.fiveHour, { usedPercentage: 23.5, resetsAt: "2026-08-30T18:00:00Z" });
  assert.equal(body.sevenDay, null);
});

test("GET /api/usage: usage after a post — locked scenario", async () => {
  statusline.setRateLimits({ five_hour: { used_percentage: 23.5, resets_at: "later" } });
  const res = await app.inject({ method: "GET", url: "/api/usage" });
  assert.equal(res.json().fiveHour.usedPercentage, 23.5);
});

test("GET /api/usage: a later seven_day post merges in without dropping the retained five_hour", async () => {
  statusline.setRateLimits({ seven_day: { used_percentage: 61, resets_at: "next-week" } });
  const res = await app.inject({ method: "GET", url: "/api/usage" });
  const body = res.json();
  assert.equal(body.fiveHour.usedPercentage, 23.5); // retained from the earlier post
  assert.deepEqual(body.sevenDay, { usedPercentage: 61, resetsAt: "next-week" });
});
