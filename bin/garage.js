#!/usr/bin/env node
// D-packaging: the `npx claude-garage` entrypoint. Checks prerequisites,
// starts the daemon in-process with GARAGE_SERVE_UI=1 (same-process
// UI+API serving — see daemon/src/index.js), then prints and best-effort
// opens the URL. Never touches tmux on shutdown — SIGINT/SIGTERM close the
// HTTP server only, so live garage sessions survive the process exiting.
import { execFile, spawn } from "node:child_process";
import readline from "node:readline/promises";

const PORT = Number(process.env.GARAGE_PORT ?? 4747);
const HEALTH_URL = `http://127.0.0.1:${PORT}/api/health`;
const UI_URL = `http://127.0.0.1:${PORT}`;

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

async function main() {
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

main();
