// p11 — `context` on GET /api/sessions entries. Shape mirrors how `title`
// (p10) and `message` (p8) ride the same response: a plain field, present
// on every entry, null when there's nothing to show. Precedence:
// statusline > fresh transcript > null; restorable entries are always null.
//
// GARAGE_DIR points the registry at a scratch dir, GARAGE_CLAUDE_HOME points
// the transcript reader's "~/.claude" root at a scratch dir — both set
// BEFORE the modules load — so this suite never touches the real
// ~/.garage/state.json or ~/.claude/projects.
import { test, before, after } from "node:test";
import assert from "node:assert/strict";
import { mkdtemp, rm, mkdir, writeFile, realpath } from "node:fs/promises";
import { tmpdir } from "node:os";
import path from "node:path";

let scratchGarage;
let scratchClaude;
let app;
let registry;
let tmux;
let statusline;
let transcript;

const RUN = `p11-context-${process.pid}`;
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

before(async () => {
  scratchGarage = await mkdtemp(path.join(tmpdir(), "garage-context-state-"));
  scratchClaude = await mkdtemp(path.join(tmpdir(), "garage-context-claude-"));
  process.env.GARAGE_DIR = scratchGarage;
  process.env.GARAGE_CLAUDE_HOME = scratchClaude;

  registry = await import("../src/registry.js");
  tmux = await import("../src/tmux.js");
  statusline = await import("../src/statusline.js");
  transcript = await import("../src/transcript.js");
  const { default: Fastify } = await import("fastify");
  const { default: sessionRoutes } = await import("../src/sessions.js");
  app = Fastify();
  await app.register(sessionRoutes);
});

after(async () => {
  await app?.close();
  await rm(scratchGarage, { recursive: true, force: true });
  await rm(scratchClaude, { recursive: true, force: true });
  delete process.env.GARAGE_DIR;
  delete process.env.GARAGE_CLAUDE_HOME;
});

function assistantLine({ input = 0, cacheRead = 0, cacheCreation = 0, model = "claude-x" } = {}) {
  return JSON.stringify({
    type: "assistant",
    message: {
      model,
      usage: {
        input_tokens: input,
        cache_read_input_tokens: cacheRead,
        cache_creation_input_tokens: cacheCreation,
      },
    },
  });
}

async function writeTranscript(dir, claudeSessionId, content) {
  const projectDir = path.join(scratchClaude, "projects", transcript.slugifyCwd(dir));
  await mkdir(projectDir, { recursive: true });
  await writeFile(path.join(projectDir, `${claudeSessionId}.jsonl`), content, "utf8");
}

test("a cold session (no statusline, no transcript meta) has context: null", async (t) => {
  const id = `garage/${RUN}/cold`;
  await tmux.createSession(id, scratchGarage, "sleep", ["60"]);
  t.after(() => tmux.killSession(id).catch(() => {}));

  const res = await app.inject({ method: "GET", url: "/api/sessions" });
  assert.equal(res.statusCode, 200);
  const entry = res.json().find((s) => s.id === id);
  assert.ok(entry, "live entry must be present");
  assert.equal(entry.context, null);
});

test("a live session fed by statusline shows source 'statusline' — locked scenario", async (t) => {
  const id = `garage/${RUN}/statusline-fed`;
  await tmux.createSession(id, scratchGarage, "sleep", ["60"]);
  t.after(() => tmux.killSession(id).catch(() => {}));

  statusline.setContext(id, 42);
  t.after(() => statusline.dropSession(id));

  const res = await app.inject({ method: "GET", url: "/api/sessions" });
  const entry = res.json().find((s) => s.id === id);
  assert.deepEqual(entry.context, { usedPercentage: 42, source: "statusline" });
});

test("a live session with only transcript data shows source 'transcript' after a refresh interval", async (t) => {
  const id = `garage/${RUN}/transcript-fed`;
  // tmux reports pane_current_path fully resolved (e.g. macOS's
  // /var/folders -> /private/var/folders symlink) — resolve scratchGarage
  // the same way so the dir this test writes a transcript under matches
  // what the route actually reads back from the live pane.
  const dir = await realpath(scratchGarage);
  const claudeSessionId = "transcript-fed-cs-id";

  await tmux.createSession(id, dir, "sleep", ["60"]);
  t.after(() => tmux.killSession(id).catch(() => {}));

  await registry.upsertSessionMeta(id, { claudeSessionId, workspace: RUN, label: "transcript-fed" });
  await writeTranscript(dir, claudeSessionId, assistantLine({ input: 150_000 })); // 75% of 200k

  // First fetch: the route must never block on the transcript read — it
  // serves the (still-empty) cache and only kicks a background refresh.
  const first = await app.inject({ method: "GET", url: "/api/sessions" });
  const firstEntry = first.json().find((s) => s.id === id);
  assert.equal(firstEntry.context, null);

  await sleep(80); // let the background refresh complete

  const second = await app.inject({ method: "GET", url: "/api/sessions" });
  const secondEntry = second.json().find((s) => s.id === id);
  assert.deepEqual(secondEntry.context, { usedPercentage: 75, source: "transcript" });
});

test("statusline takes precedence over transcript when both are present", async (t) => {
  const id = `garage/${RUN}/both`;
  const dir = await realpath(scratchGarage);
  const claudeSessionId = "both-cs-id";

  await tmux.createSession(id, dir, "sleep", ["60"]);
  t.after(() => tmux.killSession(id).catch(() => {}));

  await registry.upsertSessionMeta(id, { claudeSessionId, workspace: RUN, label: "both" });
  await writeTranscript(dir, claudeSessionId, assistantLine({ input: 180_000 })); // 90% — must NOT win
  statusline.setContext(id, 11);
  t.after(() => statusline.dropSession(id));

  // Give the transcript cache a moment to populate too, so precedence is
  // proven against a populated (not merely absent) alternative.
  await app.inject({ method: "GET", url: "/api/sessions" });
  await sleep(80);

  const res = await app.inject({ method: "GET", url: "/api/sessions" });
  const entry = res.json().find((s) => s.id === id);
  assert.deepEqual(entry.context, { usedPercentage: 11, source: "statusline" });
});

test("restorable entries always have context: null, even if a statusline post exists for that id", async () => {
  const id = `garage/${RUN}/restorable`;
  await registry.upsertSessionMeta(id, {
    claudeSessionId: "restorable-cs-id",
    workspace: RUN,
    label: "restorable",
  });
  statusline.setContext(id, 99); // adversarial: must be ignored for a non-live entry

  const res = await app.inject({ method: "GET", url: "/api/sessions" });
  const entry = res.json().find((s) => s.id === id);
  assert.ok(entry, "restorable entry must be present");
  assert.equal(entry.restorable, true);
  assert.equal(entry.context, null);
});
