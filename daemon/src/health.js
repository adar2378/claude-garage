// p8.2 stale-daemon detection: /api/health reports the daemon's package
// version and pid so the launcher (bin/garage.js) can spot a daemon left
// over from before an upgrade (it keeps serving old code — symptom:
// 404s on routes the new UI/TUI call) and restart it by pid — never by
// process-name matching. Restarting is safe by design: tmux owns the
// sessions and state.json is on disk.
import { readFileSync } from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

// daemon/src/health.js -> repo/package root is two levels up.
const __dirname = path.dirname(fileURLToPath(import.meta.url));

// Read once at daemon start. A daemon that cannot read its own
// package.json still serves health — the launcher treats the missing
// version as stale, which errs toward a (harmless) restart.
let version = null;
try {
  version = JSON.parse(
    readFileSync(path.resolve(__dirname, "..", "..", "package.json"), "utf8")
  ).version ?? null;
} catch {
  // leave version null
}

export function healthPayload() {
  return { status: "ok", version, pid: process.pid };
}
