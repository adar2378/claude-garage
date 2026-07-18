// Unified-diff parsing helpers shared by FileDiff/DiffList (pane + review
// mode reuse the same rendering, per design D-diff-render). Pure parsing
// only — no JSX here, matching the lib/*.js convention (groups.js,
// status.js) of plain-JS modules the Vite/esbuild default loader can parse
// without a JSX-aware loader.
import parseDiff from "parse-diff";

// Turns one file's unified-diff text (daemon's per-file `diff` string) into
// a small render-friendly shape: an array of hunks, each with its `@@ ... @@`
// header line and a flat list of {type, content, lineNumber} lines.
export function parseFileDiff(diffText) {
  if (!diffText) return [];
  let files;
  try {
    files = parseDiff(diffText);
  } catch {
    return [];
  }
  const file = files[0];
  if (!file || !file.chunks) return [];

  return file.chunks.map((chunk) => ({
    header: chunk.content,
    lines: chunk.changes.map((change) => ({
      type: change.type, // "add" | "del" | "normal"
      content: change.content,
      lineNumber: lineNumberFor(change),
    })),
  }));
}

function lineNumberFor(change) {
  if (change.type === "add" || change.type === "del") return change.ln;
  return change.ln2 ?? change.ln1;
}

// "Open at the line of its first hunk" (spec: editor-escape UI affordances)
// — the new-file line the first hunk starts at, falling back to the old-file
// start (e.g. a pure deletion), defaulting to 1 when nothing parses.
export function firstHunkLine(diffText) {
  if (!diffText) return 1;
  let files;
  try {
    files = parseDiff(diffText);
  } catch {
    return 1;
  }
  const chunk = files[0]?.chunks?.[0];
  if (!chunk) return 1;
  return chunk.newStart || chunk.oldStart || 1;
}
