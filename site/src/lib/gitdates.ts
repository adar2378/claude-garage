import { execFileSync } from 'node:child_process';
import { existsSync, readFileSync } from 'node:fs';
import { resolve } from 'node:path';

// Dates come from git history. Pages that are copied from the repo by scripts/sync-docs.mjs are untracked here,
// so the sync script records the dates of their source files in src/generated/doc-dates.json instead.
const root = process.cwd();
const cache = new Map<string, string>();

function git(args: string[]): string {
  const key = args.join('\0');
  if (cache.has(key)) return cache.get(key)!;
  let out = '';
  try {
    out = execFileSync('git', args, { cwd: root, encoding: 'utf8', stdio: ['ignore', 'pipe', 'ignore'] }).trim();
  } catch {
    out = '';
  }
  cache.set(key, out);
  return out;
}

const head = () => git(['log', '-1', '--format=%cI']);

let synced: Record<string, { published: string; modified: string }> | undefined;
function syncedDates() {
  if (!synced) {
    const f = resolve(root, 'src/generated/doc-dates.json');
    synced = existsSync(f) ? JSON.parse(readFileSync(f, 'utf8')) : {};
  }
  return synced!;
}

/** Dates (ISO 8601 with offset) for a file path relative to site/. */
export function fileDates(rel: string): { published: string; modified: string } {
  rel = rel.replace(/^\.\//, '');
  const s = syncedDates()[rel];
  if (s) return s;
  const modified = git(['log', '-1', '--format=%cI', '--', rel]);
  // %cI of the oldest commit that touches the file
  const all = git(['log', '--follow', '--format=%cI', '--', rel]);
  const published = all ? all.split('\n').pop()! : '';
  const fallback = head();
  return { published: published || modified || fallback, modified: modified || fallback };
}

/** The latest modified date among several files. */
export function latest(rels: string[]): string {
  const ds = rels.map((r) => fileDates(r).modified);
  return ds.sort((a, b) => new Date(a).getTime() - new Date(b).getTime()).pop() ?? head();
}
