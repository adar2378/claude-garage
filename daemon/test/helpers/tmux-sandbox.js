// p20: preloaded into every daemon test process (`node --import`, see the
// `test` script). Points tmux at a private socket inside a private
// TMUX_TMPDIR and drops $TMUX, so no test can reach a developer's live tmux
// server. On exit the private server is killed and its dir removed.
import { execFileSync } from "node:child_process";
import { existsSync, mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import path from "node:path";

// unix socket paths are capped near 104 bytes on macOS, and its default
// tmpdir (/var/folders/...) is already ~50: use /tmp when there is one.
const base = existsSync("/tmp") ? "/tmp" : tmpdir();
const dir = mkdtempSync(path.join(base, "gt-"));
const socket = `gt-${process.pid}`;

delete process.env.TMUX;
delete process.env.TMUX_PANE;
process.env.TMUX_TMPDIR = dir;
process.env.GARAGE_TMUX_SOCKET = socket;

process.on("exit", () => {
  try {
    execFileSync("tmux", ["-L", socket, "kill-server"], {
      stdio: "ignore",
      env: { ...process.env, TMUX: "" , TMUX_TMPDIR: dir },
    });
  } catch {
    // no server was started
  }
  rmSync(dir, { recursive: true, force: true });
});
