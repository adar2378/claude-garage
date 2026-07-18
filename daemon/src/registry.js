import { readFile, writeFile, rename, mkdir, unlink } from "node:fs/promises";
import { homedir } from "node:os";
import path from "node:path";
import { randomUUID } from "node:crypto";

// D-registry: ~/.garage/state.json {workspaces:{name:{dir}}}.
// Created lazily on first write — a user who registers nothing has no file
// to migrate or corrupt. tmux ls remains the sole source of truth for
// sessions; this file is a directory mapping only, never a session list.
const GARAGE_DIR = path.join(homedir(), ".garage");
const STATE_PATH = path.join(GARAGE_DIR, "state.json");

async function readState() {
  try {
    const raw = await readFile(STATE_PATH, "utf8");
    const parsed = JSON.parse(raw);
    const workspaces =
      parsed && typeof parsed.workspaces === "object" && parsed.workspaces !== null
        ? parsed.workspaces
        : {};
    // D-resume-meta: sessions map defaulted the same way workspaces always
    // has been — a pre-P3 state.json (no `sessions` key) loads exactly as it
    // did before, and silently gains the key on the first write post-upgrade.
    // Spreading `parsed` preserves hookToken and any other existing keys.
    const sessions =
      parsed && typeof parsed.sessions === "object" && parsed.sessions !== null
        ? parsed.sessions
        : {};
    return { ...parsed, workspaces, sessions };
  } catch {
    // Missing file, unreadable, or malformed JSON — treat as empty registry.
    return { workspaces: {}, sessions: {} };
  }
}

// Atomic write: temp file in the same directory, then rename(). rename() is
// atomic on the same filesystem, so a crash mid-write leaves the previous
// complete file or the new complete file — never a truncated one.
async function writeState(state) {
  await mkdir(GARAGE_DIR, { recursive: true });
  const tmpPath = path.join(GARAGE_DIR, `.state.json.${process.pid}.${randomUUID()}.tmp`);
  try {
    await writeFile(tmpPath, JSON.stringify(state, null, 2));
    await rename(tmpPath, STATE_PATH);
  } catch (err) {
    await unlink(tmpPath).catch(() => {});
    throw err;
  }
}

export async function listWorkspaces() {
  const state = await readState();
  return Object.entries(state.workspaces).map(([name, entry]) => ({
    name,
    dir: entry.dir,
  }));
}

export async function getWorkspace(name) {
  const state = await readState();
  return state.workspaces[name] ?? null;
}

export async function upsertWorkspace(name, dir) {
  const state = await readState();
  state.workspaces[name] = { dir };
  await writeState(state);
  return { name, dir };
}

// D-resume-meta: sessions map, keyed by garage session id, {claudeSessionId,
// workspace, label}. Written opportunistically by the poller whenever it
// observes a session's Claude Code sessionId (see poller.js), deleted only on
// a deliberate DELETE /api/sessions/:id (see sessions.js) — a session
// vanishing from tmux any other way (reboot, `tmux kill-server`, an
// out-of-band `tmux kill-session`) leaves its meta in place so it can be
// offered as restorable.
export async function getSessionMeta(id) {
  const state = await readState();
  return state.sessions[id] ?? null;
}

export async function listSessionMetas() {
  const state = await readState();
  return Object.entries(state.sessions).map(([id, meta]) => ({ id, ...meta }));
}

// Write only if changed: a session's claudeSessionId is stable for the life
// of one conversation, so after the first tick following spawn this is a
// no-op comparison on every subsequent poll — the file is touched once per
// session's lifetime in the common case, mirroring getHookToken()'s
// generate-and-persist-once discipline and avoiding the write-amplification
// D-registry (P0) was careful to avoid.
export async function upsertSessionMeta(id, meta) {
  const state = await readState();
  const next = {
    claudeSessionId: meta.claudeSessionId,
    workspace: meta.workspace,
    label: meta.label,
  };
  const existing = state.sessions[id];
  const unchanged =
    existing &&
    existing.claudeSessionId === next.claudeSessionId &&
    existing.workspace === next.workspace &&
    existing.label === next.label;
  if (unchanged) return existing;

  state.sessions[id] = next;
  await writeState(state);
  return next;
}

export async function removeSessionMeta(id) {
  const state = await readState();
  if (!(id in state.sessions)) return;
  delete state.sessions[id];
  await writeState(state);
}

// Per-install shared secret for the hook endpoint (CSRF defense-in-depth):
// generated once, embedded in the hook snippet URL, persisted so daemon
// restarts don't invalidate hooks already installed in ~/.claude/settings.json.
export async function getHookToken() {
  const state = await readState();
  if (typeof state.hookToken === "string" && state.hookToken.length >= 32) {
    return state.hookToken;
  }
  const { randomBytes } = await import("node:crypto");
  state.hookToken = randomBytes(32).toString("hex");
  await writeState(state);
  return state.hookToken;
}
