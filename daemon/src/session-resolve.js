import { execFile } from "node:child_process";
import { promisify } from "node:util";
import { listSessions } from "./tmux.js";

const run = promisify(execFile);

async function getAgents() {
  try {
    const { stdout } = await run("claude", ["agents", "--json"]);
    return JSON.parse(stdout);
  } catch {
    return [];
  }
}

async function getPanePids() {
  try {
    const { stdout } = await run("tmux", [
      "list-panes",
      "-a",
      "-F",
      "#{session_name}\t#{pane_pid}",
    ]);
    const map = new Map(); // pid -> session name
    for (const line of stdout.split("\n")) {
      if (!line.trim()) continue;
      const [name, pid] = line.split("\t");
      map.set(Number(pid), name);
    }
    return map;
  } catch {
    return new Map();
  }
}

// Shared by hooks.js and statusline.js — both receive a Claude Code payload
// carrying `session_id`/`cwd` and need the same garage session id(s) it
// applies to. Precise path: payload session_id -> agents-json sessionId ->
// its pid -> pane_pid join. Fallback: cwd fail-open, applied to every garage
// session sharing that dir (over-notify/over-record rather than silently
// drop).
export async function resolveSessionIds({ session_id, cwd }) {
  const [agents, pidToSession] = await Promise.all([getAgents(), getPanePids()]);

  if (session_id) {
    const agent = agents.find((a) => a.sessionId === session_id);
    const sessionName = agent && pidToSession.get(agent.pid);
    if (sessionName) return [sessionName];
  }

  if (cwd) {
    const sessions = await listSessions();
    const matches = sessions.filter((s) => s.dir === cwd).map((s) => s.id);
    if (matches.length > 0) return matches;
  }

  return [];
}
