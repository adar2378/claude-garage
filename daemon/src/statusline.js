import { chmod, mkdir, rename, writeFile } from "node:fs/promises";
import { dirname, join } from "node:path";
import { getHookToken } from "./registry.js";
import { tokenMatches } from "./token-auth.js";
import { resolveSessionIds } from "./session-resolve.js";
import { readSettingsOrRefuse, writeSettingsAtomic } from "./settings-install.js";
import { claudeHome } from "./claude-home.js";

const PORT = Number(process.env.GARAGE_PORT ?? 4747);
const SETTINGS_PATH = join(claudeHome(), "settings.json");
const WRAPPER_PATH = join(claudeHome(), "garage-statusline.sh");

// ---------------------------------------------------------------------------
// In-memory store — mirrors status.js's shape (a Map, no persistence, wiped
// per the id it's keyed on). Two slots: per-session context percentage, and
// ONE account-wide rate-limits record (statusline posts carry the whole
// account's rate limits on every post, not a per-session figure).
// ---------------------------------------------------------------------------

const contextStore = new Map(); // garage session id -> { usedPercentage }
let rateLimits = { fiveHour: null, sevenDay: null };

// Exported so the pure clamp logic is directly unit-testable (mirrors
// hooks.js's isIdleReminder being exported for the same reason).
export function clampPercentage(value) {
  if (typeof value !== "number" || !Number.isFinite(value)) return null;
  return Math.min(100, Math.max(0, value));
}

export function setContext(id, usedPercentage) {
  contextStore.set(id, { usedPercentage });
}

// Read shape matches what sessions.js puts on the listing entry directly:
// {usedPercentage, source: "statusline"} | null.
export function getStatuslineContext(id) {
  const entry = contextStore.get(id);
  return entry ? { usedPercentage: entry.usedPercentage, source: "statusline" } : null;
}

// Called (like status.js's dropSession) when a session disappears from tmux,
// so a future reuse of the same garage session id starts clean — see
// poller.js. Also usable directly by tests.
export function dropSession(id) {
  contextStore.delete(id);
}

function normalizeLimit(limit) {
  if (!limit || typeof limit !== "object") return null;
  const usedPercentage = clampPercentage(limit.used_percentage);
  if (usedPercentage === null) return null;
  return { usedPercentage, resetsAt: limit.resets_at ?? null };
}

// Partial-update semantics: a post carrying only one of the two buckets
// updates that bucket and leaves the other exactly as it was — a statusline
// post's rate_limits object isn't guaranteed to always carry both.
export function setRateLimits(raw) {
  if (!raw || typeof raw !== "object") return;
  const fiveHour = normalizeLimit(raw.five_hour);
  const sevenDay = normalizeLimit(raw.seven_day);
  if (fiveHour) rateLimits = { ...rateLimits, fiveHour };
  if (sevenDay) rateLimits = { ...rateLimits, sevenDay };
}

export function getRateLimits() {
  return { fiveHour: rateLimits.fiveHour, sevenDay: rateLimits.sevenDay };
}

// ---------------------------------------------------------------------------
// Chaining wrapper — see design.md D-wrapper. The wrapper is a tiny POSIX sh
// script (not an inline settings.json command string) so chaining an
// arbitrary pre-existing command never fights JSON/shell quoting inside
// settings.json itself; settings.json only ever gets a fixed `sh <path>`
// command pointing at it.
// ---------------------------------------------------------------------------

// Single-quote a string for safe embedding in a POSIX sh script: close the
// quote, emit an escaped literal quote, reopen it.
function shSingleQuote(str) {
  return `'${String(str).replace(/'/g, `'\\''`)}'`;
}

// `originalCommand` is the pre-existing statusLine command to chain (its
// stdout must ride through unchanged), or null when there was none.
export function wrapperScript({ token, port = PORT, originalCommand = null }) {
  const url = `http://127.0.0.1:${port}/api/statusline/claude?token=${token}`;
  const chain = originalCommand
    ? `printf '%s' "$INPUT" | sh -c ${shSingleQuote(originalCommand)}\n`
    : ""; // no pre-existing statusline — post and print nothing, per spec
  return (
    `#!/bin/sh\n` +
    `# claude-garage statusline wrapper — auto-generated, do not hand-edit.\n` +
    `# Reinstall (POST /api/statusline/install) regenerates this file.\n` +
    `INPUT="$(cat)"\n` +
    `printf '%s' "$INPUT" | curl -fsS --max-time 1 -X POST ${shSingleQuote(url)} ` +
    `-H "Content-Type: application/json" --data-binary @- >/dev/null 2>&1 &\n` +
    chain
  );
}

function wrapperCommand() {
  return `sh ${shSingleQuote(WRAPPER_PATH)}`;
}

async function writeWrapperScript(content) {
  await mkdir(dirname(WRAPPER_PATH), { recursive: true });
  const tmp = `${WRAPPER_PATH}.garage-tmp-${process.pid}`;
  await writeFile(tmp, content, "utf8");
  await chmod(tmp, 0o755);
  await rename(tmp, WRAPPER_PATH);
}

export async function statuslineSnippet() {
  const token = await getHookToken();
  return {
    statusLine: { type: "command", command: wrapperCommand() },
    script: wrapperScript({ token, originalCommand: null }),
  };
}

// Same safety order as hooks.js's installHooks: parse-or-refuse -> compute
// the merge -> backup + atomic write only if something actually changes.
// Idempotency: if settings.statusLine.command already IS our wrapper
// command, there's nothing new to capture or write (see design.md's note on
// the GARAGE_PORT-change edge case — out of scope here, matches hooks.js's
// own idempotency granularity).
async function installStatusline() {
  const { settings, raw } = await readSettingsOrRefuse(SETTINGS_PATH);
  const token = await getHookToken();
  const command = wrapperCommand();

  const existing = settings.statusLine ?? null;
  if (existing && existing.command === command) {
    return { ok: true, installed: true, alreadyInstalled: true, chained: null, backup: null };
  }

  // Only a `type: "command"` statusLine has a shell command we can chain;
  // anything else (absent, or an unrecognized future type) is treated as
  // "nothing to chain" rather than erroring — the wrapper simply posts and
  // prints nothing, same as a fresh install.
  const originalCommand =
    existing && existing.type === "command" && typeof existing.command === "string"
      ? existing.command
      : null;

  await writeWrapperScript(wrapperScript({ token, originalCommand }));

  const merged = { ...settings, statusLine: { type: "command", command } };
  const backup = await writeSettingsAtomic(SETTINGS_PATH, raw, merged);

  return {
    ok: true,
    installed: true,
    alreadyInstalled: false,
    chained: originalCommand !== null,
    backup,
  };
}

export default async function statuslineRoutes(app) {
  // Auth: same per-install token as hooks.js's /api/hooks/claude — the
  // wrapper script is URL-only config (no way to send a header), so the
  // token rides as a query param.
  app.post("/api/statusline/claude", async (req, reply) => {
    const expected = await getHookToken();
    if (!tokenMatches(req.query?.token, expected)) {
      return reply.code(401).send({ error: "missing or invalid hook token" });
    }

    const payload = req.body ?? {};
    const ids = await resolveSessionIds(payload);
    if (ids.length === 0) {
      // Unresolvable session — no error, nothing recorded (context or
      // account-wide rate limits): a post that can't be attributed to a
      // known garage session tells us nothing trustworthy.
      return reply.code(200).send({ ok: true, ignored: true });
    }

    const pct = clampPercentage(payload?.context_window?.used_percentage);
    if (pct !== null) {
      for (const id of ids) setContext(id, pct);
    }
    setRateLimits(payload?.rate_limits);

    return reply.code(200).send({ ok: true, applied: ids });
  });

  app.get("/api/statusline/snippet", async () => statuslineSnippet());

  app.post("/api/statusline/install", async (req, reply) => {
    try {
      return await installStatusline();
    } catch (err) {
      return reply
        .code(err.statusCode ?? 500)
        .send({ error: err.message ?? "statusline install failed" });
    }
  });
}
