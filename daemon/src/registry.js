import { readFile, writeFile, rename, mkdir, unlink } from "node:fs/promises";
import { homedir } from "node:os";
import path from "node:path";
import { randomUUID } from "node:crypto";
import { GARAGE_PREFIX } from "./tmux.js";

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

// D-rename: moves the workspace's registry key and rewrites every
// resume-metadata key that embeds the old name (`garage/<old>/<label>` ->
// `garage/<new>/<label>`, plus the meta's own `workspace` field) in one
// atomic write — mirrors the live tmux renames the caller performs
// separately. Callers are expected to have already validated oldName exists
// and newName is free; this function trusts its inputs.
export async function renameWorkspace(oldName, newName) {
  const state = await readState();
  const entry = state.workspaces[oldName];
  if (!entry) return null;

  delete state.workspaces[oldName];
  state.workspaces[newName] = entry;

  const oldPrefix = `${GARAGE_PREFIX}${oldName}/`;
  const newPrefix = `${GARAGE_PREFIX}${newName}/`;
  const nextSessions = {};
  for (const [id, meta] of Object.entries(state.sessions)) {
    if (id.startsWith(oldPrefix)) {
      const newId = newPrefix + id.slice(oldPrefix.length);
      nextSessions[newId] = { ...meta, workspace: newName };
    } else {
      nextSessions[id] = meta;
    }
  }
  state.sessions = nextSessions;

  await writeState(state);
  return { name: newName, dir: entry.dir };
}

// D-resume-meta / D-wt-meta: sessions map, keyed by garage session id,
// {claudeSessionId, workspace, label, worktree}. `claudeSessionId`/
// `workspace`/`label` are written opportunistically by the poller whenever
// it observes a session's Claude Code sessionId (see poller.js); `worktree`
// (`{path, branch, repoDir}` or `null`) is written once, at spawn, by
// sessions.js and never touched by the poller — see upsertSessionMeta's
// field-level merge below. Deleted only on a deliberate DELETE
// /api/sessions/:id (see sessions.js) — a session vanishing from tmux any
// other way (reboot, `tmux kill-server`, an out-of-band `tmux kill-session`)
// leaves its meta in place so it can be offered as restorable.
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
//
// D-wt-meta: field-level merge, not a blind overwrite. Two independent
// writers share this record — sessions.js writes `worktree` once, at spawn,
// and never touches `claudeSessionId`/`label`/`workspace` again; poller.js
// writes `claudeSessionId`/`workspace`/`label` on every tick once it learns
// the Claude Code sessionId, and never mentions `worktree`. A field absent
// (`undefined`) from the incoming `meta` falls back to whatever's already on
// disk, so neither writer can clobber the other's field — only a caller that
// explicitly passes a field (including `null`) can change or clear it.
export async function upsertSessionMeta(id, meta) {
  const state = await readState();
  const existing = state.sessions[id];
  const next = {
    claudeSessionId:
      meta.claudeSessionId !== undefined ? meta.claudeSessionId : (existing?.claudeSessionId ?? null),
    workspace: meta.workspace !== undefined ? meta.workspace : existing?.workspace,
    label: meta.label !== undefined ? meta.label : existing?.label,
    worktree: meta.worktree !== undefined ? meta.worktree : (existing?.worktree ?? null),
  };
  const unchanged = existing && JSON.stringify(existing) === JSON.stringify(next);
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

// Deleting a workspace drops its registry entry AND its sessions' resume
// metadata (they could never restore without a registered dir — a 409 loop),
// but by design never touches tmux; see workspaces.js DELETE.
export async function removeWorkspace(name) {
  const state = await readState();
  if (!(name in state.workspaces)) return;
  delete state.workspaces[name];
  const prefix = `garage/${name}/`;
  for (const id of Object.keys(state.sessions)) {
    if (id.startsWith(prefix)) delete state.sessions[id];
  }
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
