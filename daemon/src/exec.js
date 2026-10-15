import { execFile } from "node:child_process";
import { promisify } from "node:util";

const execFileP = promisify(execFile);

// The daemon is detached from any console on Windows: without windowsHide
// every child process (git, ...) flashes its own console window.
export const run = (cmd, args, opts = {}) => execFileP(cmd, args, { windowsHide: true, ...opts });
