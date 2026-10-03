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
  respawnPane,
} from "./tmux.js";
import {
  getWorkspace,
  listSessionMetas,
  getSessionMeta,
  upsertSessionMeta,
  removeSessionMeta,
} from "./registry.js";
import { getStatusEntry, getStatus, hasStatus, dropSession as dropStatus } from "./status.js";
import { dropSession as dropStatuslineContext } from "./statusline.js";
import { pollerEvents, pollOnce } from "./poller.js";
import { createWorktree, currentBranch } from "./worktrees.js";
import { getStatuslineContext, getRateLimits } from "./statusline.js";
import { getCachedContext, refreshContext } from "./transcript.js";

const CLAUDE_CMD = process.env.GARAGE_CLAUDE_CMD ?? "claude";

const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

// D-restore-flow / p16-restart D1: shared by restore (spawn === createSession,
// a fresh tmux session) and restart (spawn === respawnPane, the SAME tmux
// session) — both need "start `claude --resume <claudeSessionId>`, then
// confirm it didn't immediately die". `claude --resume` exits (after
// printing "No conversation found") when the conversation was never
// persisted — a session with no submitted exchange — killing the pane/
// session it just started. The exit isn't instant, so confirm twice before
// trusting the resume; if it died, or there was no claudeSessionId to resume
// in the first place (p16-restart out-of-scope note: a fresh session that
// never wrote a transcript), fall back to a plain `claude` and report
// `resumed: false` rather than silently defaulting a required value.
// `spawn` takes the same (id, dir, command, extraArgs) shape createSession
// and respawnPane already share.
// Exported alongside planRestartTargets so the missing-claudeSessionId
// branch (no tmux call at all — it never reaches hasSession) is
// unit-testable with a fake `spawn`, same discipline as the target-
// selection tests below (daemon/test/restart.test.js).
export async function spawnClaudeResumed(spawn, id, dir, claudeSessionId) {
  if (!claudeSessionId) {
    await spawn(id, dir, CLAUDE_CMD);
    return false;
  }

  await spawn(id, dir, CLAUDE_CMD, ["--resume", claudeSessionId]);

  await sleep(1500);
  if (await hasSession(id)) {
    await sleep(2000);
  }
  if (await hasSession(id)) {
    return true;
  }

  await spawn(id, dir, CLAUDE_CMD);
  return false;
}

// p16-restart D2: pure target-selection step for POST /api/sessions/restart
// — no tmux/IO, so it's exhaustively unit-testable without a live tmux
// server (daemon/test/restart.test.js). `sessions` is the already-scoped
// candidate list ({id, status} — either the one requested id or every live
// session for {all:true}); busy sessions (`working`/`needs-input`) are
// skipped unless `force`. p16-restart follow-up: a status of "unknown"
// (the route's stand-in for "the poller has never observed this id" — see
// hasStatus/needsPollBeforePlan below) is treated the same way — never
// silently planned as if it were idle (D6: never default a required
// value). Exported for the test suite.
export function planRestartTargets(sessions, force) {
  const targets = [];
  const skipped = [];
  for (const s of sessions) {
    const busy = s.status === "working" || s.status === "needs-input" || s.status === "unknown";
    if (!force && busy) {
      skipped.push({ id: s.id, status: s.status });
    } else {
      targets.push(s.id);
    }
  }
  return { targets, skipped };
}

// p16-restart follow-up: pure decision — should the restart route await one
// fresh poll (pollOnce) before planning? Yes whenever ANY id in `ids` has no
// status entry yet (`hasStatusFn` is status.js's `hasStatus`, injected so
// this is unit-testable without the real store). This is exactly the
// window right after `claude-garage restart` hands off to a fresh successor
// daemon: its status store is empty, so getStatus(id) would silently read
// "idle" for a session that is actually mid-turn (status.js docs this
// default deliberately) — planning off that default would restart a
// `working` session without `force`.
export function needsPollBeforePlan(ids, hasStatusFn) {
  return ids.some((id) => !hasStatusFn(id));
}

// Worktree record + `target` (the branch repoDir has checked out, i.e. where
// a merge would land) so the finish prompt can name it. Informational only —
// /api/worktrees/finish ignores it.
async function withTarget(worktree) {
  if (!worktree) return null;
  return { ...worktree, target: await currentBranch(worktree.repoDir) };
}

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

    // p11: fetched once up front (not per-session) — same cost discipline as
    // branchCache above — so both the live loop's claudeSessionId lookup and
    // the restorable loop below share one registry read.
    const metas = await listSessionMetas();
    const metaById = new Map(metas.map((m) => [m.id, m]));

    const live = await Promise.all(
      sessions.map(async (s) => {
        // since: epoch ms the current status began, so the UI can render
        // elapsed time without a second lookup (see status.js getStatusEntry).
        const { state: status, since, message } = getStatusEntry(s.id);
        const pane = panePaths.get(s.id);
        const dir = pane?.path ?? s.dir;

        // p11: context — statusline data (pushed by the wrapper) beats the
        // transcript fallback beats null. The transcript path is a cache
        // read only: getCachedContext never blocks, refreshContext kicks a
        // background read (a no-op if the cache is still fresh or a read is
        // already in flight) — this route must never block on file IO.
        let context = getStatuslineContext(s.id);
        if (!context) {
          context = getCachedContext(s.id);
          const claudeSessionId = metaById.get(s.id)?.claudeSessionId;
          if (claudeSessionId && dir) {
            refreshContext(s.id, { dir, claudeSessionId });
          }
        }

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
          branch: await getBranch(dir),
          // p10: the pane's OSC title (e.g. Claude Code's "✳ <summary>"),
          // filtered down to null when it's just tmux's own default (empty,
          // hostname, bare shell name) — see normalizeTitle.
          title: normalizeTitle(pane?.title, hostname),
          // p11: {usedPercentage, source: "statusline"|"transcript"} | null.
          context,
        };
      })
    );
    const liveIds = new Set(live.map((s) => s.id));

    // D-restore-flow: append one entry per resume-metadata record whose id
    // has no matching live tmux session. dir is re-resolved from the
    // registry (not a cached copy in the meta record) so a workspace
    // re-registered to a new path since the crash is reflected correctly.
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
        // p11: restorable entries never carry context — there's no live
        // session to have posted a statusline or grown a fresh transcript
        // usage figure since it died (see context-telemetry spec).
        context: null,
      });
    }

    return [...live, ...restorable];
  });

  // p11: account-wide rate limits from the most recent statusline post —
  // null until one arrives. See statusline.js for the store.
  app.get("/api/usage", async () => getRateLimits());

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

        const resumed = await spawnClaudeResumed(
          createSession,
          meta.id,
          spawnDir,
          meta.claudeSessionId
        );

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

  // p16-restart: restart every LIVE session matching {id}, or every live
  // session for {all:true}, in place — tmux respawn-pane (D1), not
  // kill+restore, so the tmux session identity (and everything keyed off
  // it: the wall tile, dockview panel, title) survives. Restorable
  // (non-live) sessions are never targets — they have no pane to respawn;
  // restore them via /api/sessions/restore instead. `force` restarts a
  // working/needs-input session anyway; without it those land in `skipped`
  // (D2) rather than losing an in-flight turn.
  app.post("/api/sessions/restart", async (req, reply) => {
    const { id, all, force } = req.body ?? {};
    if (!id && !all) {
      return reply.code(400).send({ error: "must provide id or all:true" });
    }

    const liveSessions = await listSessions();
    if (id && !liveSessions.some((s) => s.id === id)) {
      return reply.code(404).send({ error: `no live session: ${id}` });
    }
    const scoped = id ? liveSessions.filter((s) => s.id === id) : liveSessions;

    // p16-restart follow-up: getStatus defaults an unobserved id to "idle"
    // (by design — every other caller wants that safe default), which is
    // wrong here right after a fresh successor daemon boots with an empty
    // status store (claude-garage restart, D3) — every session would look
    // idle and a `working` one would restart without `force`. Await one
    // real poll first whenever that's the situation, so planning below
    // reads real signal instead of the default.
    if (needsPollBeforePlan(scoped.map((s) => s.id), hasStatus)) {
      await pollOnce(app);
    }

    const candidates = scoped.map((s) => ({
      id: s.id,
      // Still unobserved even after the poll (claude CLI missing, pid join
      // failed, ...) — "unknown", never silently defaulted to idle; see
      // planRestartTargets.
      status: hasStatus(s.id) ? getStatus(s.id) : "unknown",
    }));

    const { targets, skipped } = planRestartTargets(candidates, Boolean(force));

    const liveById = new Map(liveSessions.map((s) => [s.id, s]));
    const metas = await listSessionMetas();
    const metaById = new Map(metas.map((m) => [m.id, m]));

    // Targets are independent — restart them concurrently, same discipline
    // as restore-all above. A tmux failure (respawnPane rejects with tmux's
    // stderr) is caught here and reported per-target in `failed`, rather
    // than letting one bad target take down the rest of the batch.
    const results = await Promise.all(
      targets.map(async (targetId) => {
        const live = liveById.get(targetId);
        const meta = metaById.get(targetId);
        try {
          // Same dir resolution restore uses (D-wt-meta): the worktree
          // path when this is a worktree session, else the registered
          // workspace dir — falling back to the live tmux dir only when
          // the session predates any registry meta (a fresh session the
          // poller hasn't observed yet).
          const registered = await getWorkspace(live.workspace);
          const dir = meta?.worktree?.path ?? registered?.dir ?? live.dir;

          const resumed = await spawnClaudeResumed(
            respawnPane,
            targetId,
            dir,
            meta?.claudeSessionId ?? null
          );

          // The pane now runs a fresh claude process (a new pid) — the
          // status the poller had for the OLD process (working/done/
          // needs-input) no longer describes anything. Clear it the same
          // way the poller's own dropSession does for a dead session, so
          // the next tick's pid-join re-derives the real state instead of
          // carrying stale signal forward.
          dropStatus(targetId);
          dropStatuslineContext(targetId);

          return { restarted: { id: targetId, resumed } };
        } catch (err) {
          return { failed: { id: targetId, error: err.message } };
        }
      })
    );

    const restarted = results.filter((r) => r.restarted).map((r) => r.restarted);
    const failed = results.filter((r) => r.failed).map((r) => r.failed);

    // respawn-pane never changes the tmux session-id set the poller diffs
    // on (same session, new process), so the poller's own "sessions-changed"
    // detection never fires for a restart — push it explicitly so the wall
    // refetches promptly instead of waiting on an unrelated event.
    if (restarted.length > 0) pollerEvents.emit("sessions-changed");

    return reply.code(200).send({ restarted, skipped, failed });
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
    // unchanged. The worktree record rides the
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
      return reply.code(200).send({
        deleted: true,
        meta: true,
        worktree: await withTarget(meta?.worktree),
      });
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
    return reply
      .code(200)
      .send({ deleted: true, worktree: await withTarget(meta?.worktree) });
  });
}
