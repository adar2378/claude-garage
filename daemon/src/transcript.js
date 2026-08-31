import { open } from "node:fs/promises";
import { join } from "node:path";
import { claudeHome } from "./claude-home.js";

const TAIL_BYTES = 256 * 1024;
const TTL_MS = 15_000;
const DEFAULT_WINDOW = 200_000;

// Claude Code's own project-directory naming: the cwd with every `/`
// replaced by `-` (leading slash included). Exported — pure — for direct
// testing.
export function slugifyCwd(cwd) {
  return cwd.replaceAll("/", "-");
}

export function transcriptPath(dir, claudeSessionId) {
  return join(claudeHome(), "projects", slugifyCwd(dir), `${claudeSessionId}.jsonl`);
}

// Small map of known exceptions to the 200k default — "[1m]" marks the
// 1M-context beta variants of a model id (e.g.
// "claude-sonnet-4-5-20250929[1m]"). Anything else, including an unknown or
// missing model id, gets the default: a hint, not billing, per design.md.
export function windowForModel(model) {
  if (typeof model !== "string") return DEFAULT_WINDOW;
  if (model.includes("[1m]")) return 1_000_000;
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
