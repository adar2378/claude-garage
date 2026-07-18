import { spawn } from "node:child_process";
import path from "node:path";
import { getWorkspace } from "./registry.js";

// D-editor (openspec/changes/p2-diff-review/design.md): mirrors the
// GARAGE_CLAUDE_CMD override pattern already used by sessions.js.
const EDITOR_CMD = process.env.GARAGE_EDITOR_CMD ?? "code";

// Resolves once the child has actually spawned (so a caller can trust a
// resolved promise means the CLI was found), rejects with the spawn error
// (notably ENOENT when the editor CLI isn't on PATH) otherwise. The child is
// detached and its stdio ignored — it's a GUI app we fire-and-forget, not a
// process whose output or lifetime the daemon should own.
function spawnEditor(args) {
  return new Promise((resolve, reject) => {
    const child = spawn(EDITOR_CMD, args, { stdio: "ignore", detached: true });
    child.once("error", reject);
    child.once("spawn", () => {
      child.unref();
      resolve();
    });
  });
}

export default async function editorRoutes(app) {
  app.post("/api/open-editor", async (req, reply) => {
    const { workspace, file, line } = req.body ?? {};

    const registered = await getWorkspace(workspace);
    if (!registered) {
      return reply.code(404).send({ error: `unknown workspace: ${workspace}` });
    }
    const dir = registered.dir;

    let args;
    if (file !== undefined && file !== null && file !== "") {
      if (typeof file !== "string") {
        return reply.code(400).send({ error: "file must be a string" });
      }
      // Prefix-check the RESOLVED path, not the raw string, so both
      // "../"-style traversal and absolute-path overrides of `file` are
      // caught regardless of how they're spelled.
      const abs = path.resolve(dir, file);
      if (abs !== dir && !abs.startsWith(dir + path.sep)) {
        return reply.code(400).send({ error: "file escapes workspace directory" });
      }
      args = ["--goto", `${abs}:${line ?? 1}`];
    } else {
      args = [dir];
    }

    try {
      await spawnEditor(args);
    } catch (err) {
      if (err.code === "ENOENT") {
        return reply.code(501).send({ error: `editor CLI not found (${EDITOR_CMD})` });
      }
      throw err;
    }

    return reply.code(200).send({ ok: true });
  });
}
