import React, { useEffect, useState } from "react";
import { installHooks } from "../lib/api.js";

const DISMISS_KEY = "garage.hooksBannerDismissed";
const WINDOW_MS = 60_000;
const INSTALLED_LINGER_MS = 4_000;

// Simplest proxy available client-side for "no status event of hook origin
// has arrived yet": if every known session is still in a poller-only state
// (working/idle — needs-input/done only ever come from a real transition,
// which in practice means a hook fired) within the first 60s of load, and
// the user hasn't dismissed it before, nudge them to install the hooks.
// Deliberately simple per spec 3.4 — no attempt to track event provenance.
//
// p7 hooks-install:
//  - Guard: never shown while the session list is empty — the old
//    `every()` was vacuously true on an empty pit wall, so the very first
//    thing a brand-new user saw was hook-setup homework.
//  - Primary action is now one-click "install hooks for me" (POST
//    /api/hooks/install — daemon merges with backup + atomic write);
//    the snippet link stays as the secondary, manual path.
export default function HooksBanner({ sessions }) {
  const [dismissed, setDismissed] = useState(() => {
    try {
      return localStorage.getItem(DISMISS_KEY) === "1";
    } catch {
      return false;
    }
  });
  const [withinWindow, setWithinWindow] = useState(true);
  const [install, setInstall] = useState({ state: "idle", error: null, backup: null });

  useEffect(() => {
    const timer = setTimeout(() => setWithinWindow(false), WINDOW_MS);
    return () => clearTimeout(timer);
  }, []);

  // Success lingers briefly so the confirmation is readable, then the
  // banner dismisses itself permanently — hooks are installed, its job is
  // done for good.
  useEffect(() => {
    if (install.state !== "done") return;
    const timer = setTimeout(() => dismiss(), INSTALLED_LINGER_MS);
    return () => clearTimeout(timer);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [install.state]);

  if (dismissed) return null;
  if (install.state !== "done" && !withinWindow) return null;
  if (sessions.length === 0) return null;

  const onlyBaseline = sessions.every((s) => s.status === "working" || s.status === "idle");
  if (install.state === "idle" && !onlyBaseline) return null;

  function dismiss() {
    setDismissed(true);
    try {
      localStorage.setItem(DISMISS_KEY, "1");
    } catch {
      // best effort — a missing localStorage just means it reappears next load
    }
  }

  async function handleInstall() {
    setInstall({ state: "busy", error: null, backup: null });
    try {
      const result = await installHooks();
      setInstall({ state: "done", error: null, backup: result.backup ?? null });
    } catch (err) {
      setInstall({ state: "idle", error: err.message, backup: null });
    }
  }

  if (install.state === "done") {
    return (
      <div className="flex flex-none items-center gap-3 bg-garage-sel px-4 py-2 text-xs text-garage-dim">
        <span className="text-garage-green">
          ✓ hooks installed — needs-input detection is now instant
          {install.backup && (
            <span className="text-garage-dim"> (backup saved next to settings.json)</span>
          )}
        </span>
        <button
          onClick={dismiss}
          className="ml-auto rounded-md px-2 py-1 text-xs text-garage-dim hover:bg-garage-line hover:text-garage-ink"
        >
          dismiss ×
        </button>
      </div>
    );
  }

  return (
    <div className="flex flex-none flex-wrap items-center gap-3 bg-garage-sel px-4 py-2 text-xs text-garage-dim">
      <span>Hooks make needs-input detection instant (without them, status lags the poller).</span>
      <button
        onClick={handleInstall}
        disabled={install.state === "busy"}
        className="rounded-md bg-garage-panel px-3 py-1.5 text-[13px] text-garage-ink hover:bg-garage-line disabled:opacity-40"
      >
        {install.state === "busy" ? "installing…" : "install hooks for me"}
      </button>
      <a
        href="/api/hooks/snippet"
        target="_blank"
        rel="noreferrer"
        className="text-garage-dim underline hover:text-garage-ink"
      >
        view the snippet instead
      </a>
      {install.error && (
        <span className="max-w-[24rem] truncate text-garage-red" title={install.error}>
          {install.error}
        </span>
      )}
      <button
        onClick={dismiss}
        className="ml-auto rounded-md px-2 py-1 text-xs text-garage-dim hover:bg-garage-line hover:text-garage-ink"
      >
        dismiss ×
      </button>
    </div>
  );
}
