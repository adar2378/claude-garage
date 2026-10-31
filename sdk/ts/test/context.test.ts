import assert from "node:assert/strict";
import { mkdtemp, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { describe, it } from "node:test";

import { Context, readContext } from "../src/index.js";
import type { RunContext } from "../src/index.js";

const sample: RunContext = {
  api_version: 1,
  plugin: { id: "demo", version: "1.0.0", state_dir: "/data/demo" },
  trigger: { kind: "event", key: null, slot: null, action: "event:session.turn", args: {} },
  focus: {
    workspace: "garage",
    session: { id: "garage/main", dir: "/dev/garage", status: "idle", branch: "main", worktree: null },
    dir: null,
  },
  selection: { session: "garage/main", index: 0, view: null },
  worktree: null,
  event: { name: "session.turn", at: 1700000000000, data: { id: "garage/main", workspace: "main", at: 1700000000000, anchor: { transcript_offset: null, row: 12, history_limit: 2000 } } },
  ui: { slots: {} },
};

describe("readContext", () => {
  it("prefers the inline variable", async () => {
    const ctx = await readContext({
      env: { GARAGE_CONTEXT: JSON.stringify(sample), GARAGE_CONTEXT_FILE: "/nope" },
    });
    assert.equal(ctx.workspace, "garage");
    assert.equal(ctx.session, "garage/main");
    assert.equal(ctx.eventName, "session.turn");
    assert.deepEqual(ctx.eventData, {
      id: "garage/main",
      workspace: "main",
      at: 1700000000000,
      anchor: { transcript_offset: null, row: 12, history_limit: 2000 },
    });
    assert.equal(ctx.stateDir, "/data/demo");
    assert.equal(ctx.cwd, "/dev/garage");
    assert.equal(ctx.triggerKind, "event");
  });

  it("reads the file when the context is large", async () => {
    const dir = await mkdtemp(join(tmpdir(), "garage-sdk-"));
    const file = join(dir, "ctx.json");
    await writeFile(file, JSON.stringify(sample));
    const ctx = await readContext({ env: { GARAGE_CONTEXT_FILE: file } });
    assert.equal(ctx.session, "garage/main");
  });

  it("falls back to standard input", async () => {
    const ctx = await readContext({ env: {}, stdin: async () => JSON.stringify(sample) });
    assert.equal(ctx.workspace, "garage");
  });

  it("says what went wrong with a bad context", async () => {
    await assert.rejects(
      () => readContext({ env: { GARAGE_CONTEXT: "{nope" } }),
      /cannot read the run context/,
    );
  });
});

describe("Context accessors", () => {
  it("treat null and empty as undefined", () => {
    const ctx = new Context({
      ...sample,
      focus: { workspace: null, session: null, dir: null },
      event: null,
      trigger: { ...sample.trigger, kind: "palette" },
    });
    assert.equal(ctx.workspace, undefined);
    assert.equal(ctx.session, undefined);
    assert.equal(ctx.event, undefined);
    assert.equal(ctx.eventName, undefined);
    assert.equal(ctx.cwd, undefined);
  });

  it("reads argv of a CLI run", () => {
    const ctx = new Context({
      ...sample,
      trigger: { ...sample.trigger, kind: "cli", args: { argv: ["open", "~/dev/app", 3] } },
    });
    assert.deepEqual(ctx.argv, ["open", "~/dev/app"]);
    assert.deepEqual(new Context(sample).argv, []);
    const withDir = new Context({
      ...sample,
      focus: { workspace: null, session: null, dir: "/home/me/app" },
    });
    assert.equal(withDir.cwd, "/home/me/app");
  });
});
