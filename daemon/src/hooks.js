import { join } from "node:path";
import { setStatus } from "./status.js";
import { getHookToken } from "./registry.js";
import { tokenMatches } from "./token-auth.js";
import { resolveSessionIds } from "./session-resolve.js";
import { readSettingsOrRefuse, writeSettingsAtomic } from "./settings-install.js";
import { claudeHome } from "./claude-home.js";

const PORT = Number(process.env.GARAGE_PORT ?? 4747);
const HOOK_URL = `http://127.0.0.1:${PORT}/api/hooks/claude`;

const EVENT_TO_STATE = {
  Notification: "needs-input",
  Stop: "done",
};

// Claude Code's Notification hook fires for real blockers (permission
// prompts, questions, plan approvals) AND for a ~60s idle reminder
// ("Claude is waiting for your input") after a turn ends. The reminder is
// idle semantics — waiting for you to ASK, not to ANSWER — and mapping it
// to needs-input raises a sticky false alarm (needs-input never decays by
// design). Filter it by message; unknown messages stay needs-input
// (fail toward attention, never away from it).
const IDLE_REMINDER_RE = /waiting for your input/i;

// Exported for the test suite.
export function isIdleReminder(message) {
  return IDLE_REMINDER_RE.test(message ?? "");
}

export async function hookSnippet() {
  const token = await getHookToken();
  const url = `${HOOK_URL}?token=${token}`;
  return {
    hooks: {
      Notification: [{ hooks: [{ type: "http", url }] }],
      Stop: [{ hooks: [{ type: "http", url }] }],
    },
    allowedHttpHookUrls: [`http://127.0.0.1:${PORT}/*`],
  };
}

// p7 hooks-install (design D-hooks-install): merge the snippet into
// ~/.claude/settings.json server-side so the UI can offer one-click
// installation instead of asking users to hand-merge JSON.
//
// Safety order: parse-or-refuse (a corrupt file is returned as an error,
// byte-for-byte untouched) -> timestamped backup of the pre-install file
// -> atomic write (tmp + rename, same directory, so a crash mid-write can
// never leave a half-written settings.json) — see settings-install.js.
// Idempotency comes from entry-level dedupe: a hook group is only appended
// when no existing group for that event already carries one of its hook
// URLs (the URL embeds the per-install token, which is stable — see
// getHookToken).
const SETTINGS_PATH = join(claudeHome(), "settings.json");

function groupHasAnyUrl(group, urls) {
  return (group?.hooks ?? []).some((h) => urls.has(h.url));
}

export function mergeHookSnippet(settings, snippet) {
  const merged = { ...settings };
  let changed = false;

  merged.hooks = { ...(merged.hooks ?? {}) };
  for (const [event, snippetGroups] of Object.entries(snippet.hooks)) {
    const existing = [...(merged.hooks[event] ?? [])];
    for (const group of snippetGroups) {
      const urls = new Set((group.hooks ?? []).map((h) => h.url));
      if (!existing.some((g) => groupHasAnyUrl(g, urls))) {
        existing.push(group);
        changed = true;
      }
    }
    merged.hooks[event] = existing;
  }

  const allowed = new Set(merged.allowedHttpHookUrls ?? []);
  for (const url of snippet.allowedHttpHookUrls ?? []) {
    if (!allowed.has(url)) {
      allowed.add(url);
      changed = true;
    }
  }
  merged.allowedHttpHookUrls = [...allowed];

  return { merged, changed };
}

async function installHooks() {
  const { settings, raw } = await readSettingsOrRefuse(SETTINGS_PATH);

  const snippet = await hookSnippet();
  const { merged, changed } = mergeHookSnippet(settings, snippet);
  if (!changed) {
    return { ok: true, installed: true, alreadyInstalled: true, backup: null };
  }

  const backup = await writeSettingsAtomic(SETTINGS_PATH, raw, merged);
  return { ok: true, installed: true, alreadyInstalled: false, backup };
}

export default async function hookRoutes(app) {
  // Auth: per-install token in the URL (claude's HTTP hooks are URL-only
  // config, so the token rides as a query param). Combined with the normal
  // Origin allowlist — no exemption — a foreign browser page is stopped by
  // Origin, and anything else without the token is stopped here.
  app.post("/api/hooks/claude", async (req, reply) => {
    const expected = await getHookToken();
    if (!tokenMatches(req.query?.token, expected)) {
      return reply.code(401).send({ error: "missing or invalid hook token" });
    }

    const payload = req.body ?? {};
    const state = EVENT_TO_STATE[payload.hook_event_name];
    if (!state) {
      return reply.code(200).send({ ok: true, ignored: true });
    }

    if (payload.hook_event_name === "Notification" && isIdleReminder(payload.message)) {
      // Idle reminder — leave the state alone (Stop already set done,
      // which decays to idle on its own).
      return reply.code(200).send({ ok: true, ignored: "idle-reminder" });
    }

    const ids = await resolveSessionIds(payload);
    // p8 message capture: a Notification's text rides into the status store
    // alongside needs-input; other events pass no message (setStatus clears
    // it on any transition away from needs-input).
    const message = state === "needs-input" ? payload.message ?? null : undefined;
    for (const id of ids) setStatus(id, state, message);

    return reply.code(200).send({ ok: true, applied: ids });
  });

  app.get("/api/hooks/snippet", async () => hookSnippet());

  // p7 hooks-install: called by the TUI's `I` key (rides the normal Origin
  // allowlist, like every /api route except /api/hooks/claude's token scheme).
  app.post("/api/hooks/install", async (req, reply) => {
    try {
      return await installHooks();
    } catch (err) {
      return reply
        .code(err.statusCode ?? 500)
        .send({ error: err.message ?? "hook install failed" });
    }
  });
}
