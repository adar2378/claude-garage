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
      "#{session_name}\t#{session_path}\t#{session_attached}",
    ]));
  } catch {
    // tmux exits non-zero when no server is running — that means "no sessions".
    return [];
  }
  return stdout
    .split("\n")
    .filter((line) => line.startsWith(GARAGE_PREFIX))
    .map((line) => {
      const [name, dir, attached] = line.split("\t");
      const [, workspace, label] = name.split("/");
      return { id: name, workspace, label, dir, attached: attached !== "0" };
    });
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
}

export async function killSession(id) {
  await run("tmux", ["kill-session", "-t", exact(id)]);
}
