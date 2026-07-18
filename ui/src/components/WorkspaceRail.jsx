import React from "react";
import { glyphFor, colorFor } from "../lib/status.js";
import AddSessionControl from "./AddSessionControl.jsx";

// Left pane (spec: 3.1). Groups are pre-ordered needs-you-first by
// lib/groups.js; this component only renders. Index hints (1-9) reflect
// the same rail order the "1"-"9" keybindings switch to.
export default function WorkspaceRail({
  groups,
  focusedWorkspace,
  focusedSessionId,
  onSelectWorkspace,
  onSelectSession,
  onSessionCreated,
  onBlurChrome,
}) {
  return (
    <nav
      onMouseDown={onBlurChrome}
      aria-label="Workspaces"
      className="min-h-0 overflow-y-auto border-r border-garage-line bg-garage-panel px-1 py-2"
    >
      {groups.length === 0 && (
        <p className="px-3 py-2 text-xs text-garage-dim">no workspaces yet</p>
      )}
      {groups.map((group, i) => {
        const active = group.name === focusedWorkspace;
        return (
          <div key={group.name} className="mb-1">
            <div
              className={`flex items-center gap-1 px-2 py-1 ${
                active ? "text-garage-amber" : "text-garage-ink"
              }`}
            >
              <button
                type="button"
                onClick={() => onSelectWorkspace(group.name)}
                className="flex min-w-0 flex-1 items-center gap-2 text-left"
              >
                {i < 9 && <span className="text-garage-faint">{i + 1}</span>}
                <span className="truncate font-semibold">{group.name}</span>
                {!group.registered && (
                  <span
                    className="text-[10px] text-garage-faint"
                    title="workspace not registered — derived from session id"
                  >
                    ?
                  </span>
                )}
                <span className="ml-auto shrink-0 text-[10px] text-garage-faint">
                  {group.sessions.length}×
                </span>
              </button>
              <AddSessionControl workspaceName={group.name} onCreated={onSessionCreated} />
            </div>
            <div className="ml-3 border-l border-garage-line pl-2">
              {group.sessions.map((s) => {
                const isFocused = s.id === focusedSessionId;
                return (
                  <button
                    key={s.id}
                    type="button"
                    onClick={() => onSelectSession(group.name, s.id)}
                    aria-current={isFocused}
                    className={`flex w-full items-start gap-2 px-2 py-1 text-left text-xs ${
                      isFocused ? "bg-garage-sel" : "hover:bg-garage-sel"
                    }`}
                  >
                    <span className={colorFor(s.status)}>{glyphFor(s.status)}</span>
                    <span className="truncate">{s.label}</span>
                  </button>
                );
              })}
              {group.sessions.length === 0 && (
                <p className="px-2 py-1 text-[11px] text-garage-faint">no sessions</p>
              )}
            </div>
          </div>
        );
      })}
    </nav>
  );
}
