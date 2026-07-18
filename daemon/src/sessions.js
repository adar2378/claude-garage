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

const CLAUDE_CMD = process.env.GARAGE_CLAUDE_CMD ?? "claude";

export default async function sessionRoutes(app) {
  app.get("/api/sessions", async () => listSessions());

  app.post("/api/sessions", async (req, reply) => {
    const { workspace, label, dir } = req.body ?? {};

    if (!NAME_RE.test(workspace ?? "") || !NAME_RE.test(label ?? "")) {
      return reply
        .code(400)
        .send({ error: "workspace and label must match [a-z0-9-]+" });
    }
    const dirStat = await stat(dir ?? "").catch(() => null);
    if (!dirStat?.isDirectory()) {
      return reply.code(400).send({ error: `dir is not a directory: ${dir}` });
    }

    const id = sessionId(workspace, label);
    if (await hasSession(id)) {
      return reply.code(409).send({ error: `session already exists: ${id}` });
    }

    await createSession(id, dir, CLAUDE_CMD);
    return reply.code(201).send({ id, workspace, label, dir });
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
