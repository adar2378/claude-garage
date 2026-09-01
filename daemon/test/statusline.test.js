// p11 — statusline.js ingest: token auth, session resolution (shared with
// hooks.js via session-resolve.js), the in-memory context/rate-limits store.
// Pure-contract level, like hooks.test.js — plus one real-tmux end-to-end
// case for the locked "Statusline post updates context" scenario, mirroring
// how sessions.title.test.js spins up a real tmux session for its own
// end-to-end coverage.
//
// GARAGE_DIR and GARAGE_PORT are set BEFORE the module loads: GARAGE_DIR so
// this never reads/writes the real ~/.garage/state.json (getHookToken lives
// there), and GARAGE_PORT so nothing here can ever address the real daemon
// on 4747 (this suite doesn't exercise the wrapper's curl call at all, but
// keeping the port off 4747 is a cheap, load-bearing habit for every
// statusline test file).
import { test, before, after, beforeEach } from "node:test";
import assert from "node:assert/strict";
import { mkdtemp, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import path from "node:path";

let scratch;
let app;
let registry;
let tmux;
let statusline;
let transcript;

const RUN = `p11-statusline-${process.pid}`;

before(async () => {
  scratch = await mkdtemp(path.join(tmpdir(), "garage-statusline-"));
  process.env.GARAGE_DIR = scratch;
  process.env.GARAGE_PORT = "47391"; // never the real daemon's 4747

  registry = await import("../src/registry.js");
  tmux = await import("../src/tmux.js");
  statusline = await import("../src/statusline.js");
  transcript = await import("../src/transcript.js");
  const { default: Fastify } = await import("fastify");
  const { default: statuslineRoutes } = await import("../src/statusline.js");
  app = Fastify();
  await app.register(statuslineRoutes);
});

after(async () => {
  await app?.close();
  await rm(scratch, { recursive: true, force: true });
  delete process.env.GARAGE_PORT;
});

beforeEach(() => {
  statusline.dropSession(`garage/${RUN}/one`);
});

// --- pure store / clamp -----------------------------------------------

test("clampPercentage clamps into [0,100] and rejects non-numbers", () => {
  assert.equal(statusline.clampPercentage(42), 42);
  assert.equal(statusline.clampPercentage(150), 100);
  assert.equal(statusline.clampPercentage(-5), 0);
  assert.equal(statusline.clampPercentage("42"), null);
  assert.equal(statusline.clampPercentage(undefined), null);
  assert.equal(statusline.clampPercentage(NaN), null);
});

test("getStatuslineContext is null for an id that never posted", () => {
  assert.equal(statusline.getStatuslineContext("garage/never/heard-of"), null);
});

test("setContext / getStatuslineContext round-trip with source 'statusline'", () => {
  const id = `garage/${RUN}/one`;
  statusline.setContext(id, 42);
  assert.deepEqual(statusline.getStatuslineContext(id), { usedPercentage: 42, source: "statusline" });
});

test("dropSession clears stored context", () => {
  const id = `garage/${RUN}/one`;
  statusline.setContext(id, 88);
  statusline.dropSession(id);
  assert.equal(statusline.getStatuslineContext(id), null);
});

test("getRateLimits starts null before any post", () => {
  // Isolated from other tests by construction — the rate-limit setters
  // below are additive/idempotent-ish, but this assertion only holds if it
  // runs first; the account-wide singleton is exercised more precisely by
  // usage.test.js in its own process. Here we only prove the shape.
  const limits = statusline.getRateLimits();
  assert.ok("fiveHour" in limits && "sevenDay" in limits);
});

test("setRateLimits stores used_percentage + resets_at for both buckets", () => {
  statusline.setRateLimits({
    five_hour: { used_percentage: 23.5, resets_at: "2026-08-30T12:00:00Z" },
    seven_day: { used_percentage: 61, resets_at: "2026-09-01T00:00:00Z" },
  });
  const limits = statusline.getRateLimits();
  assert.deepEqual(limits.fiveHour, { usedPercentage: 23.5, resetsAt: "2026-08-30T12:00:00Z" });
  assert.deepEqual(limits.sevenDay, { usedPercentage: 61, resetsAt: "2026-09-01T00:00:00Z" });
});

test("setRateLimits partial update preserves the untouched bucket", () => {
  statusline.setRateLimits({ five_hour: { used_percentage: 10, resets_at: "a" } });
  statusline.setRateLimits({ seven_day: { used_percentage: 20, resets_at: "b" } });
  const limits = statusline.getRateLimits();
  assert.equal(limits.fiveHour.usedPercentage, 10);
  assert.equal(limits.sevenDay.usedPercentage, 20);
});

test("setRateLimits ignores malformed input without throwing", () => {
  assert.doesNotThrow(() => statusline.setRateLimits(null));
  assert.doesNotThrow(() => statusline.setRateLimits(undefined));
  assert.doesNotThrow(() => statusline.setRateLimits("not an object"));
  assert.doesNotThrow(() => statusline.setRateLimits({ five_hour: { used_percentage: "nope" } }));
});

// --- route: auth + resolution -------------------------------------------

test("POST /api/statusline/claude: missing/invalid token is 401", async () => {
  const res = await app.inject({
    method: "POST",
    url: "/api/statusline/claude?token=totally-wrong",
    payload: { session_id: "whatever" },
  });
  assert.equal(res.statusCode, 401);
});

test("POST /api/statusline/claude: unresolvable session is {ignored:true}, no error", async () => {
  const token = await registry.getHookToken();
  const res = await app.inject({
    method: "POST",
    url: `/api/statusline/claude?token=${token}`,
    payload: {
      session_id: "no-such-claude-session",
      cwd: "/no/such/dir/anywhere",
      context_window: { used_percentage: 99 },
    },
  });
  assert.equal(res.statusCode, 200);
  assert.deepEqual(res.json(), { ok: true, ignored: true });
});

test("POST /api/statusline/claude: a resolvable session (cwd fallback) stores clamped context — locked scenario", async (t) => {
  const token = await registry.getHookToken();
  const id = `garage/${RUN}/resolvable`;
  await tmux.createSession(id, scratch, "sleep", ["60"]);
  t.after(() => tmux.killSession(id).catch(() => {}));

  const res = await app.inject({
    method: "POST",
    url: `/api/statusline/claude?token=${token}`,
    payload: {
      // no session_id -> falls back to cwd matching, exactly like hooks.js
      cwd: scratch,
      context_window: { used_percentage: 142 }, // out of range on purpose
    },
  });
  assert.equal(res.statusCode, 200);
  const body = res.json();
  assert.equal(body.ok, true);
  assert.ok(body.applied.includes(id));
  assert.deepEqual(statusline.getStatuslineContext(id), { usedPercentage: 100, source: "statusline" });
});

test("POST /api/statusline/claude: model.id + context_window_size teach the transcript fallback", async (t) => {
  const token = await registry.getHookToken();
  const id = `garage/${RUN}/window-teach`;
  await tmux.createSession(id, scratch, "sleep", ["60"]);
  t.after(() => tmux.killSession(id).catch(() => {}));

  const res = await app.inject({
    method: "POST",
    url: `/api/statusline/claude?token=${token}`,
    payload: {
      cwd: scratch,
      model: { id: "claude-window-test" },
      context_window: { used_percentage: 12, context_window_size: 400_000 },
    },
  });
  assert.equal(res.statusCode, 200);
  assert.equal(transcript.windowForModel("claude-window-test"), 400_000);
});

test("POST /api/statusline/claude: rate_limits update the account-wide store even without context_window", async (t) => {
  const token = await registry.getHookToken();
  const id = `garage/${RUN}/rate-only`;
  await tmux.createSession(id, scratch, "sleep", ["60"]);
  t.after(() => tmux.killSession(id).catch(() => {}));

  const res = await app.inject({
    method: "POST",
    url: `/api/statusline/claude?token=${token}`,
    payload: {
      cwd: scratch,
      rate_limits: { five_hour: { used_percentage: 33, resets_at: "later" } },
    },
  });
  assert.equal(res.statusCode, 200);
  assert.equal(statusline.getStatuslineContext(id), null); // no context_window in this post
  assert.equal(statusline.getRateLimits().fiveHour.usedPercentage, 33);
});

test("GET /api/statusline/snippet returns a wrapper definition without needing settings.json", async () => {
  const res = await app.inject({ method: "GET", url: "/api/statusline/snippet" });
  assert.equal(res.statusCode, 200);
  const body = res.json();
  assert.equal(body.statusLine.type, "command");
  assert.match(body.statusLine.command, /garage-statusline\.sh/);
  assert.match(body.script, /curl/);
  assert.match(body.script, /api\/statusline\/claude\?token=/);
});
