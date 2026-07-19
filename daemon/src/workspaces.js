import { stat } from "node:fs/promises";
import {
  GARAGE_PREFIX,
  NAME_RE,
  listSessions,
  renameSession,
  resolveBranch,
  killSession,
} from "./tmux.js";
import {
  listWorkspaces,
  upsertWorkspace,
  getWorkspace,
  renameWorkspace,
  removeWorkspace,
} from "./registry.js";

export default async function workspaceRoutes(app) {
  // D-branch: same helper as sessions.js's per-session branch (tmux.js's
  // resolveBranch), keyed off the registered dir instead of a live pane
  // cwd — a registered workspace has no pane of its own. This route isn't
  // hit on every SSE tick the way GET /api/sessions is, so no per-request
  // dedup here; a plain per-workspace resolve is cheap enough at this
  // scale (one execFile pair per registered workspace).
  app.get("/api/workspaces", async () => {
    const workspaces = await listWorkspaces();
    return Promise.all(
      workspaces.map(async (w) => ({ ...w, branch: await resolveBranch(w.dir) }))
    );
  });

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

  // Removing a workspace by default NEVER touches tmux: the registry is a
  // directory mapping, not a session store. Live sessions keep running and
  // reappear in the rail as an unregistered group; the workspace's resume
  // metadata is dropped (removeWorkspace handles that) so no
  // permanently-unrestorable ghosts linger.
  //
  // p7 follow-up: `?sessions=kill` opts into shutting the workspace down
  // for real — every live garage/<name>/* tmux session is killed first,
  // then the registry entry (and, via removeWorkspace, the resume
  // metadata) is removed. Per-session kill failures are reported without
  // aborting the rest. Worktree directories/branches are deliberately left
  // untouched — bulk removal must never silently discard branch work; they
  // remain recoverable via plain git.
  app.delete("/api/workspaces/:name", async (req, reply) => {
    const name = req.params.name;
    if (!(await getWorkspace(name))) {
      return reply.code(404).send({ error: `unknown workspace: ${name}` });
    }

    const killMode = req.query?.sessions === "kill";
    if (!killMode) {
      await removeWorkspace(name);
      return reply.code(204).send();
    }

    const prefix = `${GARAGE_PREFIX}${name}/`;
    const live = (await listSessions()).filter((s) => s.id.startsWith(prefix));
    const killedSessions = [];
    const failedSessions = [];
    for (const s of live) {
      try {
        await killSession(s.id);
        killedSessions.push(s.id);
      } catch (err) {
        failedSessions.push({ id: s.id, error: err.message ?? String(err) });
      }
    }

    await removeWorkspace(name);

    const response = { removed: name, killedSessions };
    if (failedSessions.length > 0) response.failedSessions = failedSessions;
    return reply.code(200).send(response);
  });
}
