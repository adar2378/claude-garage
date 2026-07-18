import React from "react";
import SessionTerminal from "../SessionTerminal.jsx";
import { glyphFor, colorFor } from "../lib/status.js";

// Center pane (spec: 3.2). Renders every session of the focused workspace
// as its own live SessionTerminal, stacked. Cell identity is keyed by
// session id, so switching `group` unmounts the previous workspace's
// cells wholesale — SessionTerminal's existing cleanup (ws.close()) tears
// down its pty, satisfying the "old ptys closed on switch" requirement
// without any extra bookkeeping here.
export default function TerminalGrid({ group, focusedSessionId, onFocusCell, onBlurChrome }) {
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
            }`}
          >
            <div className="flex flex-none items-center gap-2 border-b border-garage-line px-2 py-1 text-xs">
              <span className={colorFor(s.status)}>{glyphFor(s.status)}</span>
              <span className={focused ? "font-semibold text-garage-amber" : "text-garage-ink"}>
                {s.label}
              </span>
              <span className="ml-auto text-garage-faint">{s.status}</span>
            </div>
            <div className="min-h-0 flex-1 p-1">
              <SessionTerminal id={s.id} />
            </div>
          </div>
        );
      })}
    </div>
  );
}
