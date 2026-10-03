// p17 — finishWorktree merge safety + currentBranch helper, against scratch
// git repos under os.tmpdir(). Never touches ~/.garage or real worktrees.
import { test, before, after } from "node:test";
import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import { existsSync } from "node:fs";
import { mkdtemp, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import path from "node:path";
import { finishWorktree, currentBranch } from "../src/worktrees.js";

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
  wt = path.join(scratch, "wt");
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
