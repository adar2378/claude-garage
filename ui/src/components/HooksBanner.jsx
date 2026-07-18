import React, { useEffect, useState } from "react";

const DISMISS_KEY = "garage.hooksBannerDismissed";
const WINDOW_MS = 60_000;

// Simplest proxy available client-side for "no status event of hook origin
// has arrived yet": if every known session is still in a poller-only state
// (working/idle — needs-input/done only ever come from a real transition,
// which in practice means a hook fired) within the first 60s of load, and
// the user hasn't dismissed it before, nudge them to install the hooks.
// Deliberately simple per spec 3.4 — no attempt to track event provenance.
export default function HooksBanner({ sessions }) {
  const [dismissed, setDismissed] = useState(() => {
    try {
      return localStorage.getItem(DISMISS_KEY) === "1";
    } catch {
      return false;
    }
  });
  const [withinWindow, setWithinWindow] = useState(true);

  useEffect(() => {
    const timer = setTimeout(() => setWithinWindow(false), WINDOW_MS);
    return () => clearTimeout(timer);
  }, []);

  if (dismissed || !withinWindow) return null;

  const onlyBaseline = sessions.every((s) => s.status === "working" || s.status === "idle");
  if (!onlyBaseline) return null;

  function dismiss() {
    setDismissed(true);
    try {
      localStorage.setItem(DISMISS_KEY, "1");
    } catch {
      // best effort — a missing localStorage just means it reappears next load
    }
  }

  return (
    <div className="flex flex-none items-center gap-3 border-b border-garage-line bg-garage-panel px-4 py-1 text-xs text-garage-amber">
      <span>
        Install Claude Code hooks for precise needs-input detection —{" "}
        <a
          href="/api/hooks/snippet"
          target="_blank"
          rel="noreferrer"
          className="underline hover:text-garage-ink"
        >
          get the settings.json snippet
        </a>
      </span>
      <button onClick={dismiss} className="ml-auto text-garage-dim hover:text-garage-ink">
        dismiss ×
      </button>
    </div>
  );
}
