import { execFile } from "node:child_process";
import { promisify } from "node:util";
import { EventEmitter } from "node:events";
import os from "node:os";
import { listSessions, normalizeTitle } from "./tmux.js";
import { setStatus, getStatus, dropSession } from "./status.js";
import { upsertSessionMeta } from "./registry.js";

const run = promisify(execFile);
const POLL_MS = 2000;

// Emits "sessions-changed" whenever the poller notices something the wall's
// session listing depends on changed: the garage session set itself
// (spawn/death) OR — p10 — a live session's normalized pane title, so a
// `tmux select-pane -T` (e.g. Claude Code's OSC title) gets pushed to the
// UI instead of only being picked up on the next unrelated refetch.
export const pollerEvents = new EventEmitter();
pollerEvents.setMaxListeners(0);

let lastSessionIds = new Set();
// p10: normalized title per currently-live garage session, from the tick
// this map was last rebuilt in. Rebuilt wholesale every tick from `sessions`
// (never merged), so a session that dies has its entry dropped for free —
// memory-trivial, sessions-only.
let lastTitles = new Map();

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

// tmux pane pid + title per session name — pane_pid joins exactly to the
// `pid` reported by `claude agents --json` since garage spawns claude as the
// pane's root process (design D-status). p10: pane_title rides the same
// list-panes call (kept to ONE per tick daemon-wide) so the title-change
// diff below needs no tmux invocation of its own; title is free text, so it
// sits last in the format string and everything past the second tab is
// rejoined into it (same rationale as tmux.js's parsePaneLine).
async function getPanePids() {
  try {
    const { stdout } = await run("tmux", [
      "list-panes",
      "-a",
      "-F",
      "#{session_name}\t#{pane_pid}\t#{pane_title}",
    ]);
    const map = new Map();
    for (const line of stdout.split("\n")) {
      if (!line.trim()) continue;
      const [name, pid, ...rest] = line.split("\t");
      map.set(name, { pid: Number(pid), title: rest.join("\t") });
    }
    return map;
  } catch {
    return new Map();
  }
}

// `claude agents --json` self-reports busy | waiting | idle. `waiting`
// means blocked on the user (permission prompt, question) — that IS
// needs-input, just 2s-coarse. Mapping it to idle (the pre-publish bug)
// meant hookless installs never saw the product's core signal at all.
// Exported for the test suite.
export function applyAgentStatus(id, agentStatus) {
  const mapped =
    agentStatus === "busy" ? "working" : agentStatus === "waiting" ? "needs-input" : "idle";
  // The poller's coarse 'idle' must not stomp the hook-sourced precision
  // states: needs-input clears only on real activity, and done holds until
  // its decay timer (status.js) or a 'busy' observation (-> working).
  const current = getStatus(id);
  if (mapped === "idle" && (current === "needs-input" || current === "done")) return;
  setStatus(id, mapped);
}

// p10 wave-2 fix: pure id+title diff, no tmux/IO — exported so it's
// exhaustively unit-testable without a live tmux server. `sessions` is this
// tick's listSessions() result; `panePids` maps session id -> {pid, title}
// (from this tick's single list-panes call — a session can be missing an
// entry, e.g. mid-spawn); `prevIds`/`prevTitles` are the previous tick's
// state. Session-id-set changes (spawn/death) AND a normalized-title change
// on any still-live session both count as "changed" — a pane-title-only
// change (`tmux select-pane -T`) never touches the id set, so without the
// title half this would sit unpushed until some unrelated refetch noticed.
export function diffSessionState(sessions, panePids, hostname, prevIds, prevTitles) {
  const currentIds = new Set(sessions.map((s) => s.id));
  let changed = currentIds.size !== prevIds.size;
  if (!changed) {
    for (const id of prevIds) {
      if (!currentIds.has(id)) {
        changed = true;
        break;
      }
    }
  }
  if (!changed) {
    for (const id of currentIds) {
      if (!prevIds.has(id)) {
        changed = true;
        break;
      }
    }
  }

  const titles = new Map();
  for (const session of sessions) {
    const title = normalizeTitle(panePids.get(session.id)?.title, hostname);
    titles.set(session.id, title);
    if (prevTitles.get(session.id) !== title) changed = true;
  }

  return { changed, ids: currentIds, titles };
}

async function tick() {
  const sessions = await listSessions().catch(() => []);

  if (sessions.length === 0) {
    for (const id of lastSessionIds) dropSession(id);
    const changed = lastSessionIds.size !== 0;
    lastSessionIds = new Set();
    lastTitles = new Map();
    if (changed) pollerEvents.emit("sessions-changed");
    return;
  }

  // p10: title rides the same list-panes call already made for the pid
  // join — kept to ONE list-panes invocation per tick daemon-wide.
  const [agents, panePids] = await Promise.all([getAgents(), getPanePids()]);
  const hostname = os.hostname();
  const { changed, ids, titles } = diffSessionState(
    sessions,
    panePids,
    hostname,
    lastSessionIds,
    lastTitles
  );
  for (const id of lastSessionIds) {
    if (!ids.has(id)) dropSession(id);
  }
  lastSessionIds = ids;
  lastTitles = titles;
  if (changed) pollerEvents.emit("sessions-changed");

  if (agents.length === 0) return; // claude CLI missing/no live agents

  const agentByPid = new Map(agents.map((a) => [a.pid, a]));
  const unmatched = [];

  for (const session of sessions) {
    const panePid = panePids.get(session.id)?.pid;
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
