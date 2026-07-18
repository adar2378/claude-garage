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
import { getWorkspace } from "./registry.js";
import { getStatus } from "./status.js";

const CLAUDE_CMD = process.env.GARAGE_CLAUDE_CMD ?? "claude";

export default async function sessionRoutes(app) {
  app.get("/api/sessions", async () => {
    const sessions = await listSessions();
    return sessions.map((s) => ({ ...s, status: getStatus(s.id) }));
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
    return reply.code(204).send();
  });
}
