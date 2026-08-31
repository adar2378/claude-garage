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

// p10: pane titles are free text — tmux doesn't forbid a literal tab in one
// (unlikely, but "typically" isn't "never"). `#{pane_title}` sits LAST in
// the format string and everything from the second tab onward is rejoined
// into the title field, so an embedded separator there can never shift
// session_name/pane_current_path out from under the earlier destructuring.
// Exported (pure, no tmux call) so the split logic is unit-testable without
// a live server.
export function parsePaneLine(line) {
  const [name, panePath, ...rest] = line.split("\t");
  return { name, panePath, title: rest.join("\t") };
}

// D-branch: live pane cwd + title per garage session, keyed by session name
// — NOT `session_path` (that's the session's *starting* dir, frozen at
// creation; a user who `cd`s into a worktree inside the pane changes
// pane_current_path but not session_path). `-a` lists every pane on the
// server across every session, so results are filtered to the garage prefix
// same as listSessions(). A session can have multiple panes/windows if the
// user split it manually — only the first pane encountered per session is
// kept, there's no principled way to pick "the" cwd/title of a multi-pane
// session.
export async function listPanePaths() {
  let stdout;
  try {
    ({ stdout } = await run("tmux", [
      "list-panes",
      "-a",
      "-F",
      "#{session_name}\t#{pane_current_path}\t#{pane_title}",
    ]));
  } catch {
    // No tmux server running — no panes.
    return new Map();
  }
  const panes = new Map();
  for (const line of stdout.split("\n")) {
    if (!line.startsWith(GARAGE_PREFIX)) continue;
    const { name, panePath, title } = parsePaneLine(line);
    if (!panes.has(name)) panes.set(name, { path: panePath, title });
  }
  return panes;
}

// p10: tmux's own default pane titles carry no signal — empty, the machine
// hostname (tmux's out-of-the-box default), or the bare shell/login-shell
// name it sets before anything else runs. Filtering those down to null
// means the UI only ever sees a title something *set* (e.g. Claude Code's
// OSC title updates). Pure — hostname is passed in rather than read via
// os.hostname() internally so this stays trivially unit-testable; the one
// production caller (sessions.js) passes os.hostname().
const BARE_PROCESS_NAMES = new Set([
  "sh",
  "bash",
  "zsh",
  "fish",
  "-sh",
  "-bash",
  "-zsh",
  "-fish",
  "tmux",
]);

export function normalizeTitle(title, hostname) {
  if (title == null) return null;
  const trimmed = title.trim();
  if (!trimmed) return null;

  const lower = trimmed.toLowerCase();
  if (BARE_PROCESS_NAMES.has(lower)) return null;

  if (hostname) {
    const hostLower = hostname.toLowerCase();
    const shortHost = hostLower.split(".")[0];
    if (lower === hostLower || lower === shortHost) return null;
  }

  return trimmed;
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
