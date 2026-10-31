// p17 — finishWorktree merge safety + currentBranch helper, against scratch
// git repos under os.tmpdir(). Never touches ~/.garage or real worktrees.
import { test, before, after } from "node:test";
import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import { existsSync } from "node:fs";
import { mkdir, mkdtemp, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import path from "node:path";

let finishWorktree, currentBranch;
let scratch;
let repo;
let wt;

const git = (dir, ...args) =>
  execFileSync("git", ["-C", dir, ...args], { encoding: "utf8" });

async function commitFile(dir, name, content, msg) {
  await writeFile(path.join(dir, name), content);
  git(dir, "add", name);
  git(dir, "commit", "-q", "-m", msg);
}

before(async () => {
  scratch = await mkdtemp(path.join(tmpdir(), "garage-finish-"));
  repo = path.join(scratch, "repo");
  // The finish guard only accepts paths under <GARAGE_DIR>/worktrees/.
  process.env.GARAGE_DIR = path.join(scratch, "garage");
  ({ finishWorktree, currentBranch } = await import("../src/worktrees.js"));
  wt = path.join(scratch, "garage", "worktrees", "ws", "wt");
  await mkdir(path.dirname(wt), { recursive: true });
  execFileSync("git", ["init", "-q", "-b", "main", repo]);
  git(repo, "config", "user.email", "t@example.com");
  git(repo, "config", "user.name", "t");
  await commitFile(repo, "f.txt", "base\n", "base");
  git(repo, "worktree", "add", "-q", "-b", "garage/x", wt);
  git(wt, "config", "user.email", "t@example.com");
  git(wt, "config", "user.name", "t");
});

after(async () => {
  await rm(scratch, { recursive: true, force: true });
});

test("currentBranch returns the checked-out branch, null for a non-repo", async () => {
  assert.equal(await currentBranch(repo), "main");
  assert.equal(await currentBranch(scratch), null);
});

test("dirty worktree refuses merge with 409", async () => {
  await writeFile(path.join(wt, "untracked.txt"), "x\n");
  await assert.rejects(
    finishWorktree({ repoDir: repo, path: wt, branch: "garage/x", action: "merge" }),
    (err) => err.statusCode === 409 && /uncommitted/.test(err.message)
  );
  assert.ok(!existsSync(path.join(repo, ".git", "MERGE_HEAD")));
  await rm(path.join(wt, "untracked.txt"));
});

test("merge conflict -> 409 with stderr, merge aborted, worktree and branch kept", async () => {
  await commitFile(wt, "f.txt", "from branch\n", "branch change");
  await commitFile(repo, "f.txt", "from main\n", "main change");

  await assert.rejects(
    finishWorktree({ repoDir: repo, path: wt, branch: "garage/x", action: "merge" }),
    (err) => err.statusCode === 409 && typeof err.stderr === "string"
  );

  assert.ok(!existsSync(path.join(repo, ".git", "MERGE_HEAD")), "no merge in progress");
  assert.equal(git(repo, "status", "--porcelain").trim(), "");
  assert.ok(existsSync(wt));
  assert.match(git(repo, "branch", "--list", "garage/x"), /garage\/x/);
});

test("discard refuses a path outside the garage worktrees folder, deleting nothing", async () => {
  const outside = path.join(scratch, "outside");
  git(repo, "worktree", "add", "-q", "-b", "garage/outside", outside);
  await assert.rejects(
    finishWorktree({ repoDir: repo, path: outside, branch: "garage/outside", action: "discard" }),
    (err) => err.statusCode === 400
  );
  assert.ok(existsSync(outside));
  assert.match(git(repo, "branch", "--list", "garage/outside"), /garage\/outside/);
});

test("discard refuses a worktree not on the recorded branch, deleting nothing", async () => {
  await assert.rejects(
    finishWorktree({ repoDir: repo, path: wt, branch: "garage/other", action: "discard" }),
    (err) => err.statusCode === 400
  );
  assert.ok(existsSync(wt));
});

test("discard removes a worktree under the garage folder on its branch", async () => {
  const res = await finishWorktree({ repoDir: repo, path: wt, branch: "garage/x", action: "discard" });
  assert.equal(res.discarded, true);
  assert.ok(!existsSync(wt));
});
