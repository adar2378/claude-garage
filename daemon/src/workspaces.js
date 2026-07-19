import { stat } from "node:fs/promises";
import { GARAGE_PREFIX, NAME_RE, listSessions, renameSession } from "./tmux.js";
import {
  listWorkspaces,
  upsertWorkspace,
  getWorkspace,
  renameWorkspace,
  removeWorkspace,
} from "./registry.js";

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

  // D-rename: renames everything that embeds the workspace name — the
  // registry key, every live tmux session under it, and their
  // resume-metadata keys. Live-session renames happen first (best-effort,
  // no rollback on partial failure per design) so a mid-flight failure
  // still leaves the registry consistent with whichever sessions actually
  // got renamed.
  app.patch("/api/workspaces/:name", async (req, reply) => {
    const oldName = req.params.name;
    const { name: newName } = req.body ?? {};

    const existing = await getWorkspace(oldName);
    if (!existing) {
      return reply.code(404).send({ error: `unknown workspace: ${oldName}` });
    }
    if (!NAME_RE.test(newName ?? "")) {
      return reply.code(400).send({ error: "name must match [a-z0-9-]+" });
    }
    if (newName !== oldName && (await getWorkspace(newName))) {
      return reply
        .code(409)
        .send({ error: `workspace already registered: ${newName}` });
    }

    const oldPrefix = `${GARAGE_PREFIX}${oldName}/`;
    const newPrefix = `${GARAGE_PREFIX}${newName}/`;
    const live = (await listSessions()).filter((s) => s.id.startsWith(oldPrefix));

    const renamedSessions = [];
    const failedSessions = [];
    for (const s of live) {
      const newId = newPrefix + s.id.slice(oldPrefix.length);
      try {
        await renameSession(s.id, newId);
        renamedSessions.push({ from: s.id, to: newId });
      } catch (err) {
        failedSessions.push({ id: s.id, error: err.message ?? String(err) });
      }
    }

    const workspace = await renameWorkspace(oldName, newName);

    const response = { ...workspace, renamedSessions };
    if (failedSessions.length > 0) {
      response.failedSessions = failedSessions;
    }
    return reply.code(200).send(response);
  });

  // Removing a workspace NEVER touches tmux: the registry is a directory
  // mapping, not a session store. Live sessions keep running and reappear
  // in the rail as an unregistered group; the workspace's resume metadata
  // is dropped so no permanently-unrestorable ghosts linger.
  app.delete("/api/workspaces/:name", async (req, reply) => {
    const name = req.params.name;
    if (!(await getWorkspace(name))) {
      return reply.code(404).send({ error: `unknown workspace: ${name}` });
    }
    await removeWorkspace(name);
    return reply.code(204).send();
  });
}
