import { stat } from "node:fs/promises";
import { NAME_RE } from "./tmux.js";
import { listWorkspaces, upsertWorkspace } from "./registry.js";

export default async function workspaceRoutes(app) {
  app.get("/api/workspaces", async () => listWorkspaces());

  app.put("/api/workspaces", async (req, reply) => {
    const { name, dir } = req.body ?? {};

    if (!NAME_RE.test(name ?? "")) {
      return reply.code(400).send({ error: "name must match [a-z0-9-]+" });
    }
    const dirStat = await stat(dir ?? "").catch(() => null);
    if (!dirStat?.isDirectory()) {
      return reply.code(400).send({ error: `dir is not a directory: ${dir}` });
    }

    const workspace = await upsertWorkspace(name, dir);
    return reply.code(200).send(workspace);
  });
}
