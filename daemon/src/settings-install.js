import { copyFile, mkdir, readFile, rename, writeFile } from "node:fs/promises";
import { dirname } from "node:path";

// Shared by hooks.js and statusline.js — both merge a garage-owned fragment
// into ~/.claude/settings.json with the same safety order: parse-or-refuse
// (a corrupt file is reported as a 422, byte-for-byte untouched) -> caller
// merges its own fragment -> timestamped backup of the pre-write file ->
// atomic write (tmp + rename, same directory, so a crash mid-write can never
// leave a half-written settings.json).

// Read + parse settings.json. Returns `{settings, raw}`: `settings` is `{}`
// for a missing/empty file (nothing to refuse), the parsed object otherwise;
// `raw` is the exact original file text, or null if the file didn't exist
// (used by writeSettingsAtomic to decide whether a backup is needed).
// Throws a statusCode:422 error — file left completely untouched — for
// invalid JSON or a non-object top level.
export async function readSettingsOrRefuse(path) {
  let raw = null;
  try {
    raw = await readFile(path, "utf8");
  } catch (err) {
    if (err.code !== "ENOENT") throw err;
  }

  let settings = {};
  if (raw !== null && raw.trim() !== "") {
    try {
      settings = JSON.parse(raw);
    } catch {
      const error = new Error(
        `${path} is not valid JSON — fix it (or remove it) and retry; the file was left untouched`
      );
      error.statusCode = 422;
      throw error;
    }
    if (typeof settings !== "object" || settings === null || Array.isArray(settings)) {
      const error = new Error(`${path} is not a JSON object — the file was left untouched`);
      error.statusCode = 422;
      throw error;
    }
  }
  return { settings, raw };
}

// Backs up (if `raw` is non-null, i.e. the file previously existed) then
// atomically writes `merged`. Returns the backup path, or null when there
// was nothing to back up.
export async function writeSettingsAtomic(path, raw, merged) {
  await mkdir(dirname(path), { recursive: true });

  let backup = null;
  if (raw !== null) {
    backup = `${path}.garage-backup-${new Date().toISOString().replace(/[:.]/g, "-")}`;
    await copyFile(path, backup);
  }

  const tmp = `${path}.garage-tmp-${process.pid}`;
  await writeFile(tmp, `${JSON.stringify(merged, null, 2)}\n`, "utf8");
  await rename(tmp, path);

  return backup;
}
