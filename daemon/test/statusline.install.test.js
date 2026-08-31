// p11 — POST /api/statusline/install + GET /api/statusline/snippet: the
// chaining wrapper, with the same install guarantees as hooks.js
// (parse-or-refuse/backup/atomic/idempotent — see settings-install.js,
// shared by both).
//
// GARAGE_CLAUDE_HOME points settings.json (and the generated wrapper
// script) at a scratch dir BEFORE the module loads, so this suite never
// touches the user's real ~/.claude/settings.json. GARAGE_DIR does the same
// for the hook-token registry (~/.garage/state.json). GARAGE_PORT is set to
// a throwaway, never-4747 port: the "chain preserves output" test actually
// EXECUTES the generated wrapper script, whose background curl call must
// never have a chance of reaching the real daemon.
import { test, before, after } from "node:test";
import assert from "node:assert/strict";
import { mkdtemp, rm, readFile, writeFile, chmod, stat } from "node:fs/promises";
import { tmpdir } from "node:os";
import path from "node:path";
import { execFileSync } from "node:child_process";

// execFile's callback/promise form has no `input` option (that only exists
// on the *Sync variants) — writing to a live child's stdin requires manual
// stream plumbing, so these tests just use the sync form: quick, local,
// no-network script runs, no benefit from async here.
function runWithStdin(cmd, args, input) {
  return execFileSync(cmd, args, { input, encoding: "utf8" });
}

let scratchClaude;
let scratchGarage;
let app;
let settingsPath;
let wrapperPath;

before(async () => {
  scratchClaude = await mkdtemp(path.join(tmpdir(), "garage-claude-home-"));
  scratchGarage = await mkdtemp(path.join(tmpdir(), "garage-state-"));
  process.env.GARAGE_CLAUDE_HOME = scratchClaude;
  process.env.GARAGE_DIR = scratchGarage;
  process.env.GARAGE_PORT = "47392"; // never the real daemon's 4747

  settingsPath = path.join(scratchClaude, "settings.json");
  wrapperPath = path.join(scratchClaude, "garage-statusline.sh");

  const { default: Fastify } = await import("fastify");
  const { default: statuslineRoutes } = await import("../src/statusline.js");
  app = Fastify();
  await app.register(statuslineRoutes);
});

after(async () => {
  await app?.close();
  await rm(scratchClaude, { recursive: true, force: true });
  await rm(scratchGarage, { recursive: true, force: true });
  delete process.env.GARAGE_CLAUDE_HOME;
  delete process.env.GARAGE_DIR;
  delete process.env.GARAGE_PORT;
});

test("install with no pre-existing settings.json creates one with the wrapper statusLine", async () => {
  const res = await app.inject({ method: "POST", url: "/api/statusline/install" });
  assert.equal(res.statusCode, 200);
  const body = res.json();
  assert.equal(body.installed, true);
  assert.equal(body.alreadyInstalled, false);
  assert.equal(body.chained, false);
  assert.equal(body.backup, null); // nothing pre-existing to back up

  const settings = JSON.parse(await readFile(settingsPath, "utf8"));
  assert.equal(settings.statusLine.type, "command");
  assert.match(settings.statusLine.command, /garage-statusline\.sh/);

  const script = await readFile(wrapperPath, "utf8");
  assert.match(script, /curl/);
  assert.match(script, /api\/statusline\/claude\?token=/);
  const st = await stat(wrapperPath);
  assert.ok(st.mode & 0o100, "wrapper script must be executable"); // owner-exec bit
});

test("second install is idempotent — no backup, nothing rewritten", async () => {
  const before1 = await readFile(settingsPath, "utf8");
  const res = await app.inject({ method: "POST", url: "/api/statusline/install" });
  assert.equal(res.statusCode, 200);
  const body = res.json();
  assert.equal(body.alreadyInstalled, true);
  assert.equal(body.backup, null);
  const after1 = await readFile(settingsPath, "utf8");
  assert.equal(after1, before1);
});

test("corrupt settings.json is refused with 422 and left byte-for-byte untouched", async () => {
  const corruptPath = path.join(scratchClaude, "settings.json");
  const corrupt = "{ not: valid json !!";
  await writeFile(corruptPath, corrupt, "utf8");

  const res = await app.inject({ method: "POST", url: "/api/statusline/install" });
  assert.equal(res.statusCode, 422);

  const stillThere = await readFile(corruptPath, "utf8");
  assert.equal(stillThere, corrupt);

  // Restore a valid, empty settings.json for the tests below.
  await writeFile(corruptPath, "{}\n", "utf8");
});

test("an existing non-garage statusLine is captured and chained — output is byte-identical", async (t) => {
  // A fake pre-existing statusline (e.g. ccstatusline): ignores stdin,
  // prints a fixed marker so we can assert byte parity after wrapping.
  const originalScriptPath = path.join(scratchClaude, "fake-original.sh");
  await writeFile(originalScriptPath, `#!/bin/sh\necho "ORIGINAL-OUTPUT-MARKER"\n`, "utf8");
  await chmod(originalScriptPath, 0o755);

  await writeFile(
    settingsPath,
    JSON.stringify({
      model: "opus", // unrelated setting — must survive the merge untouched
      statusLine: { type: "command", command: originalScriptPath },
    }),
    "utf8"
  );

  const res = await app.inject({ method: "POST", url: "/api/statusline/install" });
  assert.equal(res.statusCode, 200);
  const body = res.json();
  assert.equal(body.installed, true);
  assert.equal(body.alreadyInstalled, false);
  assert.equal(body.chained, true);
  assert.ok(body.backup, "a backup must be taken — a real file existed");
  const backupContent = await readFile(body.backup, "utf8");
  assert.match(backupContent, /fake-original\.sh/);

  const settings = JSON.parse(await readFile(settingsPath, "utf8"));
  assert.equal(settings.model, "opus"); // unrelated settings preserved
  assert.match(settings.statusLine.command, /garage-statusline\.sh/);

  // Execute the generated wrapper exactly as Claude Code would: feed it
  // stdin JSON, read stdout. The background curl call targets a port
  // nothing listens on (GARAGE_PORT above) and is silenced — its failure
  // must never affect stdout.
  const originalAlone = runWithStdin("sh", [originalScriptPath], "");
  const wrapped = runWithStdin(
    "sh",
    [wrapperPath],
    JSON.stringify({ session_id: "whatever", context_window: { used_percentage: 10 } })
  );

  assert.equal(wrapped, originalAlone);
  assert.equal(wrapped, "ORIGINAL-OUTPUT-MARKER\n");
});

test("reinstalling over garage's own wrapper is still idempotent (no re-chaining of itself)", async () => {
  const before1 = await readFile(settingsPath, "utf8");
  const res = await app.inject({ method: "POST", url: "/api/statusline/install" });
  assert.equal(res.json().alreadyInstalled, true);
  const after1 = await readFile(settingsPath, "utf8");
  assert.equal(after1, before1);
});

test("a fresh install with no pre-existing statusLine produces a wrapper that prints nothing", async (t) => {
  // Reset to a clean, statusLine-less settings.json.
  await writeFile(settingsPath, "{}\n", "utf8");
  const res = await app.inject({ method: "POST", url: "/api/statusline/install" });
  assert.equal(res.json().chained, false);

  const wrapped = runWithStdin("sh", [wrapperPath], JSON.stringify({ session_id: "x" }));
  assert.equal(wrapped, "");
});
