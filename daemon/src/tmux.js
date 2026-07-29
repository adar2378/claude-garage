import { execFile } from "node:child_process";
import { promisify } from "node:util";

const run = promisify(execFile);

export const GARAGE_PREFIX = "garage/";
export const NAME_RE = /^[a-z0-9-]+$/;

export function sessionId(workspace, label) {
  return `${GARAGE_PREFIX}${workspace}/${label}`;
}

// `=` forces exact-name matching; bare -t does prefix matching.
const exact = (id) => `=${id}`;

export async function listSessions() {
  let stdout;
  try {
    ({ stdout } = await run("tmux", [
      "ls",
      "-F",
      // `session_created` (epoch SECONDS) backs the UI's elapsed timer for a
      // session that has never had a status transition — without it a live
      // session with no recorded `since` renders "—" instead of its age.
      "#{session_name}\t#{session_path}\t#{session_attached}\t#{session_created}",
    ]));
  } catch {
    // tmux exits non-zero when no server is running — that means "no sessions".
    return [];
  }
  return stdout
    .split("\n")
    .filter((line) => line.startsWith(GARAGE_PREFIX))
    .map((line) => {
      const [name, dir, attached, created] = line.split("\t");
      const [, workspace, label] = name.split("/");
      const createdMs = Number(created) * 1000;
      return {
        id: name,
        workspace,
        label,
        dir,
        attached: attached !== "0",
        createdAt: Number.isFinite(createdMs) && createdMs > 0 ? createdMs : null,
      };
    });
}

// D-branch: live pane cwd per garage session, keyed by session name — NOT
// `session_path` (that's the session's *starting* dir, frozen at creation;
// a user who `cd`s into a worktree inside the pane changes pane_current_path
// but not session_path). `-a` lists every pane on the server across every
// session, so results are filtered to the garage prefix same as
// listSessions(). A session can have multiple panes/windows if the user
// split it manually — only the first pane encountered per session is kept,
// there's no principled way to pick "the" cwd of a multi-pane session.
export async function listPanePaths() {
  let stdout;
  try {
    ({ stdout } = await run("tmux", [
      "list-panes",
      "-a",
      "-F",
      "#{session_name}\t#{pane_current_path}",
    ]));
  } catch {
    // No tmux server running — no panes.
    return new Map();
  }
  const paths = new Map();
  for (const line of stdout.split("\n")) {
    if (!line.startsWith(GARAGE_PREFIX)) continue;
    const [name, panePath] = line.split("\t");
    if (!paths.has(name)) paths.set(name, panePath);
  }
  return paths;
}

// D-branch: shared by sessions.js (per-session, keyed off live pane cwd)
// and workspaces.js (per-workspace, keyed off the registered dir) — both
// just need "what branch is HEAD at in this directory", so the git call
// lives here once. `null` for anything that isn't a usable git working
// dir (no dir, not a repo, dir vanished, etc.) rather than throwing —
// callers render it as "no chip" and must never let a git failure break
// the sessions/workspaces list response.
export async function resolveBranch(dir) {
  if (!dir) return null;
  try {
    const { stdout } = await run("git", ["-C", dir, "rev-parse", "--abbrev-ref", "HEAD"]);
    const branch = stdout.trim();
    if (branch && branch !== "HEAD") return branch;
    // Detached HEAD — abbrev-ref returns the literal string "HEAD" — fall
    // back to a short sha, prefixed so it's visually distinct from a real
    // branch name in the UI chip.
    const { stdout: sha } = await run("git", ["-C", dir, "rev-parse", "--short", "HEAD"]);
    const short = sha.trim();
    return short ? `@${short}` : null;
  } catch {
    return null;
  }
}

export async function hasSession(id) {
  try {
    await run("tmux", ["has-session", "-t", exact(id)]);
    return true;
  } catch {
    return false;
  }
}

// extraArgs are appended as separate argv entries after `command` — tmux
// joins the trailing shell-command words itself (e.g. `command, "--resume",
// sessionId` becomes the pane's `claude --resume <sessionId>`), so callers
// must not pre-join them into a single string.
export async function createSession(id, dir, command, extraArgs = []) {
  await run("tmux", ["new-session", "-d", "-s", id, "-c", dir, command, ...extraArgs]);
  // The pit wall renders its own session title bars, so tmux's status line
  // is visual noise in every grid cell. Per-session option — a plain
  // `tmux attach` escape-hatch user can restore it with `set status on`.
  // set-option does not accept the `=` exact-match target prefix; the full
  // freshly-created name is unambiguous here.
  await run("tmux", ["set-option", "-t", id, "status", "off"]).catch(() => {});
}

export async function killSession(id) {
  await run("tmux", ["kill-session", "-t", exact(id)]);
}

// D-rename: renames the tmux session entity itself. Attached clients (grid
// ptys, iTerm) stay attached across a rename — they're bound to the session
// entity, not its name — so this is safe to call on a live, attached session.
export async function renameSession(oldId, newId) {
  await run("tmux", ["rename-session", "-t", exact(oldId), newId]);
}
