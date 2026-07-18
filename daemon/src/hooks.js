import { execFile } from "node:child_process";
import { promisify } from "node:util";
import { listSessions } from "./tmux.js";
import { setStatus } from "./status.js";

const run = promisify(execFile);
const PORT = Number(process.env.GARAGE_PORT ?? 4747);
const HOOK_URL = `http://127.0.0.1:${PORT}/api/hooks/claude`;

const EVENT_TO_STATE = {
  Notification: "needs-input",
  Stop: "done",
};

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

// Resolve a hook payload to the garage session id(s) it applies to.
// Precise path: payload session_id -> agents-json sessionId -> its pid ->
// pane_pid join. Fallback: cwd fail-open, applied to every garage session
// sharing that dir (over-notify rather than silently drop).
async function resolveSessionIds({ session_id, cwd }) {
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

export function hookSnippet() {
  return {
    hooks: {
      Notification: [{ hooks: [{ type: "http", url: HOOK_URL }] }],
      Stop: [{ hooks: [{ type: "http", url: HOOK_URL }] }],
    },
    allowedHttpHookUrls: [`http://127.0.0.1:${PORT}/*`],
  };
}

export default async function hookRoutes(app) {
  // Exempt from the Origin allowlist in security.js — hook posts come from
  // the claude CLI, not a browser.
  app.post("/api/hooks/claude", async (req, reply) => {
    const payload = req.body ?? {};
    const state = EVENT_TO_STATE[payload.hook_event_name];
    if (!state) {
      return reply.code(200).send({ ok: true, ignored: true });
    }

    const ids = await resolveSessionIds(payload);
    for (const id of ids) setStatus(id, state);

    return reply.code(200).send({ ok: true, applied: ids });
  });

  app.get("/api/hooks/snippet", async () => hookSnippet());
}
