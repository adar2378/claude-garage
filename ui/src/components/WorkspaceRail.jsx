import React, { useState } from "react";
import { glyphFor, colorFor } from "../lib/status.js";
import { restoreSession, renameWorkspace, deleteWorkspace } from "../lib/api.js";
import AddSessionControl from "./AddSessionControl.jsx";

const INDENT_PX = 14;

// Left pane (spec: 3.1). `groups` is expected to be lib/groups.js's
// flattenedGroups (design D-nesting) — pre-ordered needs-you-first at
// every level, with nested workspaces already walked into render order and
// each entry carrying a `depth` (0 = top-level) this component indents by.
// Index hints (1-9) reflect that same flattened order the "1"-"9"
// keybindings switch to — callers must index into the identical array.
//
// p3-restore-and-ship: restorable sessions (status "restorable", no live
// tmux/pty behind them) render dimmed with a restore control; a
// per-workspace "restore all" control appears when every session in that
// workspace's deck is restorable (design D-restore-flow / spec
// "Restorable sessions in the rail"). Restore-all issues the per-id calls
// in parallel rather than a single {all:true} request, so it only ever
// touches this workspace's own deck.
//
// p4-layout-focus-workspace-ux:
//  - D-rename: the ✎ button on a registered workspace's header row swaps
//    it for an inline text input; Enter calls renameWorkspace(), Esc
//    cancels, 404/409/etc surface inline next to the input. On success the
//    caller is notified via onWorkspaceRenamed so it can refetch
//    workspaces/sessions (a rename changes every live session id under
//    that workspace, so the caller — not this component — owns deciding
//    what else needs to move, e.g. focus).
//  - D-dim (column semantics): each *session* row is a dim zone
//    (`data-dim-zone`), exempt from dimming when that session is
//    needs-input, and `dim-focused` whenever the rail as a whole is the
//    `activeColumn` (`columnActive`, passed down from App) — not tied to
//    which individual session is focused anymore (that's what
//    `focusedSessionId` is still used for: the `bg-garage-sel` "you are
//    here" highlight). The rail container and workspace header rows are
//    NOT zones.
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
  onWorkspaceRenamed,
  columnActive,
  onActivateColumn,
}) {
  const [restoringIds, setRestoringIds] = useState(() => new Set());
  const [restoreError, setRestoreError] = useState(null);
  const [renaming, setRenaming] = useState(null); // { workspaceName, value, error, busy } | null
  // Two-step confirm for workspace removal: first ✕ arms it ("sure?"),
  // second click within 3s deletes. Removal never kills tmux sessions —
  // live ones reappear as an unregistered group.
  const [confirmingDelete, setConfirmingDelete] = useState(null); // workspace name | null

  async function handleDelete(name) {
    if (confirmingDelete !== name) {
      setConfirmingDelete(name);
      setTimeout(() => setConfirmingDelete((c) => (c === name ? null : c)), 3000);
      return;
    }
    setConfirmingDelete(null);
    try {
      await deleteWorkspace(name);
      onWorkspaceRenamed?.(name, null); // same refetch path as rename
    } catch (err) {
      setRestoreError(err.message);
    }
  }

  function startRename(name) {
    setRenaming({ workspaceName: name, value: name, error: null, busy: false });
  }

  function cancelRename() {
    setRenaming(null);
  }

  async function submitRename() {
    if (!renaming || renaming.busy) return;
    const oldName = renaming.workspaceName;
    const newName = renaming.value.trim();
    if (!newName || newName === oldName) {
      setRenaming(null);
      return;
    }
    setRenaming((r) => ({ ...r, busy: true, error: null }));
    try {
      await renameWorkspace(oldName, newName);
      setRenaming(null);
      onWorkspaceRenamed?.(oldName, newName);
    } catch (err) {
      setRenaming((r) => ({ ...r, busy: false, error: err.message }));
    }
  }

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
      onMouseDownCapture={onActivateColumn}
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
        const depth = group.depth ?? 0;
        const isRenamingThis = renaming?.workspaceName === group.name;
        return (
          <div key={group.name} className="mb-1" style={{ marginLeft: depth * INDENT_PX }}>
            <div
              className={`flex items-center gap-1 px-2 py-1 ${
                active ? "text-garage-amber" : "text-garage-ink"
              }`}
            >
              {isRenamingThis ? (
                <form
                  onSubmit={(e) => {
                    e.preventDefault();
                    submitRename();
                  }}
                  className="flex min-w-0 flex-1 items-center gap-2"
                >
                  {i < 9 && <span className="text-garage-faint">{i + 1}</span>}
                  <input
                    autoFocus
                    value={renaming.value}
                    disabled={renaming.busy}
                    onChange={(e) =>
                      setRenaming((r) => (r ? { ...r, value: e.target.value } : r))
                    }
                    onKeyDown={(e) => {
                      e.stopPropagation();
                      if (e.key === "Escape") cancelRename();
                    }}
                    onBlur={cancelRename}
                    className="min-w-0 flex-1 border border-garage-amber bg-garage-bg px-1 py-0.5 text-xs text-garage-ink outline-none"
                  />
                  {renaming.error && (
                    <span
                      className="shrink-0 truncate text-[10px] text-garage-red"
                      title={renaming.error}
                    >
                      {renaming.error}
                    </span>
                  )}
                </form>
              ) : (
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
              )}
              {!isRenamingThis && group.registered && (
                <button
                  type="button"
                  onMouseDown={(e) => e.stopPropagation()}
                  onClick={() => startRename(group.name)}
                  title={`rename ${group.name}`}
                  className="shrink-0 px-1 text-garage-faint hover:text-garage-amber"
                >
                  ✎
                </button>
              )}
              {!isRenamingThis && group.registered && (
                <button
                  type="button"
                  onClick={() => onOpenRoot(group.name)}
                  title={`open ${group.name} root in editor`}
                  className="shrink-0 px-1 text-garage-faint hover:text-garage-amber"
                >
                  ⧉
                </button>
              )}
              {!isRenamingThis && group.registered && (
                <button
                  type="button"
                  onMouseDown={(e) => e.stopPropagation()}
                  onClick={() => handleDelete(group.name)}
                  title={`remove ${group.name} from the registry (sessions keep running)`}
                  className={`shrink-0 px-1 ${
                    confirmingDelete === group.name
                      ? "text-garage-red"
                      : "text-garage-faint hover:text-garage-red"
                  }`}
                >
                  {confirmingDelete === group.name ? "sure?" : "✕"}
                </button>
              )}
              {!isRenamingThis && allRestorable && (
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
              {!isRenamingThis && (
                <AddSessionControl workspaceName={group.name} onCreated={onSessionCreated} />
              )}
            </div>
            <div className="ml-3 border-l border-garage-line pl-2">
              {group.sessions.map((s) => {
                const isFocused = s.id === focusedSessionId;
                const isRestorable = s.status === "restorable";
                const isNeedsInput = s.status === "needs-input";
                const busy = restoringIds.has(s.id);
                return (
                  <div
                    key={s.id}
                    data-dim-zone
                    className={`flex w-full items-start gap-1 px-2 py-1 text-xs ${
                      isFocused ? "bg-garage-sel" : "hover:bg-garage-sel"
                    } ${isRestorable ? "opacity-60" : ""} ${columnActive ? "dim-focused" : ""} ${
                      isNeedsInput ? "dim-exempt" : ""
                    }`}
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
