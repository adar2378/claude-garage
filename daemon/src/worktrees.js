import { execFile } from "node:child_process";
import { promisify } from "node:util";
import { existsSync } from "node:fs";
import { mkdir } from "node:fs/promises";
import { homedir } from "node:os";
import path from "node:path";

const run = promisify(execFile);

// D-wt-location: outside any repo, so a worktree never pollutes `git status`
// for the repo itself. Keyed by workspace so `~/.garage/worktrees/<ws>/` is
// one place to `ls` per workspace.
// Follows the registry's GARAGE_DIR override (testability — see registry.js).
const WORKTREES_ROOT = path.join(
  process.env.GARAGE_DIR ?? path.join(homedir(), ".garage"),
  "worktrees"
);

// D-wt-branch: namespaced so a repo's real branches never collide with ours,
// and so the discard/merge cleanup guard below can refuse to touch anything
// that isn't ours.
const GARAGE_BRANCH_PREFIX = "garage/";

async function git(dir, args) {
  try {
    return await run("git", ["-C", dir, ...args]);
  } catch (err) {
    // execFile's rejection already carries .stderr/.stdout when the process
    // ran and exited non-zero; wrap so callers get a clean .message (git's
    // stderr, trimmed) while still exposing the raw stderr for surfacing to
    // the API caller (D-wt-finish: merge conflicts return git's stderr).
    const wrapped = new Error(err.stderr?.trim() || err.message);
    wrapped.stderr = err.stderr;
    wrapped.cause = err;
    throw wrapped;
  }
}

function assertGarageBranch(branch) {
  if (typeof branch !== "string" || !branch.startsWith(GARAGE_BRANCH_PREFIX)) {
    const err = new Error(`refusing to touch non-garage branch: ${branch}`);
    err.statusCode = 400;
    throw err;
  }
}

async function branchExists(repoDir, branch) {
  try {
    await run("git", ["-C", repoDir, "rev-parse", "--verify", "--quiet", `refs/heads/${branch}`]);
    return true;
  } catch {
    return false;
  }
}

// D-wt-location / D-wt-branch: both the directory and the branch name are
// derived from the same candidate label, suffixed together (-2, -3, ...)
// until a candidate is free on both axes — keeps `garage/<label>` and its
// worktree dir visually paired even after a collision.
async function pickCandidate(repoDir, workspace, label) {
  const baseDir = path.join(WORKTREES_ROOT, workspace);
  let suffix = 1;
  let candidate = label;
  for (;;) {
    const dirPath = path.join(baseDir, candidate);
    const branch = `${GARAGE_BRANCH_PREFIX}${candidate}`;
    const dirTaken = existsSync(dirPath);
    const branchTaken = dirTaken || (await branchExists(repoDir, branch));
    if (!dirTaken && !branchTaken) return { dirPath, branch };
    suffix += 1;
    candidate = `${label}-${suffix}`;
  }
}

// Creates `~/.garage/worktrees/<workspace>/<label>` (suffixed on collision)
// as a new git worktree on a fresh `garage/<label>` branch cut from repoDir's
// current HEAD. Throws a 400 error (with git's stderr as .message) if repoDir
// isn't a git repo or `git worktree add` fails for any reason (dirty/locked
// repo state, etc.) — callers should surface err.statusCode / err.message
// directly, no session or tmux state is created before this succeeds.
export async function createWorktree({ repoDir, workspace, label }) {
  try {
    await run("git", ["-C", repoDir, "rev-parse", "--is-inside-work-tree"]);
  } catch {
    const err = new Error(`not a git repository: ${repoDir}`);
    err.statusCode = 400;
    throw err;
  }

  const { dirPath, branch } = await pickCandidate(repoDir, workspace, label);
  await mkdir(path.dirname(dirPath), { recursive: true });

  try {
    await run("git", ["-C", repoDir, "worktree", "add", "-b", branch, dirPath]);
  } catch (err) {
    const wrapped = new Error(err.stderr?.trim() || err.message);
    wrapped.statusCode = 400;
    throw wrapped;
  }

  return { path: dirPath, branch };
}

// The branch checked out in repoDir (what a merge would land on), or null if
// it can't be read (not a repo, unborn HEAD, ...). Informational only.
export async function currentBranch(repoDir) {
  try {
    const { stdout } = await run("git", ["-C", repoDir, "rev-parse", "--abbrev-ref", "HEAD"]);
    return stdout.trim() || null;
  } catch {
    return null;
  }
}

// D-wt-finish: runs after the session is dead — repoDir/path/branch come
// from the worktree record the caller already holds (DELETE's response
// body), never re-derived. `merge` and `discard` are guarded to garage/*
// branches so this can never touch a repo's real branches even if a caller
// passes a bogus record. `keep` is a deliberate no-op so the UI's three-way
// finish prompt is uniform (dismissing it is equivalent).
export async function finishWorktree({ repoDir, path: wtPath, branch, action }) {
  if (action === "keep") {
    return { action: "keep", path: wtPath, branch };
  }

  if (action !== "merge" && action !== "discard") {
    const err = new Error(`unknown action: ${action}`);
    err.statusCode = 400;
    throw err;
  }

  assertGarageBranch(branch);

  if (action === "merge") {
    // The most common failure isn't a conflict — it's an agent that edited
    // but never committed: the branch has nothing to merge and `worktree
    // remove` would refuse anyway. Catch it up front with a human message.
    const dirty = await git(wtPath, ["status", "--porcelain"]).catch(() => null);
    if (dirty?.stdout && dirty.stdout.trim() !== "") {
      const err = new Error(
        "worktree has uncommitted changes — commit them in the session first, or discard"
      );
      err.statusCode = 409;
      throw err;
    }
    try {
      await git(repoDir, ["merge", "--no-ff", branch]);
    } catch (err) {
      // D-wt-finish: non-zero exit -> 409, worktree and branch kept. Abort
      // first so repoDir isn't left mid-merge (no conflict UI to finish it).
      await git(repoDir, ["merge", "--abort"]).catch(() => {});
      const conflictErr = new Error(
        "merge conflict — merge aborted, nothing changed; resolve it in the session (merge the target into the branch), then try again"
      );
      conflictErr.statusCode = 409;
      conflictErr.stderr = err.stderr;
      throw conflictErr;
    }
    await git(repoDir, ["worktree", "remove", wtPath]);
    await git(repoDir, ["branch", "-d", branch]);
    return { action: "merge", path: wtPath, branch, merged: true };
  }

  // discard
  await git(repoDir, ["worktree", "remove", "--force", wtPath]);
  await git(repoDir, ["branch", "-D", branch]);
  return { action: "discard", path: wtPath, branch, discarded: true };
}

export default async function worktreeRoutes(app) {
  app.post("/api/worktrees/finish", async (req, reply) => {
    const { worktree, action } = req.body ?? {};

    if (!worktree || typeof worktree !== "object") {
      return reply.code(400).send({ error: "worktree record required" });
    }
    const { path: wtPath, branch, repoDir } = worktree;
    if (!wtPath || !branch || !repoDir) {
      return reply
        .code(400)
        .send({ error: "worktree record requires path, branch, and repoDir" });
    }
    if (!["merge", "discard", "keep"].includes(action)) {
      return reply
        .code(400)
        .send({ error: "action must be one of merge, discard, keep" });
    }

    try {
      const result = await finishWorktree({ repoDir, path: wtPath, branch, action });
      return reply.code(200).send(result);
    } catch (err) {
      const body = { error: err.message };
      if (err.stderr) body.stderr = err.stderr;
      return reply.code(err.statusCode ?? 500).send(body);
    }
  });
}
