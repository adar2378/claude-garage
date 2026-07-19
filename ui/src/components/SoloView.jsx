import React, { useEffect, useState } from "react";
import SessionTerminal from "../SessionTerminal.jsx";
import { glyphFor, colorFor } from "../lib/status.js";
import { fetchSessions } from "../lib/api.js";
import { registerHeartbeat } from "../lib/popouts.js";
import { useApplyTheme } from "../lib/theme.js";

// design D-popout: rendered by App.jsx instead of the pit wall when the
// page loads with `?solo=<id>` — the URL a pop-out window is opened at
// (see lib/popouts.js#openPopout, triggered from a grid cell's ⇱ button).
// Minimal chrome on purpose: a thin header (id + status glyph) and one
// full-viewport terminal, nothing else from the main grid. This is also
// the heartbeat half of the popout protocol — the main window's
// placeholder cell reclaims once this stops beating (registerHeartbeat's
// cleanup covers both a clean unmount and pagehide/beforeunload).
export default function SoloView({ id }) {
  // null until the one-shot fetch (or the first SSE patch) resolves — no
  // glyph shown meanwhile, which is fine; it's a nicety, not load-bearing.
  const [status, setStatus] = useState(null);

  // p8-theming: popouts follow the main window's theme automatically —
  // settings sync across windows via `storage` events.
  useApplyTheme();

  useEffect(() => registerHeartbeat(id), [id]);

  // One-shot fetch for the initial status (design: "GET /api/sessions
  // polling or SSE — keep simple: fetch once + SSE status patch").
  useEffect(() => {
    let cancelled = false;
    fetchSessions()
      .then((sessions) => {
        if (cancelled) return;
        const mine = sessions.find((s) => s.id === id);
        if (mine) setStatus(mine.status);
      })
      .catch(() => {
        // best-effort — the header glyph is a nicety; the terminal below
        // doesn't depend on this fetch succeeding.
      });
    return () => {
      cancelled = true;
    };
  }, [id]);

  // Live patch after the initial fetch, same "status" event shape App.jsx
  // consumes from the main SSE stream.
  useEffect(() => {
    const es = new EventSource("/api/events");
    es.addEventListener("status", (e) => {
      let payload;
      try {
        payload = JSON.parse(e.data);
      } catch {
        return;
      }
      if (payload.id === id) setStatus(payload.status);
    });
    return () => es.close();
  }, [id]);

  return (
    <main className="flex h-screen flex-col bg-garage-bg font-mono text-sm text-garage-ink">
      <header className="flex flex-none items-center gap-2 border-b border-garage-line bg-garage-panel px-3 py-1.5 text-xs">
        {status && <span className={colorFor(status)}>{glyphFor(status)}</span>}
        <span className="text-garage-ink">{id}</span>
        {status && <span className="ml-auto text-garage-faint">{status}</span>}
      </header>
      <div className="min-h-0 flex-1 p-1">
        <SessionTerminal id={id} />
      </div>
    </main>
  );
}
