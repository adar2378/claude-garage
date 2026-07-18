import { stat } from "node:fs/promises";
import {
  GARAGE_PREFIX,
  NAME_RE,
  sessionId,
  listSessions,
  hasSession,
  createSession,
  killSession,
} from "./tmux.js";
import { getWorkspace, listSessionMetas, removeSessionMeta } from "./registry.js";
import { getStatus } from "./status.js";

const CLAUDE_CMD = process.env.GARAGE_CLAUDE_CMD ?? "claude";

export default async function sessionRoutes(app) {
  app.get("/api/sessions", async () => {
    const sessions = await listSessions();
    const live = sessions.map((s) => ({ ...s, status: getStatus(s.id) }));
    const liveIds = new Set(live.map((s) => s.id));

    // D-restore-flow: append one entry per resume-metadata record whose id
    // has no matching live tmux session. dir is re-resolved from the
    // registry (not a cached copy in the meta record) so a workspace
    // re-registered to a new path since the crash is reflected correctly.
    const metas = await listSessionMetas();
    const restorable = [];
    for (const meta of metas) {
      if (liveIds.has(meta.id)) continue;
      const registered = await getWorkspace(meta.workspace);
      restorable.push({
        id: meta.id,
        workspace: meta.workspace,
        label: meta.label,
        dir: registered?.dir ?? null,
        attached: false,
        status: "restorable",
        restorable: true,
      });
    }

    return [...live, ...restorable];
  });

  app.post("/api/sessions", async (req, reply) => {
    const { workspace, label } = req.body ?? {};

    if (!NAME_RE.test(workspace ?? "") || !NAME_RE.test(label ?? "")) {
      return reply
        .code(400)
        .send({ error: "workspace and label must match [a-z0-9-]+" });
    }

    const registered = await getWorkspace(workspace);
    if (!registered) {
      return reply.code(404).send({ error: `unknown workspace: ${workspace}` });
    }

    const dirStat = await stat(registered.dir).catch(() => null);
    if (!dirStat?.isDirectory()) {
      return reply
        .code(400)
        .send({ error: `registered dir no longer exists: ${registered.dir}` });
    }

    const id = sessionId(workspace, label);
    if (await hasSession(id)) {
      return reply.code(409).send({ error: `session already exists: ${id}` });
    }

    await createSession(id, registered.dir, CLAUDE_CMD);
    return reply.code(201).send({ id, workspace, label, dir: registered.dir });
  });

  // D-restore-flow: {id} restores one restorable session; {all:true}
  // restores every session whose metadata has no live tmux match. Each
  // target is independent — one failure (stale workspace, missing dir, name
  // collision) never blocks the rest, and the meta record is retained on
  // failure so a later retry can still succeed.
  app.post("/api/sessions/restore", async (req, reply) => {
    const { id, all } = req.body ?? {};
    if (!id && !all) {
      return reply.code(400).send({ error: "must provide id or all:true" });
    }

    const metas = await listSessionMetas();
    let targets;
    if (all) {
      const liveIds = new Set((await listSessions()).map((s) => s.id));
      targets = metas.filter((m) => !liveIds.has(m.id));
    } else {
      const meta = metas.find((m) => m.id === id);
      if (!meta) {
        return reply.code(404).send({ error: `no resume metadata for: ${id}` });
      }
      targets = [meta];
    }

    const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

    // Targets are independent — restore them concurrently so the
    // died-on-resume confirmation waits below don't serialize restore-all.
    const results = await Promise.all(
      targets.map(async (meta) => {
        const registered = await getWorkspace(meta.workspace);
        if (!registered) {
          return { failed: { id: meta.id, reason: "workspace no longer registered" } };
        }

        const dirStat = await stat(registered.dir).catch(() => null);
        if (!dirStat?.isDirectory()) {
          return { failed: { id: meta.id, reason: "registered dir no longer exists" } };
        }

        if (await hasSession(meta.id)) {
          return { failed: { id: meta.id, reason: "session already running" } };
        }

        await createSession(meta.id, registered.dir, CLAUDE_CMD, [
          "--resume",
          meta.claudeSessionId,
        ]);

        // `claude --resume` exits (after printing "No conversation found")
        // when the conversation was never persisted — a session with no
        // submitted exchange — killing the new tmux session and leaving a
        // restore loop. The exit isn't instant, so confirm twice before
        // trusting the resume; if it died, fall back to a fresh claude.
        let resumed = true;
        await sleep(1500);
        if (await hasSession(meta.id)) {
          await sleep(2000);
        }
        if (!(await hasSession(meta.id))) {
          resumed = false;
          await createSession(meta.id, registered.dir, CLAUDE_CMD);
        }

        return {
          restored: {
            id: meta.id,
            workspace: meta.workspace,
            label: meta.label,
            dir: registered.dir,
            resumed,
          },
        };
      })
    );

    const restored = results.filter((r) => r.restored).map((r) => r.restored);
    const failed = results.filter((r) => r.failed).map((r) => r.failed);

    const status = restored.length > 0 ? (all ? 200 : 201) : 409;
    return reply.code(status).send({ restored, failed });
  });

  // id contains slashes — capture the whole tail as a wildcard.
  app.delete("/api/sessions/*", async (req, reply) => {
    const id = decodeURIComponent(req.params["*"]);

    if (!id.startsWith(GARAGE_PREFIX)) {
      return reply
        .code(403)
        .send({ error: "refusing to touch non-garage sessions" });
    }
    if (!(await hasSession(id))) {
      return reply.code(404).send({ error: `no such session: ${id}` });
    }

    await killSession(id);
    // Deliberate kill: the user is done with this conversation, so there's
    // nothing to offer restoring later.
    await removeSessionMeta(id).catch(() => {});
    return reply.code(204).send();
  });
}
