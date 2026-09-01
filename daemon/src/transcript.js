import { open } from "node:fs/promises";
import { join } from "node:path";
import { claudeHome } from "./claude-home.js";

const TAIL_BYTES = 256 * 1024;
const TTL_MS = 15_000;
const DEFAULT_WINDOW = 1_000_000;

// Claude Code's own project-directory naming: the cwd with every `/`
// replaced by `-` (leading slash included). Exported — pure — for direct
// testing.
export function slugifyCwd(cwd) {
  return cwd.replaceAll("/", "-");
}

export function transcriptPath(dir, claudeSessionId) {
  return join(claudeHome(), "projects", slugifyCwd(dir), `${claudeSessionId}.jsonl`);
}

// Model ids known to have a 200k window — Haiku, the 3.x family, and
// everything before Opus/Sonnet 4.6 (the 4.6+ generation and the 5 family
// are natively 1M). Prefix-matched so dated snapshot ids
// ("claude-sonnet-4-20250514") land in the right bucket.
const SMALL_WINDOW_PREFIXES = [
  "claude-haiku",
  "claude-3",
  "claude-opus-4-0",
  "claude-opus-4-1",
  "claude-opus-4-2", // dated Opus 4 ids: claude-opus-4-20250514
  "claude-opus-4-5",
  "claude-sonnet-4-0",
  "claude-sonnet-4-2", // dated Sonnet 4 ids: claude-sonnet-4-20250514
  "claude-sonnet-4-5",
];

// Learned model id -> window size, fed by statusline posts (Claude Code's
// own `context_window.context_window_size`) — the only authoritative local
// source; no file on disk maps model ids to windows. In-memory like the
// statusline context store: wrong-until-first-post beats stale-forever.
const modelWindows = new Map();

export function recordModelWindow(model, size) {
  if (typeof model !== "string" || !model) return;
  if (typeof size !== "number" || !Number.isFinite(size) || size < 1000) return;
  modelWindows.set(model, size);
}

// Precedence: a window Claude Code told us about via statusline, then the
// "[1m]" beta-variant marker on old model ids
// ("claude-sonnet-4-5-20250929[1m]"), then the known-200k list, then the
// 1M default (everything current is natively 1M). A hint, not billing,
// per design.md.
export function windowForModel(model) {
  if (typeof model !== "string") return DEFAULT_WINDOW;
  const learned = modelWindows.get(model);
  if (learned !== undefined) return learned;
  if (model.includes("[1m]")) return 1_000_000;
  if (SMALL_WINDOW_PREFIXES.some((p) => model.startsWith(p))) return 200_000;
  return DEFAULT_WINDOW;
}

// Scans `text` (the tail-bounded transcript content) backward for the last
// assistant message carrying a `usage` block, skipping malformed JSON lines
// and any line that isn't an assistant/usage entry. Returns
// {tokens, model} | null. Pure — exported for direct unit tests.
export function parseLastAssistantUsage(text) {
  const lines = text.split("\n");
  for (let i = lines.length - 1; i >= 0; i--) {
    const line = lines[i].trim();
    if (!line) continue;

    let entry;
    try {
      entry = JSON.parse(line);
    } catch {
      continue; // malformed line (or a partial line from the tail cut) — skip
    }

    if (entry?.type !== "assistant") continue;
    const usage = entry?.message?.usage;
    if (!usage || typeof usage !== "object") continue;

    const input = Number(usage.input_tokens) || 0;
    const cacheRead = Number(usage.cache_read_input_tokens) || 0;
    const cacheCreation = Number(usage.cache_creation_input_tokens) || 0;
    return { tokens: input + cacheRead + cacheCreation, model: entry.message.model ?? null };
  }
  return null;
}

// Reads only the last <=256KB of the file (a multi-MB transcript must never
// cost a full read). Returns the tail text, or null if the file doesn't
// exist / can't be read. The first line of the tail may be a partial line
// cut mid-record — parseLastAssistantUsage tolerates that by skipping
// unparseable lines, which is exactly what a truncated leading line is.
async function readTail(path) {
  let handle;
  try {
    handle = await open(path, "r");
    const { size } = await handle.stat();
    const start = Math.max(0, size - TAIL_BYTES);
    const length = size - start;
    const buffer = Buffer.alloc(length);
    if (length > 0) {
      await handle.read(buffer, 0, length, start);
    }
    return buffer.toString("utf8");
  } catch {
    return null;
  } finally {
    await handle?.close().catch(() => {});
  }
}

// The percentage the fallback exposes: (input + cache_read + cache_creation)
// over the model's context window. Async, does real file IO — callers on
// the hot path must use the cache below (getCachedContext/refreshContext),
// never call this directly from a request handler.
export async function computeContext({ dir, claudeSessionId }) {
  if (!dir || !claudeSessionId) return null;
  const tail = await readTail(transcriptPath(dir, claudeSessionId));
  if (tail === null) return null;

  const usage = parseLastAssistantUsage(tail);
  if (!usage) return null; // no assistant usage within the tail-bounded read

  const window = windowForModel(usage.model);
  const usedPercentage = Math.min(100, Math.max(0, Math.round((usage.tokens / window) * 100)));
  return { usedPercentage, source: "transcript" };
}

// ---------------------------------------------------------------------------
// Per-session cache: {value, at, pending}. `value` is null until the first
// successful (or empty) read completes. The sessions route (sessions.js)
// must NEVER block on a transcript read — it calls getCachedContext for the
// synchronous, always-available answer, and refreshContext to kick a
// background read when the cache is stale. refreshContext returns
// immediately in every case; its promise is intentionally not returned so a
// caller can't accidentally await it into the request path.
// ---------------------------------------------------------------------------

const cache = new Map(); // garage session id -> {value, at, pending}

export function getCachedContext(id) {
  return cache.get(id)?.value ?? null;
}

// Exposes the full cache entry (including `at`) — mainly for tests to
// observe TTL behavior without waiting on wall-clock time.
export function getCacheEntry(id) {
  return cache.get(id) ?? { value: null, at: null };
}

export function clearCache(id) {
  cache.delete(id);
}

export function refreshContext(id, { dir, claudeSessionId }) {
  const entry = cache.get(id);
  const fresh = entry && Date.now() - entry.at < TTL_MS;
  if (fresh || entry?.pending) return;

  cache.set(id, { value: entry?.value ?? null, at: entry?.at ?? 0, pending: true });
  computeContext({ dir, claudeSessionId })
    .then((value) => {
      cache.set(id, { value, at: Date.now(), pending: false });
    })
    .catch(() => {
      // A read failure keeps whatever was already cached (stale-but-known
      // beats null) but still stamps `at` so a persistently failing read
      // doesn't retry every single request.
      cache.set(id, { value: entry?.value ?? null, at: Date.now(), pending: false });
    });
}
