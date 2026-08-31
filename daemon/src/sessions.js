import { stat } from "node:fs/promises";
import os from "node:os";
import {
  GARAGE_PREFIX,
  NAME_RE,
  sessionId,
  listSessions,
  listPanePaths,
  resolveBranch,
  normalizeTitle,
  hasSession,
  createSession,
  killSession,
} from "./tmux.js";
import {
  getWorkspace,
  listSessionMetas,
  getSessionMeta,
  upsertSessionMeta,
  removeSessionMeta,
} from "./registry.js";
import { getStatusEntry } from "./status.js";
import { createWorktree } from "./worktrees.js";

const CLAUDE_CMD = process.env.GARAGE_CLAUDE_CMD ?? "claude";

export default async function sessionRoutes(app) {
  app.get("/api/sessions", async () => {
    const sessions = await listSessions();
    const panePaths = await listPanePaths();
    // p10: same tmux hostname for every session in this request — read once.
    const hostname = os.hostname();

    // D-branch: this route is re-hit on every SSE-driven refetch, so the
    // git calls it triggers must stay cheap — one execFile per unique
    // directory per request, not per session. `getBranch` memoizes on
    // that dir for the lifetime of this single request only (a fresh Map
    // per call, no cross-request caching); resolveBranch itself already
    // swallows errors and returns null, so a bad/non-git dir never surfaces
    // here as a rejection.
    const branchCache = new Map();
    const getBranch = (dir) => {
      if (!dir) return Promise.resolve(null);
      if (!branchCache.has(dir)) branchCache.set(dir, resolveBranch(dir));
      return branchCache.get(dir);
    };

    const live = await Promise.all(
      sessions.map(async (s) => {
        // since: epoch ms the current status began, so the UI can render
        // elapsed time without a second lookup (see status.js getStatusEntry).
        const { state: status, since, message } = getStatusEntry(s.id);
        const pane = panePaths.get(s.id);
        return {
          ...s,
          status,
          // p8: the Notification hook's text for a needs-input session; the
          // store guarantees null for every other state (see status.js).
          message: message ?? null,
          // A session that has never transitioned has no recorded `since`
          // (the store only stamps one on a real state change), which would
          // render as "—" in the UI even though the session is live and has
          // an obvious age. Fall back to when tmux created it.
          since: since ?? s.createdAt ?? null,
          // Live pane cwd, not session_path (see listPanePaths) — falls back
          // to session_path only if the pane vanished between the two tmux
          // calls above (a session that died mid-request).
          branch: await getBranch(pane?.path ?? s.dir),
          // p10: the pane's OSC title (e.g. Claude Code's "✳ <summary>"),
          // filtered down to null when it's just tmux's own default (empty,
          // hostname, bare shell name) — see normalizeTitle.
          title: normalizeTitle(pane?.title, hostname),
        };
      })
    );
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
      const dir = registered?.dir ?? null;
      restorable.push({
        id: meta.id,
        workspace: meta.workspace,
        label: meta.label,
        dir,
        attached: false,
        status: "restorable",
        since: null,
        message: null,
        // p10: no live pane to read a title from — restorable entries never
        // have one (see session-status spec).
        title: null,
        restorable: true,
        // No live pane to read a cwd from — resolve against the
        // registered workspace dir instead; null if that's gone too.
        branch: await getBranch(dir),
      });
    }

    return [...live, ...restorable];
  });

  app.post("/api/sessions", async (req, reply) => {
    const { workspace, label, worktree: wantWorktree } = req.body ?? {};

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

    // D-wt-meta: worktree creation happens before any tmux state exists —
    // createWorktree throws (400, with git's stderr) on a non-git repoDir or
    // a failed `git worktree add`, and nothing has been spawned yet to clean
    // up in that case.
    let worktree = null;
    if (wantWorktree) {
      try {
        worktree = await createWorktree({ repoDir: registered.dir, workspace, label });
      } catch (err) {
        return reply.code(err.statusCode ?? 400).send({ error: err.message });
      }
    }

    const spawnDir = worktree?.path ?? registered.dir;
    await createSession(id, spawnDir, CLAUDE_CMD);

    if (worktree) {
      // Written immediately, at spawn — not by the poller. The poller's own
      // upserts (claudeSessionId/workspace/label, once it observes the
      // agent) merge alongside this without clobbering it; see registry.js.
      await upsertSessionMeta(id, {
        workspace,
        label,
        worktree: { ...worktree, repoDir: registered.dir },
      });
    }

    return reply.code(201).send({ id, workspace, label, dir: spawnDir, worktree });
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

        // D-wt-meta: a worktree session restores INTO its worktree, not the
        // registered workspace dir. The stat-gate keeps the metadata
        // (neither the session nor the worktree record is removed on
        // failure) so a later retry — or a manual `finish` with the
        // recorded path/branch — can still recover.
        const spawnDir = meta.worktree?.path ?? registered.dir;
        const dirStat = await stat(spawnDir).catch(() => null);
        if (!dirStat?.isDirectory()) {
          const reason = meta.worktree ? "worktree missing" : "registered dir no longer exists";
          return { failed: { id: meta.id, reason } };
        }

        if (await hasSession(meta.id)) {
          return { failed: { id: meta.id, reason: "session already running" } };
        }

        await createSession(meta.id, spawnDir, CLAUDE_CMD, [
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
          await createSession(meta.id, spawnDir, CLAUDE_CMD);
        }

        return {
          restored: {
            id: meta.id,
            workspace: meta.workspace,
            label: meta.label,
            dir: spawnDir,
            worktree: meta.worktree ?? null,
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

    // p8.1: `?meta=1` deletes ONLY the stored resume metadata of a NON-live
    // (restorable) session — the plain DELETE below 404s for those since
    // there is no tmux session to kill. Behavior without the param is
    // unchanged (the web UI never sends it). The worktree record rides the
    // response the same way, so the caller can still surface "worktree
    // kept" for a discarded restorable worktree session.
    if (req.query?.meta === "1") {
      if (await hasSession(id)) {
        return reply
          .code(409)
          .send({ error: `session is live — delete it without meta=1: ${id}` });
      }
      const meta = await getSessionMeta(id).catch(() => null);
      if (!meta) {
        return reply.code(404).send({ error: `no resume metadata for: ${id}` });
      }
      await removeSessionMeta(id).catch(() => {});
      return reply
        .code(200)
        .send({ deleted: true, meta: true, worktree: meta?.worktree ?? null });
    }

    if (!(await hasSession(id))) {
      return reply.code(404).send({ error: `no such session: ${id}` });
    }

    // D-wt-meta: capture the meta BEFORE removing it — the worktree record
    // (if any) rides the response body so the caller can drive
    // POST /api/worktrees/finish afterward, since by then the session (and
    // its metadata) are already gone.
    const meta = await getSessionMeta(id).catch(() => null);

    await killSession(id);
    // Deliberate kill: the user is done with this conversation, so there's
    // nothing to offer restoring later.
    await removeSessionMeta(id).catch(() => {});
    return reply.code(200).send({ deleted: true, worktree: meta?.worktree ?? null });
  });
}
