import { execFile } from "node:child_process";
import { promisify } from "node:util";
import { EventEmitter } from "node:events";
import { listSessions } from "./tmux.js";
import { setStatus, getStatus, dropSession } from "./status.js";
import { upsertSessionMeta } from "./registry.js";

const run = promisify(execFile);
const POLL_MS = 2000;

// Emits "sessions-changed" whenever the poller notices the garage session
// list itself changed (spawn/death), so events.js can tell the UI to refetch.
export const pollerEvents = new EventEmitter();
pollerEvents.setMaxListeners(0);

let lastSessionIds = new Set();

// `claude agents --json` — missing CLI or any failure is a silent no-op,
// per D-status's graceful degradation to poller-absent behavior.
async function getAgents() {
  try {
    const { stdout } = await run("claude", ["agents", "--json"]);
    return JSON.parse(stdout);
  } catch {
    return [];
  }
}

// tmux pane pid per session name — pane_pid joins exactly to the `pid`
// reported by `claude agents --json` since garage spawns claude as the
// pane's root process (design D-status).
async function getPanePids() {
  try {
    const { stdout } = await run("tmux", [
      "list-panes",
      "-a",
      "-F",
      "#{session_name}\t#{pane_pid}",
    ]);
    const map = new Map();
    for (const line of stdout.split("\n")) {
      if (!line.trim()) continue;
      const [name, pid] = line.split("\t");
      map.set(name, Number(pid));
    }
    return map;
  } catch {
    return new Map();
  }
}

function applyAgentStatus(id, agentStatus) {
  const mapped = agentStatus === "busy" ? "working" : "idle";
  // The poller's coarse 'idle' must not stomp the hook-sourced precision
  // states: needs-input clears only on real activity, and done holds until
  // its decay timer (status.js) or a 'busy' observation (-> working).
  const current = getStatus(id);
  if (mapped === "idle" && (current === "needs-input" || current === "done")) return;
  setStatus(id, mapped);
}

async function tick() {
  const sessions = await listSessions().catch(() => []);

  const currentIds = new Set(sessions.map((s) => s.id));
  let changed = currentIds.size !== lastSessionIds.size;
  for (const id of lastSessionIds) {
    if (!currentIds.has(id)) {
      dropSession(id);
      changed = true;
    }
  }
  if (!changed) {
    for (const id of currentIds) {
      if (!lastSessionIds.has(id)) {
        changed = true;
        break;
      }
    }
  }
  lastSessionIds = currentIds;
  if (changed) pollerEvents.emit("sessions-changed");

  if (sessions.length === 0) return;

  const [agents, panePids] = await Promise.all([getAgents(), getPanePids()]);
  if (agents.length === 0) return; // claude CLI missing/no live agents

  const agentByPid = new Map(agents.map((a) => [a.pid, a]));
  const unmatched = [];

  for (const session of sessions) {
    const panePid = panePids.get(session.id);
    const agent = panePid !== undefined ? agentByPid.get(panePid) : undefined;
    if (agent) {
      applyAgentStatus(session.id, agent.status);
      // D-resume-meta: only on the pid-join succeeding do we have a trusted
      // sessionId for this garage session; upsertSessionMeta itself is a
      // no-op write when nothing changed. Never let a meta-write failure
      // interrupt status polling for the rest of the tick.
      if (agent.sessionId) {
        await upsertSessionMeta(session.id, {
          claudeSessionId: agent.sessionId,
          workspace: session.workspace,
          label: session.label,
        }).catch(() => {});
      }
    } else {
      unmatched.push(session);
    }
  }

  // Fail-open fallback: pid join missed (e.g. claude exec'd through a
  // wrapper) — apply the cwd-matching agent's status to ALL sessions in
  // that dir. Over-notify rather than silently drop.
  for (const session of unmatched) {
    const cwdAgent = agents.find((a) => a.cwd === session.dir);
    if (cwdAgent) applyAgentStatus(session.id, cwdAgent.status);
  }
}

export function startPoller(app) {
  const timer = setInterval(() => {
    tick().catch((err) => app?.log?.warn?.({ err }, "poller tick failed"));
  }, POLL_MS);
  timer.unref?.();
  return () => clearInterval(timer);
}
