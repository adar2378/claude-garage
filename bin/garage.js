#!/usr/bin/env node
// D-packaging: the `npx claude-garage` entrypoint. Checks prerequisites,
// starts the daemon in-process with GARAGE_SERVE_UI=1 (same-process
// UI+API serving — see daemon/src/index.js), then prints and best-effort
// opens the URL. Never touches tmux on shutdown — SIGINT/SIGTERM close the
// HTTP server only, so live garage sessions survive the process exiting.
//
// p8-packaging: `claude-garage tui` runs the same prerequisite checks, then
// attaches the compiled TUI to a running daemon — starting one detached
// first (so it outlives the TUI) when /api/health is unreachable. Quitting
// the TUI leaves the daemon and every tmux session running.
import { execFile, spawn, spawnSync } from "node:child_process";
import { accessSync, chmodSync, constants as fsConstants, copyFileSync, existsSync, mkdirSync, openSync, readdirSync, readFileSync, statSync } from "node:fs";
import os from "node:os";
import path from "node:path";
import readline from "node:readline/promises";
import { fileURLToPath } from "node:url";

const PORT = Number(process.env.GARAGE_PORT ?? 4747);
const HEALTH_URL = `http://127.0.0.1:${PORT}/api/health`;
const UI_URL = `http://127.0.0.1:${PORT}`;

// bin/garage.js -> repo/package root is one level up. Resolves correctly
// whether run from a checkout or an installed package.
const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");

// The launcher's own package version — compared against the version the
// daemon reports on /api/health to detect a stale (pre-upgrade) daemon
// still serving old code (p8.2; symptom: 404s on routes the new UI/TUI
// call, e.g. `?meta=1`).
const VERSION = JSON.parse(
  readFileSync(path.join(ROOT, "package.json"), "utf8")
).version;

function execFileP(cmd, args) {
  return new Promise((resolve, reject) => {
    execFile(cmd, args, (err, stdout, stderr) => {
      if (err) reject(err);
      else resolve({ stdout, stderr });
    });
  });
}

// Presence-only checks (D-packaging Open Questions: version-gate later if a
// version-specific bug report ever surfaces). Missing binary produces a
// readable, actionable error rather than letting the daemon fail deeper and
// more cryptically the first time it shells out to a missing binary.
async function requireBinary(bin, versionArgs, installHint) {
  try {
    await execFileP(bin, versionArgs);
  } catch (err) {
    if (err && err.code === "ENOENT") {
      console.error(`${bin} not found — install: ${installHint}`);
      process.exit(1);
    }
    // Any other failure (non-zero exit, odd version-flag behavior) is
    // treated as "present" — we only gate on outright absence.
  }
}

// tmux missing: offer to install it — consent-gated, never silent (the
// README promises garage touches nothing without asking). Only when this
// is an interactive terminal AND Homebrew is present; any other situation
// falls back to the plain instruction. Streams brew's own output so the
// user watches exactly what runs.
async function offerTmuxInstall() {
  if (!process.stdin.isTTY || !process.stdout.isTTY) return false;
  try {
    await execFileP("brew", ["--version"]);
  } catch {
    return false;
  }
  const rl = readline.createInterface({ input: process.stdin, output: process.stdout });
  let answer;
  try {
    answer = (await rl.question("tmux not found. Install it now with Homebrew? [y/N] "))
      .trim()
      .toLowerCase();
  } finally {
    rl.close();
  }
  if (answer !== "y" && answer !== "yes") return false;
  console.log("→ brew install tmux");
  const ok = await new Promise((resolve) => {
    const child = spawn("brew", ["install", "tmux"], { stdio: "inherit" });
    child.on("exit", (code) => resolve(code === 0));
    child.on("error", () => resolve(false));
  });
  if (!ok) return false;
  try {
    await execFileP("tmux", ["-V"]);
    return true;
  } catch {
    return false;
  }
}

// Shared by the web and TUI paths — identical checks, identical errors.
async function checkPrerequisites() {
  try {
    await execFileP("tmux", ["-V"]);
  } catch (err) {
    if (err && err.code === "ENOENT") {
      const installed = await offerTmuxInstall();
      if (!installed) {
        console.error("tmux not found — install: brew install tmux");
        process.exit(1);
      }
    }
    // any other failure (odd version-flag behavior) — treat as present,
    // same discipline as requireBinary below
  }
  await requireBinary(
    "claude",
    ["--version"],
    "see https://docs.claude.com/en/docs/claude-code for install instructions"
  );
}

async function main() {
  await checkPrerequisites();

  // Stale-daemon gate (p8.2), same as the tui path: a healthy daemon that
  // predates this launcher is stopped by its reported pid so the
  // in-process daemon below can bind the freed port and serve current
  // code. A daemon at the launcher's own version is left alone (the
  // import below will then report EADDRINUSE as before).
  const health = await fetchHealth();
  if (daemonIsStale(health)) await stopStaleDaemon(health);

  process.env.GARAGE_SERVE_UI = "1";

  // Relative to this file's URL, not cwd — resolves correctly whether run
  // from a checkout or an installed package (see root package.json `files`).
  const { app } = await import("../daemon/src/index.js");

  const shutdown = async () => {
    // app.close() drains connections — but SSE streams and terminal
    // WebSockets never end on their own, so a polite close hangs forever
    // when a browser tab is open. Race it against a hard deadline: tmux
    // owns everything that matters, so force-exiting loses nothing.
    const deadline = new Promise((r) => setTimeout(r, 1500));
    try {
      await Promise.race([app.close(), deadline]);
    } catch {
      // best-effort — we're exiting regardless
    } finally {
      process.exit(0);
    }
  };
  process.on("SIGINT", shutdown);
  process.on("SIGTERM", shutdown);

  setTimeout(async () => {
    try {
      const res = await fetch(HEALTH_URL);
      if (!res.ok) throw new Error(`health check returned ${res.status}`);

      console.log(`claude-garage pit wall → ${UI_URL}`);

      if (process.platform === "darwin") {
        // Best-effort convenience, same "never throw for a non-critical
        // extra" discipline as notify.js's osascript call — swallow any
        // failure silently.
        execFile("open", [UI_URL], () => {});
      }
    } catch {
      console.error(
        `claude-garage did not come up on port ${PORT} — see the log above ` +
          `(port already in use? set GARAGE_PORT to pick a different one)`
      );
      process.exit(1);
    }
  }, 600);
}

// ---------------------------------------------------------------------------
// `claude-garage tui` (p8-packaging)
// ---------------------------------------------------------------------------

async function healthOk(timeoutMs = 750) {
  return (await fetchHealth(timeoutMs)) !== null;
}

// The parsed /api/health body, or null when no healthy daemon answers.
// A daemon that answers 200 with an unparseable body still counts as
// present ({}), so the version gate below treats it as stale rather than
// racing a second daemon onto an occupied port.
async function fetchHealth(timeoutMs = 750) {
  try {
    const res = await fetch(HEALTH_URL, { signal: AbortSignal.timeout(timeoutMs) });
    if (!res.ok) return null;
    return await res.json().catch(() => ({}));
  } catch {
    return null;
  }
}

// ── Stale-daemon gate (p8.2) ────────────────────────────────────────────
// A daemon left running across an upgrade keeps serving pre-upgrade code
// (observed as `?meta=1` 404s from a daemon older than the launcher). When
// the version /api/health reports differs from the launcher's — or the
// field is missing entirely (pre-upgrade daemons) — stop it by the pid it
// reports (never pkill by name) and let the caller start a fresh one the
// same way it would have with no daemon at all. Restarting is safe by
// design: tmux owns the sessions and state.json is on disk.

function daemonIsStale(health) {
  return health !== null && health.version !== VERSION;
}

async function waitForPortFree(deadlineMs = 10000) {
  const start = Date.now();
  while (Date.now() - start < deadlineMs) {
    if (!(await healthOk(500))) return true;
    await new Promise((r) => setTimeout(r, 250));
  }
  return false;
}

// The pid of the process LISTENING on the daemon port — the fallback for
// pre-upgrade daemons whose /api/health reports no pid field. Port-based
// and exact (never process-name matching); null when it cannot be
// determined unambiguously.
async function pidListeningOnPort() {
  try {
    const { stdout } = await execFileP("lsof", [
      "-nP", `-iTCP:${PORT}`, "-sTCP:LISTEN", "-t",
    ]);
    const pids = [...new Set(stdout.split("\n").map((s) => s.trim()).filter(Boolean))];
    if (pids.length !== 1) return null;
    const pid = Number(pids[0]);
    return Number.isInteger(pid) && pid > 1 ? pid : null;
  } catch {
    return null; // lsof missing or nothing listening
  }
}

async function stopStaleDaemon(health) {
  const label = health.version ? `v${health.version}` : "with no version (pre-upgrade)";
  console.log(
    `garage daemon ${label} is stale (launcher v${VERSION}) — restarting it; ` +
      `sessions are untouched (tmux owns them)`
  );
  // Prefer the pid health reports (current daemons); fall back to the
  // port's listener for pre-upgrade daemons that report none.
  let pid = Number(health.pid);
  if (!Number.isInteger(pid) || pid <= 1) {
    pid = await pidListeningOnPort();
  }
  if (pid === null) {
    console.error(
      `could not determine the stale daemon's pid — stop it yourself ` +
        `(lsof -ti tcp:${PORT} | xargs kill) and re-run`
    );
    process.exit(1);
  }
  try {
    process.kill(pid, "SIGTERM");
  } catch (err) {
    if (!(err && err.code === "ESRCH")) {
      console.error(
        `could not stop the stale daemon (pid ${pid}): ${err.message} — ` +
          `stop it yourself and re-run`
      );
      process.exit(1);
    }
    // ESRCH: already gone — the port check below settles it either way.
  }
  if (!(await waitForPortFree())) {
    console.error(
      `port ${PORT} did not free up after stopping daemon pid ${pid} — ` +
        `something else may be serving /api/health there; stop it ` +
        `(lsof -ti tcp:${PORT}) and re-run`
    );
    process.exit(1);
  }
}

// Start the daemon as a detached child so it outlives the TUI — same daemon
// module the web entrypoint imports in-process, same GARAGE_SERVE_UI=1 flag.
// daemon/src/index.js degrades gracefully when ui/dist is absent (logs and
// serves API-only), so the TUI path never requires a built UI, while the web
// wall keeps working alongside whenever ui/dist exists.
function startDetachedDaemon() {
  const daemonEntry = path.join(ROOT, "daemon", "src", "index.js");
  // daemon.log lives beside the daemon's state: honor GARAGE_DIR (the
  // p8.1 scratch-dir override the daemon itself uses) so a scratch-port
  // launcher run never writes into the real ~/.garage.
  const logDir = process.env.GARAGE_DIR ?? path.join(os.homedir(), ".garage");
  mkdirSync(logDir, { recursive: true });
  const logPath = path.join(logDir, "daemon.log");
  const out = openSync(logPath, "a");
  const child = spawn(process.execPath, [daemonEntry], {
    detached: true,
    stdio: ["ignore", out, out],
    env: { ...process.env, GARAGE_SERVE_UI: "1" },
  });
  child.unref();
  return logPath;
}

async function waitForHealth(deadlineMs = 15000) {
  const start = Date.now();
  while (Date.now() - start < deadlineMs) {
    if (await healthOk(500)) return true;
    await new Promise((r) => setTimeout(r, 250));
  }
  return false;
}

// cargo build --release, then copy the result into wall/dist/<target-name>
// (creating the dist dir if needed) and mark it executable. Shared by the
// "no prebuilt binary yet" path and the p11 staleness-guard rebuild path
// below — one cargo invocation, one copy-into-dist step, used both places.
function cargoBuildTuiBinary(rustBinary) {
  const build = spawnSync("cargo", ["build", "--release"], {
    cwd: path.join(ROOT, "wall"),
    stdio: "inherit",
  });
  const built = path.join(ROOT, "wall", "target", "release", "garage-wall");
  if (build.status !== 0 || !existsSync(built)) {
    console.error("cargo build failed — see the output above");
    process.exit(1);
  }
  mkdirSync(path.dirname(rustBinary), { recursive: true });
  copyFileSync(built, rustBinary);
  chmodSync(rustBinary, 0o755);
}

// The latest mtime (ms) of any file under `dir`, recursively; 0 if the
// directory doesn't exist or is empty. Used by the p11 staleness guard to
// compare wall/src against the built binary — a plain recursive walk since
// wall/src is small and this only runs once per launch.
function newestMtimeUnder(dir) {
  let newest = 0;
  const stack = [dir];
  while (stack.length > 0) {
    const current = stack.pop();
    let entries;
    try {
      entries = readdirSync(current, { withFileTypes: true });
    } catch {
      continue;
    }
    for (const entry of entries) {
      const full = path.join(current, entry.name);
      if (entry.isDirectory()) {
        stack.push(full);
      } else {
        const mtime = statSync(full).mtimeMs;
        if (mtime > newest) newest = mtime;
      }
    }
  }
  return newest;
}

// p11 staleness guard: true when anything under wall/src, or wall/Cargo.toml
// / wall/Cargo.lock, has an mtime newer than the built binary — i.e. the
// checkout has source changes the binary predates (the "invisible picker"
// report's root cause: a stale binary silently missing a fix). Source-file
// mtimes only; never compares against GARAGE_TUI_BIN (that path never calls
// this — see resolveTuiBinary's override branch, which returns first).
function wallSourcesNewerThan(rustBinary) {
  const binaryMtime = statSync(rustBinary).mtimeMs;
  const wallDir = path.join(ROOT, "wall");
  for (const f of ["Cargo.toml", "Cargo.lock"]) {
    const full = path.join(wallDir, f);
    if (existsSync(full) && statSync(full).mtimeMs > binaryMtime) return true;
  }
  return newestMtimeUnder(path.join(wallDir, "src")) > binaryMtime;
}

// Binary lookup order (p9-ratatui-port "TUI binary availability"):
// 0. GARAGE_TUI_BIN — test hook for the e2e harnesses (parity gate): an
//    explicit binary path that wins over everything else. Used only when
//    set and executable; a set-but-unusable value is a hard error (a test
//    hook must never silently fall through to a different binary). Honored
//    verbatim — the p11 staleness guard below never runs for it.
// 1. prebuilt Rust binary at wall/dist/garage-wall-<platform>-<arch> — in a
//    source checkout (wall/Cargo.toml present), p11 additionally checks it
//    isn't stale against wall/src before returning it (see below).
// 2. build once with cargo (checkout only — needs wall/ sources), announced
// 3. actionable error naming what is missing (Rust toolchain first),
//    exit non-zero.
function resolveTuiBinary() {
  const override = process.env.GARAGE_TUI_BIN;
  if (override) {
    try {
      accessSync(override, fsConstants.X_OK);
      return override;
    } catch {
      console.error(
        `GARAGE_TUI_BIN is set but not an executable file: ${override}`
      );
      process.exit(1);
    }
  }

  const target = `${process.platform}-${process.arch}`;
  const distDir = path.join(ROOT, "wall", "dist");
  const rustBinary = path.join(distDir, `garage-wall-${target}`);
  const wallSrc = path.join(ROOT, "wall", "Cargo.toml");

  if (existsSync(rustBinary)) {
    // p11 launcher staleness guard: only in a source checkout (wall/
    // present at all — an installed package ships no wall/ sources, so this
    // never triggers there) AND only when wall/src actually outdates the
    // binary. Mirrors the stale-daemon gate's shape: detect, announce,
    // self-heal — reusing the exact same cargo-build path task 2 above (and
    // "no prebuilt binary" below) already uses.
    if (existsSync(wallSrc) && wallSourcesNewerThan(rustBinary)) {
      const haveCargo = spawnSync("cargo", ["--version"], { stdio: "ignore" }).status === 0;
      if (haveCargo) {
        console.log("wall sources newer than the built binary — rebuilding");
        cargoBuildTuiBinary(rustBinary);
      } else {
        console.error(
          `warning: wall/src is newer than the built TUI binary (${rustBinary}) ` +
            `and no cargo on PATH to rebuild it — it may be stale`
        );
      }
    }
    return rustBinary;
  }

  const haveCargo = spawnSync("cargo", ["--version"], { stdio: "ignore" }).status === 0;
  if (haveCargo && existsSync(wallSrc)) {
    console.log(
      `no prebuilt TUI binary for ${target} — building once with cargo ` +
        `(release build; lands at wall/dist/garage-wall-${target})`
    );
    cargoBuildTuiBinary(rustBinary);
    return rustBinary;
  }

  if (existsSync(wallSrc)) {
    console.error(
      `no prebuilt TUI binary for ${target} and no Rust toolchain (cargo) on PATH.\n` +
        `Install Rust (https://rustup.rs) and re-run — the TUI builds itself once ` +
        `from wall/ — or use a platform with a shipped binary (macOS arm64).`
    );
  } else {
    console.error(
      `no prebuilt TUI binary for ${target} in this package, and no wall/ sources ` +
        `to build from.\nUse a platform with a shipped binary (macOS arm64), or run ` +
        `from a git checkout with the Rust toolchain installed (npm run build:tui).`
    );
  }
  process.exit(1);
}

async function tuiMain() {
  await checkPrerequisites();

  // Resolve (and if needed build) the binary BEFORE starting a daemon: a
  // missing binary should fail fast without a side-effect daemon appearing.
  const tuiBinary = resolveTuiBinary();

  // Stale-daemon gate (p8.2): a healthy daemon that predates this launcher
  // is stopped (by its reported pid) and replaced exactly like the
  // daemon-absent path below.
  const health = await fetchHealth();
  const stale = daemonIsStale(health);
  if (stale) await stopStaleDaemon(health);
  if (health === null || stale) {
    if (!stale) {
      console.log(`no daemon on port ${PORT} — starting one (it outlives the TUI)`);
    }
    const logPath = startDetachedDaemon();
    if (!(await waitForHealth())) {
      console.error(
        `claude-garage daemon did not come up on port ${PORT} — see ${logPath} ` +
          `(port already in use? set GARAGE_PORT to pick a different one)`
      );
      process.exit(1);
    }
  }

  // Foreground, inherited stdio: the TUI owns the terminal until it exits.
  // The daemon (in-process elsewhere or the detached child above) is left
  // running — tmux sessions and the web wall stay live.
  const child = spawn(tuiBinary, [], { stdio: "inherit", env: process.env });
  child.on("error", (err) => {
    console.error(`failed to start TUI binary at ${tuiBinary}: ${err.message}`);
    process.exit(1);
  });
  child.on("exit", (code, signal) => {
    process.exit(signal ? 1 : (code ?? 0));
  });
}

// ---------------------------------------------------------------------------
// `claude-garage restart` (p16-restart)
// ---------------------------------------------------------------------------

const JSON_HEADERS = { "content-type": "application/json" };

// D4: same 15s-cap/250ms-poll shape as waitForHealth above, but also
// requires the answering pid to differ from `previousPid` (undefined counts
// as "no prior daemon", so any live match immediately satisfies it) AND the
// version to match this launcher's own — a daemon that hasn't actually
// swapped yet (still the old process, mid-handoff) must not read as done.
async function waitForHealthChanged(previousPid, deadlineMs = 15000) {
  const start = Date.now();
  while (Date.now() - start < deadlineMs) {
    const health = await fetchHealth(500);
    if (health && health.version === VERSION && health.pid !== previousPid) return health;
    await new Promise((r) => setTimeout(r, 250));
  }
  return null;
}

// D4/D5: `claude-garage restart --sessions` — POSTs /api/sessions/restart
// with `{all: true, force: <--all>}` and prints one line per restarted,
// skipped and failed session, then a summary line.
async function restartSessionsCLI(force) {
  const res = await fetch(`http://127.0.0.1:${PORT}/api/sessions/restart`, {
    method: "POST",
    headers: JSON_HEADERS,
    body: JSON.stringify({ all: true, force }),
  });
  const body = await res.json().catch(() => ({}));
  if (!res.ok) {
    console.error(body.error || `session restart failed (${res.status})`);
    process.exit(1);
  }

  for (const r of body.restarted ?? []) {
    console.log(
      r.resumed
        ? `restarted ${r.id} (resumed)`
        : `restarted ${r.id} (fresh — no conversation to resume)`
    );
  }
  for (const s of body.skipped ?? []) {
    console.log(`skipped ${s.id} — ${s.status} (use --all to include)`);
  }
  for (const f of body.failed ?? []) {
    console.log(`failed ${f.id} — ${f.error}`);
  }

  const restartedN = body.restarted?.length ?? 0;
  const skippedN = body.skipped?.length ?? 0;
  const failedN = body.failed?.length ?? 0;
  console.log(`${restartedN} restarted, ${skippedN} skipped, ${failedN} failed`);
}

// D3/D4: restarts the daemon — via its own self-restart endpoint when one
// is already healthy (so the in-process successor logic is exercised the
// same way the TUI's stale-daemon gate exercises stopStaleDaemon), else
// there's nothing to hand off from, so just start one detached the same way
// `tui` does when no daemon answers. `--sessions` then restarts every live
// Claude Code session in place; `--all` also includes busy ones.
async function restartMain() {
  const args = process.argv.slice(3);
  const sessions = args.includes("--sessions");
  const all = args.includes("--all");

  const before = await fetchHealth();
  if (before) {
    const res = await fetch(`http://127.0.0.1:${PORT}/api/daemon/restart`, { method: "POST" });
    if (!res.ok) {
      console.error(
        `daemon restart request failed (${res.status}) — the running daemon ` +
          `(v${before.version ?? "?"}, pid ${before.pid ?? "?"}) may predate this endpoint; ` +
          `stop it yourself (lsof -ti tcp:${PORT} | xargs kill) and re-run "claude-garage"`
      );
      process.exit(1);
    }
    if (!(await waitForHealthChanged(before.pid))) {
      console.error(
        `garage daemon did not come back up on port ${PORT} within 15s of restarting`
      );
      process.exit(1);
    }
  } else {
    console.log(`no daemon on port ${PORT} — starting one`);
    startDetachedDaemon();
    if (!(await waitForHealth())) {
      console.error(`claude-garage daemon did not come up on port ${PORT}`);
      process.exit(1);
    }
  }

  const after = await fetchHealth();
  console.log(`garage daemon restarted → v${after.version} (pid ${after.pid})`);

  if (sessions) {
    // p16-restart follow-up: the daemon now awaits one fresh poll before
    // planning restart targets whenever any of them has no status entry
    // yet (exactly the case right after this handoff) — say why there's a
    // short pause instead of leaving the user staring at silence.
    console.log("daemon restarted — refreshing session status before restarting sessions");
    await restartSessionsCLI(all);
  }
}

const subcommand = process.argv[2];
if (subcommand === undefined) {
  main();
} else if (subcommand === "tui") {
  tuiMain();
} else if (subcommand === "restart") {
  restartMain();
} else {
  console.error(`unknown subcommand: ${subcommand}`);
  console.error(`usage: claude-garage [tui|restart [--sessions] [--all]]`);
  process.exit(1);
}
