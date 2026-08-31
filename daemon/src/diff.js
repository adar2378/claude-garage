import { execFile } from "node:child_process";
import { promisify } from "node:util";
import { readFile, stat } from "node:fs/promises";
import { join } from "node:path";
import { getWorkspace, getSessionMeta } from "./registry.js";
import { listPanePaths, resolveBranch } from "./tmux.js";

// D-diff-cmd (openspec/changes/p2-diff-review/design.md): every git
// invocation here is read-only — status/diff/rev-parse only, never `add -N`
// or anything else that would touch the index. Verified by the gate-1.3
// "status identical before/after repeated calls" check.
const run = promisify(execFile);

// Diffs can legitimately be larger than Node's 1MB default execFile
// maxBuffer; we apply our own byte budgets below, so let git hand back
// whatever it has and truncate ourselves rather than erroring on big output.
const MAX_BUFFER = 64 * 1024 * 1024;

const FILE_BUDGET = 30 * 1024; // ~30KB per file, cut at a hunk boundary when possible
const TOTAL_BUDGET = 300 * 1024; // ~300KB total diff bytes attached to one response

const BINARY_RE = /^Binary files .* differ$/m;

async function git(dir, args) {
  return run("git", ["-C", dir, ...args], { maxBuffer: MAX_BUFFER });
}

// Parses `git status --porcelain=v2 -z` output. -z NUL-terminates each
// record and, for type "2" (rename/copy) records, also uses a NUL (instead
// of the usual tab) to separate the new path from the original path — so a
// rename record consumes an extra token from the split.
function parseStatus(raw) {
  const tokens = raw.split("\0").filter((t) => t.length > 0);
  const entries = [];
  let i = 0;
  while (i < tokens.length) {
    const token = tokens[i];
    const type = token[0];
    if (type === "1") {
      const parts = token.split(" ");
      entries.push({ kind: "ordinary", xy: parts[1], path: parts.slice(8).join(" ") });
      i += 1;
    } else if (type === "2") {
      const parts = token.split(" ");
      const path = parts.slice(9).join(" ");
      const origPath = tokens[i + 1];
      entries.push({ kind: "rename", xy: parts[1], path, origPath });
      i += 2;
    } else if (type === "u") {
      const parts = token.split(" ");
      entries.push({ kind: "ordinary", xy: "MM", path: parts.slice(10).join(" ") });
      i += 1;
    } else if (type === "?") {
      entries.push({ kind: "untracked", path: token.slice(2) });
      i += 1;
    } else {
      // "!" (ignored) or anything unrecognized — nothing to report.
      i += 1;
    }
  }
  return entries;
}

function changeTypeFor(entry) {
  if (entry.kind === "untracked") return "untracked";
  if (entry.kind === "rename") return "renamed";
  const [x, y] = entry.xy;
  if (x === "A" || y === "A") return "added";
  if (x === "D" || y === "D") return "deleted";
  return "modified";
}

// Splits one `git diff` blob into per-file chunks keyed by the file's
// current ("b/") path. Note: git's own -M similarity detection inside this
// single bulk diff can occasionally disagree with `git status`'s rename
// pairing (e.g. a rename with a big enough content change can show up here
// as a plain delete+add instead of one "rename from/rename to" chunk) — in
// that case this map still has a chunk keyed by the new path (the "add"
// half), just without the old-path pairing in its text. Known, accepted gap
// per design's Risks section on rename-detection nuance.
function splitDiffByFile(diffText) {
  const map = new Map();
  if (!diffText) return map;
  const pieces = diffText.split(/(?=^diff --git )/m).filter(Boolean);
  for (const chunk of pieces) {
    const nl = chunk.indexOf("\n");
    const headerLine = nl === -1 ? chunk : chunk.slice(0, nl);
    const m = headerLine.match(/^diff --git a\/(.*) b\/(.*)$/);
    if (m) map.set(m[2], chunk.replace(/\n$/, ""));
  }
  return map;
}

function countStats(diffText) {
  let additions = 0;
  let deletions = 0;
  for (const line of diffText.split("\n")) {
    if (line.startsWith("+++ ") || line.startsWith("--- ")) continue;
    if (line.startsWith("+")) additions++;
    else if (line.startsWith("-")) deletions++;
  }
  return { additions, deletions };
}

function splitHeaderAndHunks(diffText) {
  const lines = diffText.split("\n");
  const firstHunk = lines.findIndex((l) => l.startsWith("@@ "));
  if (firstHunk === -1) return { header: diffText, hunks: [] };
  const header = lines.slice(0, firstHunk).join("\n") + "\n";
  const hunks = [];
  let start = firstHunk;
  for (let idx = firstHunk + 1; idx <= lines.length; idx++) {
    if (idx === lines.length || lines[idx].startsWith("@@ ")) {
      hunks.push(lines.slice(start, idx).join("\n") + "\n");
      start = idx;
    }
  }
  return { header, hunks };
}

function hardCut(text, budget) {
  return Buffer.from(text, "utf8").subarray(0, budget).toString("utf8");
}

// Caps a single file's diff text at FILE_BUDGET, cutting at a hunk boundary
// when one exists so we never emit a half-hunk; falls back to a hard byte
// cut only when no hunk boundary helps (e.g. one gigantic hunk).
function truncateToBudget(diffText, budget) {
  if (Buffer.byteLength(diffText, "utf8") <= budget) {
    return { diff: diffText, truncated: false };
  }
  const { header, hunks } = splitHeaderAndHunks(diffText);
  if (hunks.length === 0) {
    return { diff: hardCut(diffText, budget), truncated: true };
  }
  let acc = header;
  for (const hunk of hunks) {
    const next = acc + hunk;
    if (Buffer.byteLength(next, "utf8") > budget) {
      if (acc === header) return { diff: hardCut(next, budget), truncated: true };
      return { diff: acc, truncated: true };
    }
    acc = next;
  }
  return { diff: acc, truncated: true };
}

async function countLines(absPath) {
  try {
    const content = await readFile(absPath, "utf8");
    if (content.length === 0) return 0;
    return content.split("\n").length - (content.endsWith("\n") ? 1 : 0);
  } catch {
    return 0;
  }
}

// Untracked (or, in a headless repo, "everything") content comes from a
// read-only `git diff --no-index -- /dev/null <path>` — deliberately not
// `git add -N`, which would write an index entry (a mutation, however
// reversible). --no-index exits 1 when there IS a difference (the normal
// case here); that's not an error, it's the expected signal.
async function noIndexDiff(dir, relPath) {
  try {
    const { stdout } = await git(dir, ["diff", "--no-color", "--no-index", "--", "/dev/null", relPath]);
    return stdout;
  } catch (err) {
    if (typeof err.stdout === "string") return err.stdout;
    throw err;
  }
}

// D-wt-diff: `?sessionId=` overrides the diff root when the named session's
// live pane cwd (or, failing that, its worktree meta) sits outside the
// registry dir. Live pane cwd is preferred — it reflects wherever the user's
// shell/claude actually is right now, same source as the sessions list's
// branch chip — falling back to the recorded worktree path only if the pane
// is gone (session died between kill and finish, or was already dead when
// this request landed). Returns registeredDir unchanged if no override
// applies (no sessionId, unknown session, override path vanished, or the
// resolved path IS the registry dir).
// Returns { dir, isWorktree }: isWorktree marks that the override target
// is a worktree session (has worktree metadata), which switches the diff
// base to the fork point (see the route) so committed-but-unmerged branch
// work still shows.
async function resolveDiffRoot(registeredDir, sessionId) {
  if (!sessionId) return { dir: registeredDir, isWorktree: false };

  const meta = await getSessionMeta(sessionId).catch(() => null);
  const wtPath = meta?.worktree?.path;

  const panePaths = await listPanePaths();
  const paneCwd = panePaths.get(sessionId)?.path;
  if (paneCwd && paneCwd !== registeredDir) {
    const s = await stat(paneCwd).catch(() => null);
    if (s?.isDirectory()) return { dir: paneCwd, isWorktree: !!wtPath };
  }

  if (wtPath && wtPath !== registeredDir) {
    const s = await stat(wtPath).catch(() => null);
    if (s?.isDirectory()) return { dir: wtPath, isWorktree: true };
  }

  return { dir: registeredDir, isWorktree: false };
}

// `git diff --name-status -z` records: STATUS\0PATH\0, renames/copies:
// Rnnn\0OLD\0NEW\0. Emitted in parseStatus's entry shape so the same
// changeTypeFor/rendering path applies.
function parseNameStatusZ(raw) {
  const tokens = raw.split("\0").filter((t) => t.length > 0);
  const entries = [];
  let i = 0;
  while (i < tokens.length) {
    const status = tokens[i];
    if (status.startsWith("R") || status.startsWith("C")) {
      entries.push({ kind: "rename", xy: "R.", path: tokens[i + 2], origPath: tokens[i + 1] });
      i += 3;
    } else {
      entries.push({ kind: "ordinary", xy: status[0], path: tokens[i + 1] });
      i += 2;
    }
  }
  return entries;
}

export default async function diffRoutes(app) {
  app.get("/api/diff/:workspace", async (req, reply) => {
    const { workspace } = req.params;
    const { sessionId } = req.query ?? {};
    const registered = await getWorkspace(workspace);
    if (!registered) {
      return reply.code(404).send({ error: `unknown workspace: ${workspace}` });
    }
    const { dir, isWorktree } = await resolveDiffRoot(registered.dir, sessionId);
    const root = dir;

    // One gate covers both "not a git repo" and "registered dir no longer
    // exists" — both mean the same observable thing: nothing to diff.
    try {
      await git(dir, ["rev-parse", "--is-inside-work-tree"]);
    } catch {
      return { files: [], truncated: false, root, branch: null };
    }

    const branch = await resolveBranch(dir);

    let statusRaw;
    try {
      ({ stdout: statusRaw } = await git(dir, ["status", "--porcelain=v2", "-z"]));
    } catch {
      return { files: [], truncated: false, root, branch };
    }

    const statusEntries = parseStatus(statusRaw);

    // Repos with zero commits have no HEAD to diff against — every
    // status-reported path is treated the same as an untracked file.
    let hasHead = true;
    try {
      await git(dir, ["rev-parse", "HEAD"]);
    } catch {
      hasHead = false;
    }

    // Fork-point diff for worktree sessions: "what changed" means
    // everything the branch would bring relative to where it forked from
    // the source repo — committed AND uncommitted — because that is the
    // delta the finish (merge/discard) decision acts on. Base = merge-base
    // of the worktree HEAD and the source repo's current HEAD. Tracked
    // entries are re-derived against that base (git status alone goes
    // blank the moment the agent commits); untracked files still come from
    // status (diff never lists them). Any failure falls back to the plain
    // uncommitted-only behavior.
    let baseRef = "HEAD";
    let entries = statusEntries;
    if (isWorktree && hasHead) {
      try {
        const { stdout: repoHead } = await git(registered.dir, ["rev-parse", "HEAD"]);
        const { stdout: mergeBase } = await git(dir, ["merge-base", "HEAD", repoHead.trim()]);
        if (mergeBase.trim()) baseRef = mergeBase.trim();
      } catch {
        // fork point unknown (repo dir gone, unrelated histories) — keep HEAD
      }
      if (baseRef !== "HEAD") {
        try {
          const { stdout } = await git(dir, [
            "diff",
            "-M",
            "--name-status",
            "-z",
            baseRef,
            "--",
            ".",
          ]);
          entries = [
            ...parseNameStatusZ(stdout),
            ...statusEntries.filter((e) => e.kind === "untracked"),
          ];
        } catch {
          entries = statusEntries;
          baseRef = "HEAD";
        }
      }
    }

    if (entries.length === 0) return { files: [], truncated: false, root, branch };

    let diffMap = new Map();
    if (hasHead) {
      const hasTracked = entries.some((e) => e.kind !== "untracked");
      if (hasTracked) {
        try {
          const { stdout } = await git(dir, ["diff", "-M", "--no-color", baseRef, "--", "."]);
          diffMap = splitDiffByFile(stdout);
        } catch {
          diffMap = new Map();
        }
      }
    }

    const files = [];
    let totalBytes = 0;
    let truncated = false;

    for (const entry of entries) {
      const relPath = entry.path;
      const oldPath = entry.kind === "rename" ? entry.origPath : undefined;
      const changeType = hasHead
        ? changeTypeFor(entry)
        : entry.kind === "untracked"
          ? "untracked"
          : "added";

      if (entry.kind === "untracked" || !hasHead) {
        if (totalBytes >= TOTAL_BUDGET) {
          const additions = await countLines(join(dir, relPath));
          files.push({
            path: relPath,
            oldPath,
            changeType,
            additions,
            deletions: 0,
            binary: false,
            diff: "",
            truncated: true,
          });
          truncated = true;
          continue;
        }

        let raw;
        try {
          raw = await noIndexDiff(dir, relPath);
        } catch {
          // Real failure (permission denied, etc.) — degrade to stat-only
          // rather than failing the whole request.
          files.push({
            path: relPath,
            oldPath,
            changeType,
            additions: 0,
            deletions: 0,
            binary: false,
            diff: "",
            truncated: false,
          });
          continue;
        }

        if (BINARY_RE.test(raw)) {
          files.push({
            path: relPath,
            oldPath,
            changeType,
            additions: 0,
            deletions: 0,
            binary: true,
            diff: "",
            truncated: false,
          });
          continue;
        }

        const { additions, deletions } = countStats(raw);
        const { diff, truncated: fileTruncated } = truncateToBudget(raw, FILE_BUDGET);
        totalBytes += Buffer.byteLength(diff, "utf8");
        if (fileTruncated) truncated = true;
        files.push({
          path: relPath,
          oldPath,
          changeType,
          additions,
          deletions,
          binary: false,
          diff,
          truncated: fileTruncated,
        });
        continue;
      }

      // Tracked (ordinary or rename) with a HEAD to diff against.
      const raw = diffMap.get(relPath);
      if (!raw) {
        files.push({
          path: relPath,
          oldPath,
          changeType,
          additions: 0,
          deletions: 0,
          binary: false,
          diff: "",
          truncated: false,
        });
        continue;
      }

      if (BINARY_RE.test(raw)) {
        files.push({
          path: relPath,
          oldPath,
          changeType,
          additions: 0,
          deletions: 0,
          binary: true,
          diff: "",
          truncated: false,
        });
        continue;
      }

      const { additions, deletions } = countStats(raw);
      if (totalBytes >= TOTAL_BUDGET) {
        files.push({
          path: relPath,
          oldPath,
          changeType,
          additions,
          deletions,
          binary: false,
          diff: "",
          truncated: true,
        });
        truncated = true;
        continue;
      }

      const { diff, truncated: fileTruncated } = truncateToBudget(raw, FILE_BUDGET);
      totalBytes += Buffer.byteLength(diff, "utf8");
      if (fileTruncated) truncated = true;
      files.push({
        path: relPath,
        oldPath,
        changeType,
        additions,
        deletions,
        binary: false,
        diff,
        truncated: fileTruncated,
      });
    }

    return { files, truncated, root, branch };
  });
}
