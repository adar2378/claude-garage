// p8.1 — DELETE /api/sessions/<id>?meta=1: drop ONLY the stored resume
// metadata of a non-live (restorable) session. The plain DELETE (no param)
// must keep its pre-p8.1 behavior exactly (404 for a session with no live
// tmux match, metadata retained) — the web UI depends on that.
//
// GARAGE_DIR points the registry at a scratch dir BEFORE the module loads,
// so this test never reads or writes the real ~/.garage/state.json.
import { test, before, after } from "node:test";
import assert from "node:assert/strict";
import { mkdtemp, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import path from "node:path";

let scratch;
let app;
let registry;

// Unique per-run id family; no tmux session will ever match it, so
// hasSession() is naturally false without mocking tmux.
const ID = `garage/p81-test-${process.pid}/one`;

before(async () => {
  scratch = await mkdtemp(path.join(tmpdir(), "garage-delete-meta-"));
  process.env.GARAGE_DIR = scratch;
  // Dynamic imports AFTER the env override — registry.js resolves
  // GARAGE_DIR at module load.
  registry = await import("../src/registry.js");
  const { default: Fastify } = await import("fastify");
  const { default: sessionRoutes } = await import("../src/sessions.js");
  app = Fastify();
  await app.register(sessionRoutes);
});

after(async () => {
  await app?.close();
  await rm(scratch, { recursive: true, force: true });
});

test("meta=1 with no stored metadata is a 404", async () => {
  const res = await app.inject({
    method: "DELETE",
    url: `/api/sessions/${encodeURIComponent(ID)}?meta=1`,
  });
  assert.equal(res.statusCode, 404);
});

test("plain DELETE of a non-live session stays 404 and keeps the meta", async () => {
  await registry.upsertSessionMeta(ID, {
    claudeSessionId: "abc-123",
    workspace: "p81-test",
    label: "one",
  });
  const res = await app.inject({
    method: "DELETE",
    url: `/api/sessions/${encodeURIComponent(ID)}`,
  });
  assert.equal(res.statusCode, 404); // web behavior unchanged
  assert.ok(await registry.getSessionMeta(ID), "meta must be retained");
});

test("meta=1 drops the metadata and echoes the worktree record", async () => {
  const worktree = {
    path: "/tmp/wt/p81-test/one",
    branch: "garage/one",
    repoDir: "/tmp/repo",
  };
  await registry.upsertSessionMeta(ID, { worktree });
  const res = await app.inject({
    method: "DELETE",
    url: `/api/sessions/${encodeURIComponent(ID)}?meta=1`,
  });
  assert.equal(res.statusCode, 200);
  const body = res.json();
  assert.equal(body.deleted, true);
  assert.equal(body.meta, true);
  // /tmp/repo is not a repo -> target is null (p17: informational field).
  assert.deepEqual(body.worktree, { ...worktree, target: null });
  assert.equal(await registry.getSessionMeta(ID), null, "meta must be gone");
});

test("meta=1 refuses non-garage ids like the plain DELETE (403)", async () => {
  const res = await app.inject({
    method: "DELETE",
    url: `/api/sessions/${encodeURIComponent("not-garage/x")}?meta=1`,
  });
  assert.equal(res.statusCode, 403);
});
