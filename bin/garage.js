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
import { existsSync, mkdirSync, openSync, readFileSync } from "node:fs";
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

// Binary lookup order (p8-packaging "TUI binary availability"):
// 1. prebuilt binary at tui/dist/garage-tui-<platform>-<arch>
// 2. build once with a local Dart SDK (checkout only — needs tui/ sources)
// 3. actionable error naming exactly what is missing, exit non-zero.
function resolveTuiBinary() {
  const target = `${process.platform}-${process.arch}`;
  const binaryPath = path.join(ROOT, "tui", "dist", `garage-tui-${target}`);
  if (existsSync(binaryPath)) return binaryPath;

  const tuiSrc = path.join(ROOT, "tui", "pubspec.yaml");
  const haveDart = spawnSync("dart", ["--version"], { stdio: "ignore" }).status === 0;
  if (haveDart && existsSync(tuiSrc)) {
    console.log(
      `no prebuilt TUI binary for ${target} — building once with the local Dart SDK ` +
        `(≈30s; lands at tui/dist/garage-tui-${target})`
    );
    const cwd = path.join(ROOT, "tui");
    const pub = spawnSync("dart", ["pub", "get"], { cwd, stdio: "inherit" });
    if (pub.status !== 0) {
      console.error("dart pub get failed — see the output above");
      process.exit(1);
    }
    mkdirSync(path.join(cwd, "dist"), { recursive: true });
    const compile = spawnSync(
      "dart",
      ["compile", "exe", "bin/garage_tui.dart", "-o", `dist/garage-tui-${target}`],
      { cwd, stdio: "inherit" }
    );
    if (compile.status !== 0 || !existsSync(binaryPath)) {
      console.error("dart compile exe failed — see the output above");
      process.exit(1);
    }
    return binaryPath;
  }

  if (!haveDart && existsSync(tuiSrc)) {
    console.error(
      `no prebuilt TUI binary for ${target} and no Dart SDK on PATH.\n` +
        `Install Dart (https://dart.dev/get-dart) and re-run — the TUI builds ` +
        `itself once — or use a platform with a shipped binary (macOS arm64).`
    );
  } else {
    console.error(
      `no prebuilt TUI binary for ${target} in this package, and no tui/ sources ` +
        `to build from.\nUse a platform with a shipped binary (macOS arm64), or run ` +
        `from a git checkout with the Dart SDK installed (npm run build:tui).`
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

if (process.argv[2] === "tui") {
  tuiMain();
} else {
  main();
}
