import React, { createContext, useCallback, useContext, useEffect, useMemo, useRef, useState } from "react";
import { DockviewReact } from "dockview-react";
import { themeAbyss } from "dockview";
import "dockview/dist/styles/dockview.css";
import "../dockview-overrides.css";
import SessionTerminal from "../SessionTerminal.jsx";
import { glyphFor, colorFor } from "../lib/status.js";
import { restoreSession } from "../lib/api.js";
import { loadOrBuildLayout, reconcile, resetLayout, saveLayout } from "../lib/layout.js";

const SAVE_DEBOUNCE_MS = 300;

// Threads live per-session state down into dockview's panel components.
// Dockview (react) mounts panel content through React portals owned by
// DockviewReact's own render tree (see dockview-react's
// ReactPanelContentPart) — that tree stays inside this component's
// subtree, so a normal Context crosses the portal boundary exactly like it
// would for any other child. That's what lets a panel's content react to
// session-state changes (status, restore-in-flight, popped-out) without
// ever calling `panel.api.updateParameters()`: each panel's own identity
// (the only thing dockview needs to persist) is just its session id —
// everything else is read fresh from context on every render.
const GridContext = createContext(null);

// Center pane (spec: 3.2, extended by p4-layout-focus-workspace-ux's
// flexible-layout capability). One dockview panel per session in the
// focused workspace, keyed by session id (design D-dock) — restorable
// sessions keep the restore placeholder as their panel's content, and
// sessions currently viewed in a pop-out window (design D-popout) render a
// reclaim placeholder instead of a live terminal, but every session still
// occupies a slot in the layout so the drag-dock arrangement never shifts
// out from under the user just because a cell's *content* changed.
export default function TerminalGrid({
  group,
  focusedSessionId,
  onFocusCell,
  onBlurChrome,
  onSessionsRestored,
  poppedOutIds,
  onPopOut,
  onReclaim,
  columnActive,
  onActivateColumn,
}) {
  const [restoringIds, setRestoringIds] = useState(() => new Set());
  const [restoreError, setRestoreError] = useState(null);

  // dockview state lives in refs, not React state: the DockviewApi is an
  // imperative handle (mutating it doesn't need a re-render), and
  // `workspaceRef` just remembers which workspace's layout is currently
  // loaded so the effect below can tell "workspace switched" (full
  // reload) apart from "same workspace's session set changed" (reconcile
  // only — see that effect for why the distinction matters).
  const apiRef = useRef(null);
  const workspaceRef = useRef(null);
  const saveTimerRef = useRef(null);

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

  const sessionsById = useMemo(() => {
    const map = new Map();
    for (const s of group?.sessions ?? []) map.set(s.id, s);
    return map;
  }, [group]);

  // Reclaim = clear the popout heartbeat record *and* focus the cell, same
  // as clicking any other cell (design: "the main grid's cell renders that
  // session as a live terminal again").
  const reclaim = useCallback(
    (id) => {
      onReclaim?.(id);
      onFocusCell(id);
    },
    [onReclaim, onFocusCell]
  );

  const contextValue = useMemo(
    () => ({
      sessionsById,
      focusedSessionId,
      restoringIds,
      restoreError,
      poppedOutIds,
      columnActive,
      onFocusCell,
      onPopOut,
      onReclaim: reclaim,
      restoreOne,
    }),
    // restoreOne is intentionally omitted from deps and re-created fresh
    // every render — it only closes over restoringIds/restoreError, both
    // already listed, so it's never stale; adding it here would just make
    // this memo pointless (a new identity every render regardless).
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [
      sessionsById,
      focusedSessionId,
      restoringIds,
      restoreError,
      poppedOutIds,
      columnActive,
      onFocusCell,
      onPopOut,
      reclaim,
    ]
  );

  const sessionIds = useMemo(() => (group?.sessions ?? []).map((s) => s.id), [group]);
  const sessionIdsKey = sessionIds.join(",");

  const scheduleSave = useCallback(() => {
    if (!workspaceRef.current || !apiRef.current) return;
    clearTimeout(saveTimerRef.current);
    saveTimerRef.current = setTimeout(() => {
      if (apiRef.current && workspaceRef.current) saveLayout(workspaceRef.current, apiRef.current);
    }, SAVE_DEBOUNCE_MS);
  }, []);

  // Fires once, the first time this DockviewReact instance mounts (it does
  // not remount on every workspace switch — see the empty-group early
  // return below for the one case it does). `group` is guaranteed
  // non-null/non-empty here by that same early return, so this is exactly
  // "on focused-workspace switch, load that workspace's layout" for
  // whichever workspace was focused at mount time; the effect right below
  // takes over for every change after that.
  function handleReady(event) {
    const api = event.api;
    apiRef.current = api;
    workspaceRef.current = group.name;
    loadOrBuildLayout(api, group.name, sessionIds);
    if (focusedSessionId) {
      const panel = api.getPanel(focusedSessionId);
      if (panel) panel.api.setActive();
    }

    api.onDidLayoutChange(scheduleSave);
    // `origin` distinguishes a user click from our own programmatic
    // `setActive()` calls (the focusedSessionId-sync effect below) — only
    // react to the former, or the two directions of the two-way binding
    // would fight in a feedback loop.
    api.onDidActivePanelChange((e) => {
      if (e.origin === "user" && e.panel) onFocusCell(e.panel.id);
    });
  }

  // Workspace switch -> full reload of that workspace's persisted layout
  // (or the default stack). Same workspace, session set changed (spawn/
  // end/restore-completion) -> reconcile only, so an in-progress custom
  // arrangement is never discarded just because a session came or went —
  // it's already being persisted continuously via onDidLayoutChange above.
  useEffect(() => {
    const api = apiRef.current;
    if (!api || !group) return;
    if (workspaceRef.current !== group.name) {
      workspaceRef.current = group.name;
      loadOrBuildLayout(api, group.name, sessionIds);
    } else {
      reconcile(api, sessionIds);
    }
    if (focusedSessionId) {
      const panel = api.getPanel(focusedSessionId);
      if (panel && !panel.api.isActive) panel.api.setActive();
    }
    // sessionIds itself is intentionally not a dep — sessionIdsKey is its
    // stable stand-in (same session set => same key => effect skipped,
    // avoiding an identity-only re-run from the useMemo above).
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [group?.name, sessionIdsKey]);

  // The other half of the two-way focus binding: App-driven focus changes
  // (rail click, [ ]/a keybindings) follow through into dockview's own
  // active-panel state.
  useEffect(() => {
    const api = apiRef.current;
    if (!api || !focusedSessionId) return;
    const panel = api.getPanel(focusedSessionId);
    if (panel && !panel.api.isActive) panel.api.setActive();
  }, [focusedSessionId]);

  useEffect(() => () => clearTimeout(saveTimerRef.current), []);

  const handleResetLayout = useCallback(() => {
    const api = apiRef.current;
    if (!api || !group) return;
    resetLayout(api, group.name, sessionIds);
  }, [group, sessionIds]);

  if (!group || group.sessions.length === 0) {
    return (
      <div
        onMouseDown={onBlurChrome}
        onMouseDownCapture={onActivateColumn}
        className="grid min-h-0 place-items-center bg-garage-bg text-xs text-garage-dim"
      >
        {group ? "no sessions in this workspace yet" : "no workspace selected"}
      </div>
    );
  }

  return (
    <div
      onMouseDown={onBlurChrome}
      onMouseDownCapture={onActivateColumn}
      className="flex min-h-0 flex-col overflow-hidden bg-garage-bg"
    >
      <div className="flex flex-none items-center gap-2 border-b border-garage-line bg-garage-panel px-2 py-1 text-xs">
        <span className="text-garage-dim">{group.name}</span>
        <button
          type="button"
          onClick={handleResetLayout}
          title="Discard the custom layout and restore the default stack"
          className="ml-auto border border-garage-line px-2 py-0.5 text-[11px] text-garage-dim hover:border-garage-amber hover:text-garage-amber"
        >
          reset layout
        </button>
      </div>
      <div className="min-h-0 flex-1 p-2">
        <GridContext.Provider value={contextValue}>
          <DockviewReact
            className="garage-dock dockview-theme-abyss"
            theme={themeAbyss}
            components={{ terminal: SessionCellPanel }}
            onReady={handleReady}
          />
        </GridContext.Provider>
      </div>
    </div>
  );
}

// A dockview panel's content. `props.api.id` is the dockview panel id,
// which is always a session id (see layout.js — every addPanel call uses
// the session id as the panel id). Everything else needed to render the
// cell — the session record, focus state, restore/popout state — comes
// from GridContext, not from dockview's own params (see that context's
// definition for why).
//
// D-dim (column semantics): every cell is a dim zone (`data-dim-zone`),
// exempt when its session is needs-input, and `dim-focused` whenever the
// grid as a whole is the `activeColumn` (`ctx.columnActive`) — not tied to
// which individual cell is focused (`focused` below still drives the
// amber border / "you are here" styling, just not dimming).
function SessionCellPanel({ api }) {
  const ctx = useContext(GridContext);
  const id = api.id;
  const s = ctx.sessionsById.get(id);

  if (!s) {
    // The session vanished between this panel mounting and the next
    // session-set effect in TerminalGrid picking up the removal — a rare,
    // instantaneous gap (the effect runs in the same commit cycle in
    // practice, but render can't assume that). Render nothing rather than
    // crash; the panel is on its way out.
    return null;
  }

  const focused = id === ctx.focusedSessionId;
  const isRestorable = s.status === "restorable";
  const isPoppedOut = ctx.poppedOutIds?.has(id) ?? false;
  const busy = ctx.restoringIds.has(id);

  return (
    <div
      onMouseDown={(e) => {
        // Contain the click here so it doesn't also trigger the
        // grid-gutter blur handler on the outer container.
        e.stopPropagation();
        ctx.onFocusCell(id);
      }}
      onFocus={() => ctx.onFocusCell(id)}
      data-dim-zone=""
      className={`flex h-full min-h-0 flex-col overflow-hidden border ${
        focused ? "border-garage-amber" : "border-garage-line"
      } ${isRestorable || isPoppedOut ? "opacity-70" : ""} ${
        s.status === "needs-input" ? "dim-exempt" : ""
      } ${ctx.columnActive ? "dim-focused" : ""}`}
    >
      <div className="flex flex-none items-center gap-2 border-b border-garage-line px-2 py-1 text-xs">
        <span className={colorFor(s.status)}>{glyphFor(s.status)}</span>
        <span className={focused ? "font-semibold text-garage-amber" : "text-garage-ink"}>{s.label}</span>
        <span className="ml-auto text-garage-faint">{s.status}</span>
        <button
          type="button"
          onClick={(e) => {
            e.stopPropagation();
            ctx.onPopOut?.(id);
          }}
          disabled={isPoppedOut}
          title="Pop out into a separate window"
          className="text-garage-dim hover:text-garage-amber disabled:opacity-30"
        >
          ⇱
        </button>
      </div>
      <div className="min-h-0 flex-1 p-1">
        {isPoppedOut ? (
          <button
            type="button"
            onClick={() => ctx.onReclaim(id)}
            className="flex h-full w-full flex-col items-center justify-center gap-2 text-garage-dim hover:text-garage-amber"
          >
            <span className="text-2xl">⇱</span>
            <span className="text-xs">viewing in separate window — reclaim</span>
          </button>
        ) : isRestorable ? (
          <div className="flex h-full flex-col items-center justify-center gap-2 text-garage-dim">
            <span className="text-2xl">⟳</span>
            <span className="text-xs">no live terminal — session needs to be restored</span>
            <button
              type="button"
              onClick={() => ctx.restoreOne(id)}
              disabled={busy}
              className="border border-garage-line px-2 py-0.5 text-xs text-garage-dim hover:border-garage-amber hover:text-garage-amber disabled:opacity-40"
            >
              restore
            </button>
            {ctx.restoreError && (
              <span className="max-w-[80%] text-center text-[11px] text-garage-red">{ctx.restoreError}</span>
            )}
          </div>
        ) : (
          <SessionTerminal id={id} />
        )}
      </div>
    </div>
  );
}
