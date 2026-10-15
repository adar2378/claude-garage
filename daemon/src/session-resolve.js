import { listSessions, listPaneInfo, runClaude } from "./tmux.js";

async function getAgents() {
  try {
    const { stdout } = await runClaude(["agents", "--json"]);
    return JSON.parse(stdout);
  } catch {
    return [];
  }
}

async function getPanePids() {
  try {
    const map = new Map(); // pid -> session name
    for (const [name, pane] of await listPaneInfo()) map.set(pane.pid, name);
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
