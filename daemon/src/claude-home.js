import { homedir } from "node:os";
import { join } from "node:path";

// p11: GARAGE_CLAUDE_HOME overrides the Claude Code config root this daemon
// reads/writes (~/.claude by default) — same spirit as GARAGE_DIR overriding
// ~/.garage for the registry (registry.js). Production always uses the real
// ~/.claude; the override exists purely for testability, so hook/statusline
// install tests and transcript fixture tests never touch the user's real
// settings.json or ~/.claude/projects transcripts. Read lazily (not cached
// in a module-level constant) so a caller that sets the env var right before
// a dynamic import — the pattern every test in this repo uses — always sees
// it, and callers that DO want a load-time constant (mirroring GARAGE_DIR)
// simply call this once at their own module's top level.
export function claudeHome() {
  return process.env.GARAGE_CLAUDE_HOME ?? join(homedir(), ".claude");
}
