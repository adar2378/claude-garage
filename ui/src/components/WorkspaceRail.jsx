import React, { useState } from "react";
import { glyphFor, colorFor } from "../lib/status.js";
import { restoreSession } from "../lib/api.js";
import AddSessionControl from "./AddSessionControl.jsx";

// Left pane (spec: 3.1). Groups are pre-ordered needs-you-first by
// lib/groups.js; this component only renders. Index hints (1-9) reflect
// the same rail order the "1"-"9" keybindings switch to.
//
// p3-restore-and-ship: restorable sessions (status "restorable", no live
// tmux/pty behind them) render dimmed with a restore control; a
// per-workspace "restore all" control appears when every session in that
// workspace's deck is restorable (design D-restore-flow / spec
// "Restorable sessions in the rail"). Restore-all issues the per-id calls
// in parallel rather than a single {all:true} request, so it only ever
// touches this workspace's own deck.
export default function WorkspaceRail({
  groups,
  focusedWorkspace,
  focusedSessionId,
  onSelectWorkspace,
  onSelectSession,
  onSessionCreated,
  onOpenRoot,
  onBlurChrome,
  onSessionsRestored,
}) {
  const [restoringIds, setRestoringIds] = useState(() => new Set());
  const [restoreError, setRestoreError] = useState(null);

  function markRestoring(ids, restoring) {
    setRestoringIds((prev) => {
      const next = new Set(prev);
      for (const id of ids) {
        if (restoring) next.add(id);
        else next.delete(id);
      }
      return next;
    });
  }

  async function restoreOne(id) {
    markRestoring([id], true);
    setRestoreError(null);
    try {
      await restoreSession({ id });
      onSessionsRestored?.();
    } catch (err) {
      setRestoreError(err.message);
    } finally {
      markRestoring([id], false);
    }
  }

  async function restoreAll(ids) {
    markRestoring(ids, true);
    setRestoreError(null);
    const results = await Promise.allSettled(ids.map((id) => restoreSession({ id })));
    const failed = results.filter((r) => r.status === "rejected");
    if (failed.length > 0) {
      setRestoreError(`${failed.length} of ${ids.length} restore(s) failed`);
    }
    onSessionsRestored?.();
    markRestoring(ids, false);
  }

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
        const restorableIds = group.sessions
          .filter((s) => s.status === "restorable")
          .map((s) => s.id);
        const allRestorable =
          group.sessions.length > 0 && restorableIds.length === group.sessions.length;
        const restoreAllBusy = restorableIds.some((id) => restoringIds.has(id));
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
              {group.registered && (
                <button
                  type="button"
                  onClick={() => onOpenRoot(group.name)}
                  title={`open ${group.name} root in editor`}
                  className="shrink-0 px-1 text-garage-faint hover:text-garage-amber"
                >
                  ⧉
                </button>
              )}
              {allRestorable && (
                <button
                  type="button"
                  onClick={() => restoreAll(restorableIds)}
                  disabled={restoreAllBusy}
                  title={`restore all sessions in ${group.name}`}
                  className="shrink-0 px-1 text-[10px] text-garage-dim hover:text-garage-amber disabled:opacity-40"
                >
                  restore all
                </button>
              )}
              <AddSessionControl workspaceName={group.name} onCreated={onSessionCreated} />
            </div>
            <div className="ml-3 border-l border-garage-line pl-2">
              {group.sessions.map((s) => {
                const isFocused = s.id === focusedSessionId;
                const isRestorable = s.status === "restorable";
                const busy = restoringIds.has(s.id);
                return (
                  <div
                    key={s.id}
                    className={`flex w-full items-start gap-1 px-2 py-1 text-xs ${
                      isFocused ? "bg-garage-sel" : "hover:bg-garage-sel"
                    } ${isRestorable ? "opacity-60" : ""}`}
                  >
                    <button
                      type="button"
                      onClick={() => onSelectSession(group.name, s.id)}
                      aria-current={isFocused}
                      className="flex min-w-0 flex-1 items-start gap-2 text-left"
                    >
                      <span className={colorFor(s.status)}>{glyphFor(s.status)}</span>
                      <span className="truncate">{s.label}</span>
                    </button>
                    {isRestorable && (
                      <button
                        type="button"
                        onClick={() => restoreOne(s.id)}
                        disabled={busy}
                        title={`restore ${s.label}`}
                        className="shrink-0 px-1 text-[10px] text-garage-dim hover:text-garage-amber disabled:opacity-40"
                      >
                        restore
                      </button>
                    )}
                  </div>
                );
              })}
              {group.sessions.length === 0 && (
                <p className="px-2 py-1 text-[11px] text-garage-faint">no sessions</p>
              )}
            </div>
          </div>
        );
      })}
      {restoreError && (
        <p className="px-3 py-1 text-[11px] text-garage-red" title={restoreError}>
          {restoreError}
        </p>
      )}
    </nav>
  );
}
