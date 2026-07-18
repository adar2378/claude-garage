import React, { useState } from "react";
import SessionTerminal from "../SessionTerminal.jsx";
import { glyphFor, colorFor } from "../lib/status.js";
import { restoreSession } from "../lib/api.js";

// Center pane (spec: 3.2). Renders every session of the focused workspace
// as its own live SessionTerminal, stacked. Cell identity is keyed by
// session id, so switching `group` unmounts the previous workspace's
// cells wholesale — SessionTerminal's existing cleanup (ws.close()) tears
// down its pty, satisfying the "old ptys closed on switch" requirement
// without any extra bookkeeping here.
//
// p3-restore-and-ship: a "restorable" session has no live tmux session
// behind it, so `WS /term/:id` would just fail — this cell renders a
// placeholder (dim, ⟳ + restore control) instead of mounting
// SessionTerminal at all, and only swaps to a live terminal once the rail
// (or this control) restores it and a refetch flips its status.
export default function TerminalGrid({ group, focusedSessionId, onFocusCell, onBlurChrome, onSessionsRestored }) {
  const [restoringIds, setRestoringIds] = useState(() => new Set());
  const [restoreError, setRestoreError] = useState(null);

  async function restoreOne(id) {
    setRestoringIds((prev) => new Set(prev).add(id));
    setRestoreError(null);
    try {
      await restoreSession({ id });
      onSessionsRestored?.();
    } catch (err) {
      setRestoreError(err.message);
    } finally {
      setRestoringIds((prev) => {
        const next = new Set(prev);
        next.delete(id);
        return next;
      });
    }
  }

  if (!group || group.sessions.length === 0) {
    return (
      <div
        onMouseDown={onBlurChrome}
        className="grid min-h-0 place-items-center bg-garage-bg text-xs text-garage-dim"
      >
        {group ? "no sessions in this workspace yet" : "no workspace selected"}
      </div>
    );
  }

  return (
    <div
      onMouseDown={onBlurChrome}
      className="grid min-h-0 auto-rows-fr gap-2 overflow-hidden bg-garage-bg p-2"
    >
      {group.sessions.map((s) => {
        const focused = s.id === focusedSessionId;
        const isRestorable = s.status === "restorable";
        const busy = restoringIds.has(s.id);
        return (
          <div
            key={s.id}
            onMouseDown={(e) => {
              // Contain the click here so it doesn't also trigger the
              // grid-gutter blur handler above.
              e.stopPropagation();
              onFocusCell(s.id);
            }}
            onFocus={() => onFocusCell(s.id)}
            className={`flex min-h-0 flex-col overflow-hidden rounded border bg-garage-panel ${
              focused ? "border-garage-amber" : "border-garage-line"
            } ${isRestorable ? "opacity-70" : ""}`}
          >
            <div className="flex flex-none items-center gap-2 border-b border-garage-line px-2 py-1 text-xs">
              <span className={colorFor(s.status)}>{glyphFor(s.status)}</span>
              <span className={focused ? "font-semibold text-garage-amber" : "text-garage-ink"}>
                {s.label}
              </span>
              <span className="ml-auto text-garage-faint">{s.status}</span>
            </div>
            <div className="min-h-0 flex-1 p-1">
              {isRestorable ? (
                <div className="flex h-full flex-col items-center justify-center gap-2 text-garage-dim">
                  <span className="text-2xl">⟳</span>
                  <span className="text-xs">no live terminal — session needs to be restored</span>
                  <button
                    type="button"
                    onClick={() => restoreOne(s.id)}
                    disabled={busy}
                    className="border border-garage-line px-2 py-0.5 text-xs text-garage-dim hover:border-garage-amber hover:text-garage-amber disabled:opacity-40"
                  >
                    restore
                  </button>
                  {restoreError && (
                    <span className="max-w-[80%] text-center text-[11px] text-garage-red">
                      {restoreError}
                    </span>
                  )}
                </div>
              ) : (
                <SessionTerminal id={s.id} />
              )}
            </div>
          </div>
        );
      })}
    </div>
  );
}
