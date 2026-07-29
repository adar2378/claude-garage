import React, { createContext, useCallback, useContext, useEffect, useMemo, useRef, useState } from "react";
import { DockviewReact } from "dockview-react";
import { themeLight, themeDark } from "dockview";
import "dockview/dist/styles/dockview.css";
import "../dockview-overrides.css";
import SessionTerminal from "../SessionTerminal.jsx";
import { glyphFor, colorFor, tipFor } from "../lib/status.js";
import { formatElapsed, useTicker } from "../lib/elapsed.js";
import { useEffectiveTheme } from "../lib/theme.js";
import { useSettings } from "../lib/settings.js";
import { restoreSession, deleteSession, finishWorktree, createSession } from "../lib/api.js";
import {
  loadOrBuildLayout,
  reconcile,
  resetLayout,
  saveLayout,
  registerPlacementHint,
} from "../lib/layout.js";

const SAVE_DEBOUNCE_MS = 300;

// First free `claude-N` label in this workspace's deck (p7 D-grid-controls:
// explicit splits and the grid-header spawn menu auto-label rather than
// prompting — the rail's + form remains the place to pick a name).
// Module-scope so the object identity is stable across renders — dockview
// re-applies its skin whenever the `theme` prop changes identity.
const GARAGE_DOCK_LIGHT = { ...themeLight, tabGroupIndicator: "none" };
const GARAGE_DOCK_DARK = { ...themeDark, tabGroupIndicator: "none" };

function nextAutoLabel(sessions) {
  const used = new Set(sessions.map((s) => s.label));
  let n = 1;
  while (used.has(`claude-${n}`)) n++;
  return `claude-${n}`;
}

// Threads live per-session state down into dockview's panel components.
// Dockview (react) mounts panel content *and* panel tabs through React
// portals owned by DockviewReact's own render tree (see dockview-react's
// ReactPanelContentPart / ReactPanelHeaderPart) — that tree stays inside
// this component's subtree, so a normal Context crosses the portal
// boundary exactly like it would for any other child. That's what lets a
// panel's content (SessionCellPanel) *and* its tab (SessionCellTab) react
// to session-state changes (status, restore-in-flight, popped-out,
// app-level focus) without ever calling `panel.api.updateParameters()`:
// each panel's own identity (the only thing dockview needs to persist) is
// just its session id — everything else is read fresh from context on
// every render.
const GridContext = createContext(null);

// Center pane (spec: 3.2, extended by p4's flexible-layout and p7's
// grid-controls capabilities). One dockview panel per session in the
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
  hiddenIds,
  onHideCell,
  columnActive,
  onActivateColumn,
  onAddWorkspace,
  gridActionsRef,
  views = [],
  focusedView,
  viewAssignments = {},
  activeViewIds,
  onSelectView,
  onDetachCell,
  onRejoinCell,
  onAssignNewSession,
}) {
  // redesign/light-minimal: the dock's base theme has to follow the app
  // theme. It was pinned to dockview's dark `abyss`, which left dockview's
  // own unoverridden chrome dark against a white wall. themeLight/themeDark
  // carry the matching colorScheme; `tabGroupIndicator: "none"` is carried
  // over from abyss because SessionCellTab IS the whole title bar and a
  // second group indicator would render as a stray bar above it.
  const [dockSettings] = useSettings();
  const appTheme = useEffectiveTheme(dockSettings.theme);
  const dockTheme = appTheme === "dark" ? GARAGE_DOCK_DARK : GARAGE_DOCK_LIGHT;

  // p8 grid-views: layouts persist PER VIEW — the main view keeps the
  // workspace's original key (backward compatible with pre-views
  // layouts), detached views get their own.
  const layoutKey = group
    ? focusedView && focusedView !== "main"
      ? `${group.name}::${focusedView}`
      : group.name
    : null;
  const [restoringIds, setRestoringIds] = useState(() => new Set());
  const [restoreError, setRestoreError] = useState(null);
  // p7 grid-controls: spawn-from-grid failures (409 label taken, 400
  // non-git for worktree spawns…) surface here, inline in the grid header.
  const [spawnError, setSpawnError] = useState(null);
  const [spawnMenuOpen, setSpawnMenuOpen] = useState(false);
  const spawnMenuRef = useRef(null);

  useEffect(() => {
    if (!spawnError) return;
    const timer = setTimeout(() => setSpawnError(null), 5000);
    return () => clearTimeout(timer);
  }, [spawnError]);

  useEffect(() => {
    if (!spawnMenuOpen) return;
    function onDocMouseDown(e) {
      if (spawnMenuRef.current && !spawnMenuRef.current.contains(e.target)) {
        setSpawnMenuOpen(false);
      }
    }
    document.addEventListener("mousedown", onDocMouseDown);
    return () => document.removeEventListener("mousedown", onDocMouseDown);
  }, [spawnMenuOpen]);

  // design D-wt-ui / D-wt-finish: the finish (merge/discard/keep) prompt
  // for a just-closed worktree session. Lives here, not in SessionCellTab,
  // because the panel that spawned it is gone by the time the DELETE
  // response comes back (the session is dead — reconcile() will have
  // dropped its panel on the very next sessions refetch) — see the render
  // below for the fixed-position toast this drives.
  // {sessionId, label, worktree: {path, branch, repoDir}, busy, error,
  //  discardArmed} — discardArmed is p7's two-step confirm (spec: "Discard
  //  requires a second, armed click").
  const [finishToast, setFinishToast] = useState(null);
  const discardArmTimerRef = useRef(null);

  useEffect(() => () => clearTimeout(discardArmTimerRef.current), []);

  const handleSessionClosed = useCallback((session, worktree) => {
    if (!worktree) return;
    setFinishToast({
      sessionId: session.id,
      label: session.label,
      worktree,
      busy: false,
      error: null,
      discardArmed: false,
    });
  }, []);

  async function handleFinishAction(action) {
    if (!finishToast || finishToast.busy) return;
    setFinishToast((prev) => (prev ? { ...prev, busy: true, error: null } : prev));
    try {
      await finishWorktree(finishToast.worktree, action);
      setFinishToast(null);
      onSessionsRestored?.();
    } catch (err) {
      setFinishToast((prev) => (prev ? { ...prev, busy: false, error: err.message } : prev));
    }
  }

  // p7 (spec: pit-wall-ui "Worktree finish prompt"): discard is the one
  // destructive finish action — first click arms it for 3s (destructive
  // styling + a warning naming the branch), only the second click within
  // the window actually discards. Same "sure?" mechanics as the ✕ kill.
  function handleDiscardClick() {
    if (!finishToast || finishToast.busy) return;
    if (!finishToast.discardArmed) {
      setFinishToast((prev) => (prev ? { ...prev, discardArmed: true } : prev));
      clearTimeout(discardArmTimerRef.current);
      discardArmTimerRef.current = setTimeout(() => {
        setFinishToast((prev) => (prev ? { ...prev, discardArmed: false } : prev));
      }, 3000);
      return;
    }
    clearTimeout(discardArmTimerRef.current);
    setFinishToast((prev) => (prev ? { ...prev, discardArmed: false } : prev));
    handleFinishAction("discard");
  }

  // "keep" is a pure dismiss (design D-wt-finish: "simply dismissing the
  // prompt is equivalent" to calling the endpoint with action:"keep") — no
  // API round-trip needed.
  function handleFinishKeep() {
    setFinishToast(null);
    onSessionsRestored?.();
  }

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

  // p7 D-grid-controls: an explicit split registers its placement hint
  // FIRST (synchronously — the SSE-driven sessions refetch can land before
  // createSession's own promise resolves), then spawns; reconcile()
  // consumes the hint when the new panel joins the layout. The new
  // session's id is knowable up front because ids are always
  // `garage/<workspace>/<label>` and we pick the label.
  const splitFrom = useCallback(
    async (referenceId, direction, { worktree = false } = {}) => {
      if (!group) return;
      const label = nextAutoLabel(group.sessions);
      const newId = `garage/${group.name}/${label}`;
      if (referenceId) registerPlacementHint(newId, referenceId, direction);
      try {
        await createSession(group.name, label, worktree ? { worktree: true } : {});
        // p8 grid-views: register the view assignment before the refetch
        // recomputes views, so a split inside a detached view lands there
        // instead of invisibly in main.
        onAssignNewSession?.(group.name, newId);
        // Focus only after the refetch resolves — App clamps focusedSessionId
        // to ids present in `sessions`, so focusing before the new session
        // lands in state would be immediately reverted.
        await onSessionsRestored?.();
        onFocusCell(newId);
      } catch (err) {
        setSpawnError(err.message);
      }
    },
    [group, onSessionsRestored, onFocusCell, onAssignNewSession]
  );

  // p7 task 1.3: dockview 7 ships group-maximize natively —
  // maximizeGroup / hasMaximizedGroup / exitMaximizedGroup (verified
  // against dockview-core 7.0.2's component.api.d.ts) — so the toggle is
  // a thin wrapper, no CSS solo-render fallback needed.
  const toggleMaximize = useCallback(() => {
    const api = apiRef.current;
    if (!api) return;
    if (api.hasMaximizedGroup()) {
      api.exitMaximizedGroup();
      return;
    }
    const panel = focusedSessionId ? api.getPanel(focusedSessionId) : null;
    // Floating cells are outside the tiling — maximize only applies to
    // grid-located groups.
    if (panel && panel.group.api.location.type === "grid") api.maximizeGroup(panel);
  }, [focusedSessionId]);

  // App's keydown listener owns `\` and `m` (single-listener design
  // D-keys), so the grid exposes its imperative actions through a ref App
  // hands down — same pattern as apiRef itself, just one level up.
  useEffect(() => {
    if (!gridActionsRef) return;
    gridActionsRef.current = {
      toggleMaximize,
      splitFocused: (direction) => {
        if (focusedSessionId) splitFrom(focusedSessionId, direction);
      },
    };
    return () => {
      gridActionsRef.current = null;
    };
  }, [gridActionsRef, toggleMaximize, splitFrom, focusedSessionId]);

  // Hidden sessions (design: hide control) are excluded here, before
  // `sessionIds` is derived below — that's what makes reconcile() drop
  // their panel from the grid, the same mechanism that removes a panel for
  // a session that ended entirely. Unlike popped-out sessions (which stay
  // full members of `group.sessions` and keep their panel slot, just
  // rendered as a reclaim placeholder — see poppedOutIds usage below),
  // hiding removes the cell from the grid outright; the rail is the only
  // place a hidden session still shows up (App owns that split).
  const visibleSessions = useMemo(
    () =>
      (group?.sessions ?? []).filter(
        (s) => !hiddenIds?.has(s.id) && (!activeViewIds || activeViewIds.has(s.id))
      ),
    [group, hiddenIds, activeViewIds]
  );

  const sessionsById = useMemo(() => {
    const map = new Map();
    for (const s of visibleSessions) map.set(s.id, s);
    return map;
  }, [visibleSessions]);

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
      onHideCell,
      onSessionsRestored,
      restoreOne,
      splitFrom,
      onSessionClosed: handleSessionClosed,
      // p8 grid-views: per-cell detach/rejoin + view lookup
      viewAssignments,
      hasMultipleViews: views.length > 1,
      onDetachCell: (id) => onDetachCell?.(group?.name, id),
      onRejoinCell: (id) => onRejoinCell?.(group?.name, id),
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
      onHideCell,
      onSessionsRestored,
      splitFrom,
      handleSessionClosed,
      viewAssignments,
      views,
      onDetachCell,
      onRejoinCell,
      group?.name,
    ]
  );

  const sessionIds = useMemo(() => visibleSessions.map((s) => s.id), [visibleSessions]);
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
    workspaceRef.current = layoutKey;
    loadOrBuildLayout(api, layoutKey, sessionIds);
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
  // (or the default grid). Same workspace, session set changed (spawn/
  // end/restore-completion) -> reconcile only, so an in-progress custom
  // arrangement is never discarded just because a session came or went —
  // it's already being persisted continuously via onDidLayoutChange above.
  useEffect(() => {
    const api = apiRef.current;
    if (!api || !group) return;
    // layoutKey changes on workspace switch AND view switch — both mean
    // "load a different persisted arrangement".
    if (workspaceRef.current !== layoutKey) {
      // Never carry a maximize across layouts — the maximized group
      // belongs to the layout being torn down.
      if (api.hasMaximizedGroup()) api.exitMaximizedGroup();
      workspaceRef.current = layoutKey;
      loadOrBuildLayout(api, layoutKey, sessionIds);
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
  }, [layoutKey, sessionIdsKey]);

  // The other half of the two-way focus binding: App-driven focus changes
  // (rail click, [ ]/a keybindings) follow through into dockview's own
  // active-panel state. p7 grid-controls: a focus change that lands
  // outside the maximized group exits maximize first (spec: "Needs-input
  // jump exits maximize") — focus within the maximized group (tab-stacked
  // cells) keeps the maximize.
  useEffect(() => {
    const api = apiRef.current;
    if (!api || !focusedSessionId) return;
    const panel = api.getPanel(focusedSessionId);
    if (!panel) return;
    if (api.hasMaximizedGroup() && !panel.group.api.isMaximized()) {
      api.exitMaximizedGroup();
    }
    if (!panel.api.isActive) panel.api.setActive();
  }, [focusedSessionId]);

  useEffect(() => () => clearTimeout(saveTimerRef.current), []);

  const handleResetLayout = useCallback(() => {
    const api = apiRef.current;
    if (!api || !layoutKey) return;
    if (api.hasMaximizedGroup()) api.exitMaximizedGroup();
    resetLayout(api, layoutKey, sessionIds);
  }, [layoutKey, sessionIds]);

  // design D-wt-finish / D-wt-ui: fixed bottom-right toast, rendered
  // regardless of which branch below fires — the session that spawned it
  // is already dead (and may have been the very last one in the grid), so
  // this can't live inside the panel/grid content itself.
  const finishToastNode = finishToast && (
    <div className="fixed bottom-4 right-4 z-50 flex items-center gap-2 rounded-lg border border-garage-line bg-garage-bg px-3 py-2 text-xs text-garage-ink shadow-sm">
      <span className="text-garage-dim">worktree for</span>
      <span className="font-semibold text-garage-ink">{finishToast.label}</span>
      <span className="text-garage-line">:</span>
      <button
        type="button"
        onClick={() => handleFinishAction("merge")}
        disabled={finishToast.busy}
        className="rounded-md px-2 py-1 text-xs text-garage-dim hover:bg-garage-sel hover:text-garage-ink disabled:opacity-40"
      >
        merge
      </button>
      <button
        type="button"
        onClick={handleDiscardClick}
        disabled={finishToast.busy}
        className={`rounded-md px-2 py-1 text-xs disabled:opacity-40 ${
          finishToast.discardArmed
            ? "text-garage-red"
            : "text-garage-dim hover:bg-garage-sel hover:text-garage-red"
        }`}
      >
        {finishToast.discardArmed ? "discard branch?" : "discard"}
      </button>
      <button
        type="button"
        onClick={handleFinishKeep}
        disabled={finishToast.busy}
        className="rounded-md px-2 py-1 text-xs text-garage-dim hover:bg-garage-sel hover:text-garage-ink disabled:opacity-40"
      >
        keep
      </button>
      {finishToast.discardArmed && !finishToast.error && (
        <span className="max-w-[16rem] truncate text-garage-red">
          click again — deletes unmerged work on {finishToast.worktree.branch}
        </span>
      )}
      {finishToast.error && (
        <span className="max-w-[14rem] truncate text-garage-red" title={finishToast.error}>
          · {finishToast.error}
        </span>
      )}
    </div>
  );

  if (!group) {
    // p7 (spec: pit-wall-ui "First-run empty state"): no focused workspace
    // means no workspaces exist at all (App auto-focuses the first group
    // whenever one exists), so this is the first-run surface — explain the
    // product, carry the primary action, point at `?`.
    return (
      <>
        <div
          onMouseDown={onBlurChrome}
          onMouseDownCapture={onActivateColumn}
          className="grid min-h-0 place-items-center bg-garage-bg p-6"
        >
          <div className="max-w-md space-y-4 rounded-xl border border-garage-line bg-garage-panel px-8 py-7">
            <h2 className="text-[15px] font-semibold text-garage-ink">your pit wall is empty</h2>
            <p className="text-[13px] text-garage-dim">
              claude-garage runs several Claude Code sessions side by side and tells you the
              moment one needs you.
            </p>
            <ol className="list-decimal space-y-1.5 pl-5 text-[13px] text-garage-dim">
              <li>
                <span className="font-semibold text-garage-ink">add a workspace</span> — point it
                at a project folder
              </li>
              <li>
                <span className="font-semibold text-garage-ink">spawn sessions</span> — the{" "}
                <span className="font-mono text-garage-ink">+</span> next to the workspace name in the rail
              </li>
              <li>
                <span className="font-semibold text-garage-ink">triage</span> — amber{" "}
                <span className="text-garage-amber">●</span> means Claude is waiting; press{" "}
                <span className="font-mono text-garage-ink">a</span> to jump there
              </li>
            </ol>
            <button
              type="button"
              onClick={() => onAddWorkspace?.()}
              className="rounded-md bg-garage-ink px-4 py-2 text-[13px] font-medium text-garage-bg hover:opacity-90"
            >
              + add your first workspace
            </button>
            <p className="text-[11px] text-garage-faint">
              press <span className="font-mono text-garage-ink">?</span> anytime for keybindings &amp; the
              status legend
            </p>
          </div>
        </div>
        {finishToastNode}
      </>
    );
  }

  if (group.sessions.length === 0) {
    return (
      <>
        <div
          onMouseDown={onBlurChrome}
          onMouseDownCapture={onActivateColumn}
          className="grid min-h-0 place-items-center bg-garage-bg text-xs text-garage-dim"
        >
          <span>
            no sessions in {group.name} yet — spawn one with the{" "}
            <span className="font-mono text-garage-ink">+</span> next to its name in the rail
          </span>
        </div>
        {finishToastNode}
      </>
    );
  }

  if (visibleSessions.length === 0) {
    return (
      <>
        <div
          onMouseDown={onBlurChrome}
          onMouseDownCapture={onActivateColumn}
          className="grid min-h-0 place-items-center bg-garage-bg text-xs text-garage-dim"
        >
          every session in this workspace is hidden — click one in the rail to bring it back
        </div>
        {finishToastNode}
      </>
    );
  }

  return (
    <div
      onMouseDown={onBlurChrome}
      onMouseDownCapture={onActivateColumn}
      className="flex min-h-0 flex-col overflow-hidden bg-garage-bg"
    >
      <div className="flex h-10 flex-none items-center gap-2 border-b border-garage-line bg-garage-panel px-3 text-[13px]">
        <span className="font-medium text-garage-ink">{group.name}</span>

        {/* p7 grid-controls toolbar: VS Code's terminal affordances — + ▾
            spawn menu, split right/down, maximize — acting on the focused
            cell. Splits also live on each cell's tab (SessionCellTab). */}
        <div ref={spawnMenuRef} className="relative flex items-center gap-1">
          <button
            type="button"
            onClick={() => setSpawnMenuOpen((v) => !v)}
            title={`new session in ${group.name}`}
            aria-haspopup="menu"
            aria-expanded={spawnMenuOpen}
            className="rounded-md px-2 py-1 text-garage-dim hover:bg-garage-sel hover:text-garage-ink"
          >
            + <span className="text-xs text-garage-faint">▾</span>
          </button>
          {spawnMenuOpen && (
            <div
              role="menu"
              className="absolute left-0 top-full z-20 mt-1 w-64 rounded-lg border border-garage-line bg-garage-bg p-1 shadow-sm"
            >
              <button
                type="button"
                role="menuitem"
                onClick={() => {
                  setSpawnMenuOpen(false);
                  splitFrom(focusedSessionId, "right");
                }}
                className="flex w-full flex-col rounded-md px-3 py-2 text-left hover:bg-garage-sel"
              >
                <span className="text-[13px] text-garage-ink">new session</span>
                <span className="text-xs text-garage-faint">
                  spawns beside the focused cell — same directory
                </span>
              </button>
              <button
                type="button"
                role="menuitem"
                onClick={() => {
                  setSpawnMenuOpen(false);
                  splitFrom(focusedSessionId, "right", { worktree: true });
                }}
                className="flex w-full flex-col rounded-md px-3 py-2 text-left hover:bg-garage-sel"
              >
                <span className="text-[13px] text-garage-ink">new worktree session</span>
                <span className="text-xs text-garage-faint">
                  isolated git worktree on branch garage/&lt;label&gt;
                </span>
              </button>
            </div>
          )}
          <button
            type="button"
            onClick={() => focusedSessionId && splitFrom(focusedSessionId, "right")}
            disabled={!focusedSessionId}
            title="split focused cell right (\)"
            className="rounded-md px-1.5 py-1 text-garage-dim hover:bg-garage-sel hover:text-garage-ink disabled:opacity-30"
          >
            ◫
          </button>
          <button
            type="button"
            onClick={() => focusedSessionId && splitFrom(focusedSessionId, "below")}
            disabled={!focusedSessionId}
            title="split focused cell down"
            className="rounded-md px-1.5 py-1 text-garage-dim hover:bg-garage-sel hover:text-garage-ink disabled:opacity-30"
          >
            ⬒
          </button>
          <button
            type="button"
            onClick={toggleMaximize}
            disabled={!focusedSessionId}
            title="maximize focused cell — toggle (m)"
            className="rounded-md px-1.5 py-1 text-garage-dim hover:bg-garage-sel hover:text-garage-ink disabled:opacity-30"
          >
            ⛶
          </button>
        </div>

        {spawnError && (
          <span className="max-w-[18rem] truncate text-[11px] text-garage-red" title={spawnError}>
            {spawnError}
          </span>
        )}

        <button
          type="button"
          onClick={handleResetLayout}
          title="Discard the custom layout and restore the default grid"
          className="ml-auto rounded-md px-2.5 py-1 text-xs text-garage-dim hover:bg-garage-sel hover:text-garage-ink"
        >
          reset layout
        </button>
      </div>

      {/* p8 grid-views: view strip — appears only once a second view
          exists. One view on screen at a time; a background view with a
          needs-input member carries the accent dot so nothing blocked can
          hide behind a view switch. */}
      {views.length > 1 && (
        <div className="flex flex-none items-center gap-1 border-b border-garage-line bg-garage-panel px-3 py-1.5 text-xs">
          {views.map((v) => {
            const active = v.name === focusedView;
            return (
              <button
                key={v.name}
                type="button"
                onClick={() => onSelectView?.(group.name, v.name)}
                title={`show view "${v.name}" (${v.sessions.length} session${
                  v.sessions.length === 1 ? "" : "s"
                })`}
                className={`flex items-center gap-1.5 rounded-md px-2.5 py-1 ${
                  active
                    ? "bg-garage-sel text-garage-ink"
                    : "text-garage-dim hover:bg-garage-sel"
                }`}
              >
                {v.needsCount > 0 && <span className="text-garage-amber">●</span>}
                {v.name}
                <span className="text-garage-faint">{v.sessions.length}×</span>
              </button>
            );
          })}
        </div>
      )}
      <div className="min-h-0 flex-1 p-2">
        <GridContext.Provider value={contextValue}>
          <DockviewReact
            className="garage-dock"
            theme={dockTheme}
            components={{ terminal: SessionCellPanel }}
            // Every panel in this grid is the same "terminal" content
            // component, so one tab component covers all of them —
            // `defaultTabComponent` is dockview-react's fallback used
            // whenever a panel doesn't name its own `tabComponent`
            // (layout.js's addPanel calls never do), which means this is
            // wired without layout.js needing to know a tab component
            // exists. `singleTabMode="fullwidth"` is dockview's own
            // built-in for "stretch the (one) tab to fill the tab strip" —
            // see dockview-overrides.css for the CSS that backstops it.
            defaultTabComponent={SessionCellTab}
            singleTabMode="fullwidth"
            onReady={handleReady}
          />
        </GridContext.Provider>
      </div>
      {finishToastNode}
    </div>
  );
}

// Dockview's per-panel tab — the ONE bar per cell, and also dockview's
// native drag handle. `props.api.id` is the dockview panel id, which is
// always a session id (see layout.js — every addPanel call uses the
// session id as the panel id); `props` otherwise follows
// IDockviewPanelHeaderProps (api/containerApi/params/tabLocation), but
// everything this needs — the session record, focus state, popout state —
// comes from GridContext instead, same as SessionCellPanel below.
//
// This is rendered inside dockview's `.dv-tab` element (see
// dockview-core's `Tab` class), which is what dockview attaches its
// drag-and-drop listeners to — so dragging this tab to redock a panel
// keeps working automatically as long as nothing here swallows the
// mousedown that starts it. The one thing that must never start a drag is
// a button, hence `stopPropagation` on every button's `onMouseDown`
// (dockview's HTML5 drag source arms on mousedown/dragstart, before
// `onClick` ever fires).
//
// D-dim: the tab is its own dim zone (`data-dim-zone`), driven by the same
// focused/needs-input/columnActive inputs as SessionCellPanel's content
// zone below, so the merged bar dims and un-dims in lockstep with the rest
// of the cell rather than as a separately-lit strip above a dimmed body.
function SessionCellTab({ api, containerApi }) {
  const ctx = useContext(GridContext);
  const id = api.id;
  const s = ctx.sessionsById.get(id);

  // p8 float-in-page: is this cell currently a floating group rather than
  // a grid tile? Driven by dockview's own location events so the toggle
  // stays correct however the cell got there (button, or dragging a
  // floating group back into the grid by its tab).
  const [isFloating, setIsFloating] = useState(() => api.location.type === "floating");
  useEffect(() => {
    const disposable = api.onDidLocationChange((e) =>
      setIsFloating(e.location.type === "floating")
    );
    return () => disposable.dispose();
  }, [api]);

  // Float ⇄ dock. Floating uses dockview's native floating groups — the
  // cell lifts out of the tiling into a draggable/resizable window INSIDE
  // the page (unlike ⇱ pop-out, which opens a separate browser window).
  // Docking back tucks it beside an existing grid tile; with no grid tile
  // left, dragging the tab onto the empty grid still works natively.
  function toggleFloat() {
    const panel = containerApi.getPanel(id);
    if (!panel) return;
    if (panel.group.api.location.type === "floating") {
      const target = containerApi.panels.find(
        (p) => p.id !== id && p.group.api.location.type === "grid"
      );
      if (target) panel.api.moveTo({ group: target.group, position: "right" });
      return;
    }
    containerApi.addFloatingGroup(panel, { x: 48, y: 32, width: 640, height: 420 });
  }

  // Two-step confirm for the close (✕) button — click 1 arms it ("sure?",
  // text-garage-red) for 3s, click 2 within that window actually deletes.
  // Mirrors WorkspaceRail's workspace-removal confirm, but kept local to
  // this component instance rather than lifted to GridContext: each
  // dockview panel/tab mounts its own SessionCellTab, so there's no
  // cross-cell state to coordinate — every cell arms/disarms independently
  // by construction.
  const [closeArmed, setCloseArmed] = useState(false);
  const [closing, setClosing] = useState(false);
  const [closeError, setCloseError] = useState(null);
  const armTimerRef = useRef(null);
  const errorTimerRef = useRef(null);

  useEffect(
    () => () => {
      clearTimeout(armTimerRef.current);
      clearTimeout(errorTimerRef.current);
    },
    []
  );

  // Header timer: advances the elapsed-time readout every second (design
  // contract §6) — a no-op-safe interval hook, cheap to call unconditionally
  // even on the early `!s` return below since hooks must run every render.
  useTicker();

  if (!s) return null;

  const focused = id === ctx.focusedSessionId;
  const isPoppedOut = ctx.poppedOutIds?.has(id) ?? false;
  // v1: the close control only ever targets a session with a real tmux
  // session behind it — a restorable entry has none, and DELETE would just
  // 404 (see daemon/src/sessions.js). Popped-out cells are still live
  // (only their *rendering* moved to another window), so they keep ✕.
  const isLive = s.status !== "restorable";
  const isNeedsInput = s.status === "needs-input";

  async function handleClose() {
    if (!closeArmed) {
      setCloseArmed(true);
      clearTimeout(armTimerRef.current);
      armTimerRef.current = setTimeout(() => setCloseArmed(false), 3000);
      return;
    }
    clearTimeout(armTimerRef.current);
    setCloseArmed(false);
    setClosing(true);
    setCloseError(null);
    try {
      const body = await deleteSession(id);
      // design D-wt-meta/D-wt-ui: a worktree session's DELETE response
      // carries the {path, branch, repoDir} record — hand it up to
      // TerminalGrid so it can drive the finish (merge/discard/keep) toast.
      // Non-worktree sessions get `worktree: null` (or no field on an
      // older daemon), so this is a no-op for them — exactly today's
      // behavior.
      if (body?.worktree) {
        ctx.onSessionClosed?.(s, body.worktree);
      }
      // Same refetch path restoreOne already uses — a close is just
      // another kind of "the session set changed under us" event.
      // Deferred a tick: this handler runs from a button INSIDE the tab
      // dockview is about to dispose; letting the refetch → reconcile →
      // removePanel chain run while this click's stack is still unwinding
      // makes dockview double-dispose the tab ("resource already
      // disposed"). One macrotask later, the stack is clear.
      setTimeout(() => ctx.onSessionsRestored?.(), 0);
    } catch (err) {
      setCloseError(err.message);
      clearTimeout(errorTimerRef.current);
      errorTimerRef.current = setTimeout(() => setCloseError(null), 4000);
    } finally {
      setClosing(false);
    }
  }

  return (
    <div
      data-dim-zone=""
      className={`group/cell flex h-9 w-full items-center gap-2 border-b border-garage-line bg-garage-panel px-3 ${
        isNeedsInput ? "bg-garage-amber/5" : ""
      } ${isNeedsInput ? "dim-exempt" : ""} ${ctx.columnActive ? "dim-focused" : ""}`}
    >
      <span className={colorFor(s.status)} title={tipFor(s.status)}>
        {glyphFor(s.status)}
      </span>
      <span
        className={`text-[13px] tracking-tight text-garage-ink ${
          focused ? "font-semibold" : "font-medium"
        }`}
      >
        {s.label}
      </span>
      {s.branch && (
        <span
          className="max-w-[90px] shrink-0 truncate font-mono text-[11px] text-garage-faint"
          title={s.branch}
        >
          ⎇ {s.branch}
        </span>
      )}
      <span
        className={`ml-auto font-mono text-xs tabular-nums ${
          isNeedsInput ? "font-semibold text-garage-amber" : "text-garage-faint"
        }`}
      >
        {formatElapsed(s.since)}
      </span>
      {closeError && (
        <span className="max-w-[9rem] truncate text-garage-red" title={closeError}>
          {closeError}
        </span>
      )}
      {/* Rest-state hint that this cell has controls, shown ONLY at rest:
          it fades out exactly as the real hover-reveal row below fades in,
          so the two never occupy the header at the same time. Decorative
          and aria-hidden — the controls themselves stay permanently in the
          accessibility tree (see the note on the row below). */}
      <span
        className="text-garage-faint transition-opacity group-hover/cell:opacity-0 group-focus-within/cell:opacity-0"
        aria-hidden="true"
      >
        ⋯
      </span>
      {/* Every secondary control below stays mounted at all times (so
          keyboard focus and every onClick/title/disabled binding keep
          working exactly as before) — only its visibility changes: hidden
          at rest, revealed on cell hover or when a control inside has
          keyboard focus. */}
      <div className="flex items-center gap-0.5 opacity-0 transition-opacity focus-within:opacity-100 group-hover/cell:opacity-100">
        {/* p7 grid-controls: per-cell splits, same semantics as the header
            toolbar but anchored to THIS cell — VS Code's per-terminal split. */}
        {isLive && (
          <button
            type="button"
            onMouseDown={(e) => e.stopPropagation()}
            onClick={(e) => {
              e.stopPropagation();
              ctx.splitFrom?.(id, "right");
            }}
            title="split right — new session beside this one"
            className="rounded-md p-1 text-garage-faint hover:bg-garage-sel hover:text-garage-ink"
          >
            ◫
          </button>
        )}
        {isLive && (
          <button
            type="button"
            onMouseDown={(e) => e.stopPropagation()}
            onClick={(e) => {
              e.stopPropagation();
              ctx.splitFrom?.(id, "below");
            }}
            title="split down — new session below this one"
            className="rounded-md p-1 text-garage-faint hover:bg-garage-sel hover:text-garage-ink"
          >
            ⬒
          </button>
        )}
        {/* p8 grid-views: detach ⇄ rejoin. Detach makes this session a
            standalone view (grid shows it alone; the others stay grouped in
            their view); rejoin returns it to main. */}
        {isLive && (
          (ctx.viewAssignments?.[id] ?? "main") !== "main" ? (
            <button
              type="button"
              onMouseDown={(e) => e.stopPropagation()}
              onClick={(e) => {
                e.stopPropagation();
                ctx.onRejoinCell?.(id);
              }}
              title="rejoin the main view — back into the group"
              className="rounded-md bg-garage-sel p-1 text-garage-ink hover:bg-garage-sel"
            >
              ◱
            </button>
          ) : (
            <button
              type="button"
              onMouseDown={(e) => e.stopPropagation()}
              onClick={(e) => {
                e.stopPropagation();
                ctx.onDetachCell?.(id);
              }}
              title="standalone — move this session to its own view (switch back anytime from the rail or view strip)"
              className="rounded-md p-1 text-garage-faint hover:bg-garage-sel hover:text-garage-ink"
            >
              ◲
            </button>
          )
        )}
        <button
          type="button"
          onMouseDown={(e) => e.stopPropagation()}
          onClick={(e) => {
            e.stopPropagation();
            toggleFloat();
          }}
          title={
            isFloating
              ? "dock back into the grid"
              : "float this cell — a draggable window above the grid, same page"
          }
          className={`rounded-md p-1 ${
            isFloating
              ? "bg-garage-sel text-garage-ink"
              : "text-garage-faint hover:bg-garage-sel hover:text-garage-ink"
          }`}
        >
          {isFloating ? "⇲" : "❐"}
        </button>
        <button
          type="button"
          onMouseDown={(e) => e.stopPropagation()}
          onClick={(e) => {
            e.stopPropagation();
            ctx.onHideCell?.(id);
          }}
          title="Hide this cell for this page run (rail keeps it — click there to bring it back)"
          className="rounded-md p-1 text-garage-faint hover:bg-garage-sel hover:text-garage-ink"
        >
          –
        </button>
        <button
          type="button"
          onMouseDown={(e) => e.stopPropagation()}
          onClick={(e) => {
            e.stopPropagation();
            ctx.onPopOut?.(id);
          }}
          disabled={isPoppedOut}
          title="Pop out into a separate window"
          className="rounded-md p-1 text-garage-faint hover:bg-garage-sel hover:text-garage-ink disabled:opacity-30"
        >
          ⇱
        </button>
        {isLive && (
          <button
            type="button"
            onMouseDown={(e) => e.stopPropagation()}
            onClick={(e) => {
              e.stopPropagation();
              handleClose();
            }}
            disabled={closing}
            title={
              closeArmed
                ? "click again to close — kills the tmux session"
                : "close this session — kills the tmux session"
            }
            className={`rounded-md p-1 disabled:opacity-40 ${
              closeArmed ? "text-garage-red" : "text-garage-faint hover:bg-garage-sel hover:text-garage-red"
            }`}
          >
            {closeArmed ? "sure?" : "✕"}
          </button>
        )}
      </div>
    </div>
  );
}

// A dockview panel's content — just the terminal / placeholder body; the
// title bar lives in the tab (SessionCellTab above). `props.api.id` is
// the dockview panel id, which is always a session id (see layout.js —
// every addPanel call uses the session id as the panel id). Everything
// else needed to render the cell — the session record, focus state,
// restore/popout state — comes from GridContext, not from dockview's own
// params (see that context's definition for why).
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

  // p7 connection-resilience: SessionTerminal reports its WS health up;
  // while down, an overlay replaces silent staleness with an explicit
  // "connection lost — tmux is fine" message + a manual retry that skips
  // the current backoff wait.
  const [connected, setConnected] = useState(true);
  const [reconnectNonce, setReconnectNonce] = useState(0);

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
      data-session-id={id}
      className={`flex h-full min-h-0 flex-col overflow-hidden rounded-lg border ${
        s.status === "needs-input"
          ? "border-garage-amber"
          : focused
            ? "cell-border-focused"
            : "border-garage-line"
      } ${isRestorable || isPoppedOut ? "opacity-70" : ""} ${
        s.status === "needs-input" ? "dim-exempt" : ""
      } ${ctx.columnActive ? "dim-focused" : ""}`}
    >
      <div className="relative min-h-0 flex-1 p-1">
        {isPoppedOut ? (
          <button
            type="button"
            onClick={() => ctx.onReclaim(id)}
            className="flex h-full w-full flex-col items-center justify-center gap-2 text-garage-dim hover:text-garage-ink"
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
              className="rounded-md px-2.5 py-1 text-xs text-garage-dim hover:bg-garage-sel hover:text-garage-ink disabled:opacity-40"
            >
              restore
            </button>
            {ctx.restoreError && (
              <span className="max-w-[80%] text-center text-[11px] text-garage-red">{ctx.restoreError}</span>
            )}
          </div>
        ) : (
          <>
            <SessionTerminal
              id={id}
              onConnectionChange={setConnected}
              reconnectSignal={reconnectNonce}
            />
            {!connected && (
              <div className="absolute inset-0 z-10 flex flex-col items-center justify-center gap-2 bg-garage-bg/90 p-3 text-center">
                <span className="text-xs text-garage-red">connection to daemon lost</span>
                <span className="text-[11px] text-garage-dim">
                  the tmux session is still alive — only this view is detached
                </span>
                <button
                  type="button"
                  onClick={() => setReconnectNonce((n) => n + 1)}
                  className="rounded-md bg-garage-ink px-3 py-1 text-xs text-garage-bg hover:opacity-90"
                >
                  reconnect now
                </button>
              </div>
            )}
          </>
        )}
      </div>
    </div>
  );
}
