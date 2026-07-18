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
    if (!parsed || typeof parsed.workspaces !== "object" || parsed.workspaces === null) {
      return { workspaces: {} };
    }
    return parsed;
  } catch {
    // Missing file, unreadable, or malformed JSON — treat as empty registry.
    return { workspaces: {} };
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
