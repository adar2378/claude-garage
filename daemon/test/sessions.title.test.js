// p10 — `title` on GET /api/sessions entries. Shape mirrors how `message`
// (p8) rides the same response: a plain field, present on every entry,
// null when there's nothing to show.
//
// GARAGE_DIR points the registry at a scratch dir BEFORE the module loads,
// so this test never reads or writes the real ~/.garage/state.json.
import { test, before, after } from "node:test";
import assert from "node:assert/strict";
import { execFile } from "node:child_process";
import { promisify } from "node:util";
import { mkdtemp, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import path from "node:path";

const run = promisify(execFile);

let scratch;
let app;
let registry;
let tmux;

// Unique per-run id family so this never collides with a real garage
// session, live or restorable.
const RUN = `p10-title-${process.pid}`;

before(async () => {
  scratch = await mkdtemp(path.join(tmpdir(), "garage-title-"));
  process.env.GARAGE_DIR = scratch;
  // Dynamic imports AFTER the env override — registry.js resolves
  // GARAGE_DIR at module load.
  registry = await import("../src/registry.js");
  tmux = await import("../src/tmux.js");
  const { default: Fastify } = await import("fastify");
  const { default: sessionRoutes } = await import("../src/sessions.js");
  app = Fastify();
  await app.register(sessionRoutes);
});

after(async () => {
  await app?.close();
  await rm(scratch, { recursive: true, force: true });
});

test("restorable entries always have title: null", async () => {
  const id = `garage/${RUN}/restorable`;
  await registry.upsertSessionMeta(id, {
    claudeSessionId: "abc-123",
    workspace: RUN,
    label: "restorable",
  });

  const res = await app.inject({ method: "GET", url: "/api/sessions" });
  assert.equal(res.statusCode, 200);
  const entry = res.json().find((s) => s.id === id);
  assert.ok(entry, "restorable entry must be present");
  assert.equal(entry.restorable, true);
  assert.equal(entry.title, null);
});

test("a live session's set pane title surfaces normalized on its entry", async (t) => {
  const id = `garage/${RUN}/live`;
  await tmux.createSession(id, scratch, "sleep", ["60"]);
  t.after(() => tmux.killSession(id).catch(() => {}));

  await run("tmux", ["select-pane", "-t", id, "-T", "  ✳ Refactoring the poller  "]);

  const res = await app.inject({ method: "GET", url: "/api/sessions" });
  assert.equal(res.statusCode, 200);
  const entry = res.json().find((s) => s.id === id);
  assert.ok(entry, "live entry must be present");
  assert.equal(entry.restorable, undefined);
  assert.equal(entry.title, "✳ Refactoring the poller"); // trimmed
});

test("a live session with no set title (tmux's own default) has title: null", async (t) => {
  const id = `garage/${RUN}/default-title`;
  await tmux.createSession(id, scratch, "sleep", ["60"]);
  t.after(() => tmux.killSession(id).catch(() => {}));
  // No select-pane -T call — the pane keeps whatever tmux's own default is
  // (hostname or the running command name), both of which normalizeTitle
  // filters to null.

  const res = await app.inject({ method: "GET", url: "/api/sessions" });
  const entry = res.json().find((s) => s.id === id);
  assert.ok(entry, "live entry must be present");
  assert.equal(entry.title, null);
});
