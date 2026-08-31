// p11 — transcript.js: cwd->slug, tail-bounded read, backward-scan for the
// last assistant usage, model->window mapping, and the per-session
// {value,at} cache with its 15s TTL + background refresh.
//
// GARAGE_CLAUDE_HOME points the reader's "~/.claude" root at a scratch dir
// BEFORE the module loads, so this suite never reads the user's real
// ~/.claude/projects transcripts.
import { test, before, after } from "node:test";
import assert from "node:assert/strict";
import { mkdtemp, rm, mkdir, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import path from "node:path";

let scratch;
let transcript;

const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

before(async () => {
  scratch = await mkdtemp(path.join(tmpdir(), "garage-claude-home-"));
  process.env.GARAGE_CLAUDE_HOME = scratch;
  transcript = await import("../src/transcript.js");
});

after(async () => {
  await rm(scratch, { recursive: true, force: true });
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
  const projectDir = path.join(scratch, "projects", transcript.slugifyCwd(dir));
  await mkdir(projectDir, { recursive: true });
  await writeFile(path.join(projectDir, `${claudeSessionId}.jsonl`), content, "utf8");
}

// --- pure functions ------------------------------------------------------

test("slugifyCwd replaces every / with -", () => {
  assert.equal(transcript.slugifyCwd("/Users/me/repo"), "-Users-me-repo");
  assert.equal(transcript.slugifyCwd("/a/b/c"), "-a-b-c");
});

test("windowForModel: unknown/missing model defaults to 200000", () => {
  assert.equal(transcript.windowForModel("claude-sonnet-4-5"), 200_000);
  assert.equal(transcript.windowForModel(undefined), 200_000);
  assert.equal(transcript.windowForModel(null), 200_000);
});

test("windowForModel: a [1m] model id gets the 1M window", () => {
  assert.equal(transcript.windowForModel("claude-sonnet-4-5-20250929[1m]"), 1_000_000);
});

test("parseLastAssistantUsage finds the LAST assistant usage, skipping malformed and non-assistant lines", () => {
  const text = [
    assistantLine({ input: 10 }), // superseded by the one below
    "{ this is not valid json",
    JSON.stringify({ type: "system", subtype: "noise" }),
    assistantLine({ input: 5, cacheRead: 100, cacheCreation: 20, model: "claude-y" }),
  ].join("\n");
  const usage = transcript.parseLastAssistantUsage(text);
  assert.deepEqual(usage, { tokens: 125, model: "claude-y" });
});

test("parseLastAssistantUsage returns null when nothing usable is found", () => {
  const text = ["not json at all", JSON.stringify({ type: "system" })].join("\n");
  assert.equal(transcript.parseLastAssistantUsage(text), null);
});

// --- computeContext (real file IO, tail-bound) ----------------------------

test("computeContext: percentage = tokens / window, default 200000 window", async () => {
  const dir = "/scratch/proj-a";
  const csid = "session-a";
  await writeTranscript(
    dir,
    csid,
    [
      JSON.stringify({ type: "system" }),
      assistantLine({ input: 50_000, cacheRead: 50_000, cacheCreation: 0 }), // 100k / 200k = 50%
    ].join("\n")
  );

  const result = await transcript.computeContext({ dir, claudeSessionId: csid });
  assert.deepEqual(result, { usedPercentage: 50, source: "transcript" });
});

test("computeContext: missing transcript file resolves to null, never throws", async () => {
  const result = await transcript.computeContext({ dir: "/nope", claudeSessionId: "no-such-id" });
  assert.equal(result, null);
});

test("computeContext: missing dir/claudeSessionId resolves to null", async () => {
  assert.equal(await transcript.computeContext({ dir: null, claudeSessionId: "x" }), null);
  assert.equal(await transcript.computeContext({ dir: "/x", claudeSessionId: undefined }), null);
});

test("computeContext: malformed trailing lines are skipped, last valid assistant usage still found", async () => {
  const dir = "/scratch/proj-malformed";
  const csid = "session-malformed";
  await writeTranscript(
    dir,
    csid,
    [
      assistantLine({ input: 20_000, cacheRead: 0, cacheCreation: 0 }), // 10%
      "{{{ garbage, not json",
      "", // blank line
    ].join("\n")
  );
  const result = await transcript.computeContext({ dir, claudeSessionId: csid });
  assert.deepEqual(result, { usedPercentage: 10, source: "transcript" });
});

test("computeContext: tail-bounded read finds usage within the last 256KB of a multi-MB file", async () => {
  const dir = "/scratch/proj-big";
  const csid = "session-big";
  const filler = "x".repeat(1024); // 1KB junk lines, well-formed-but-irrelevant JSON
  const padLine = JSON.stringify({ type: "user", junk: filler });
  const padding = Array.from({ length: 4000 }, () => padLine).join("\n"); // ~4MB
  const content = `${padding}\n${assistantLine({ input: 30_000, cacheRead: 0, cacheCreation: 0 })}\n`; // 15%
  await writeTranscript(dir, csid, content);

  const result = await transcript.computeContext({ dir, claudeSessionId: csid });
  assert.deepEqual(result, { usedPercentage: 15, source: "transcript" });
});

test("computeContext: a usage line entirely outside the tail-bounded window is not found (documented trade-off)", async () => {
  const dir = "/scratch/proj-outside-tail";
  const csid = "session-outside-tail";
  const targetUsage = assistantLine({ input: 199_999, cacheRead: 0, cacheCreation: 0 }); // would be ~100%
  const filler = "x".repeat(1024);
  const padLine = JSON.stringify({ type: "user", junk: filler });
  // >256KB of filler AFTER the target line pushes it outside the tail read.
  const trailingPadding = Array.from({ length: 400 }, () => padLine).join("\n"); // ~400KB
  const content = `${targetUsage}\n${trailingPadding}\n`;
  await writeTranscript(dir, csid, content);

  const result = await transcript.computeContext({ dir, claudeSessionId: csid });
  assert.equal(result, null);
});

// --- cache: getCachedContext / refreshContext (15s TTL, background) ------

test("getCachedContext is null before any refresh has run", () => {
  assert.equal(transcript.getCachedContext("cache-never-refreshed"), null);
});

test("refreshContext populates the cache asynchronously without blocking the caller", async () => {
  const id = "cache-async";
  const dir = "/scratch/proj-cache";
  const csid = "session-cache";
  await writeTranscript(dir, csid, assistantLine({ input: 100_000 })); // 50%

  transcript.refreshContext(id, { dir, claudeSessionId: csid });
  // Synchronous call returns immediately — the read hasn't had a chance to
  // complete yet on a fresh event-loop turn.
  assert.equal(transcript.getCachedContext(id), null);

  await sleep(50);
  assert.deepEqual(transcript.getCachedContext(id), { usedPercentage: 50, source: "transcript" });
});

test("refreshContext respects the 15s TTL and refreshes again once stale", async (t) => {
  t.mock.timers.enable({ apis: ["Date"], now: Date.now() });
  const id = "cache-ttl";
  const dir = "/scratch/proj-ttl";
  const csid = "session-ttl";
  await writeTranscript(dir, csid, assistantLine({ input: 100_000 })); // 50%

  transcript.refreshContext(id, { dir, claudeSessionId: csid });
  await sleep(50);
  const firstAt = transcript.getCacheEntry(id).at;
  assert.ok(firstAt);

  t.mock.timers.tick(5_000); // 5s later — still within the 15s TTL
  transcript.refreshContext(id, { dir, claudeSessionId: csid });
  await sleep(50);
  assert.equal(transcript.getCacheEntry(id).at, firstAt, "no refresh should have happened yet");

  t.mock.timers.tick(11_000); // total 16s — past the TTL
  transcript.refreshContext(id, { dir, claudeSessionId: csid });
  await sleep(50);
  assert.notEqual(transcript.getCacheEntry(id).at, firstAt, "a stale entry must refresh");
});

test("clearCache drops a cached entry", async () => {
  const id = "cache-clear";
  const dir = "/scratch/proj-clear";
  const csid = "session-clear";
  await writeTranscript(dir, csid, assistantLine({ input: 100_000 }));
  transcript.refreshContext(id, { dir, claudeSessionId: csid });
  await sleep(50);
  assert.ok(transcript.getCachedContext(id));
  transcript.clearCache(id);
  assert.equal(transcript.getCachedContext(id), null);
});
