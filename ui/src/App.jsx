import React, { useCallback, useEffect, useMemo, useRef, useState } from "react";
import WorkspaceRail from "./components/WorkspaceRail.jsx";
import TerminalGrid from "./components/TerminalGrid.jsx";
import SoloView from "./components/SoloView.jsx";
import HooksBanner from "./components/HooksBanner.jsx";
import AddWorkspaceForm from "./components/AddWorkspaceForm.jsx";
import ChangesPane from "./components/ChangesPane.jsx";
import ReviewMode from "./components/ReviewMode.jsx";
import HelpOverlay from "./components/HelpOverlay.jsx";
import { fetchSessions, fetchWorkspaces, reportVisibility, fetchDiff, openEditor } from "./lib/api.js";
import { buildGroupTree } from "./lib/groups.js";
import SettingsPopover from "./components/SettingsPopover.jsx";
import { useSettings } from "./lib/settings.js";
import { loadViewedMap, markViewed, pruneViewed, hashContent } from "./lib/viewed.js";
import { firstHunkLine } from "./lib/diff.js";
import { listPoppedOut, openPopout, clearPopout, subscribe as subscribePopouts } from "./lib/popouts.js";

// Kept for the page's lifetime (module scope, not persisted) — see API
// contract for POST /api/ui/visibility.
const CLIENT_ID = crypto.randomUUID();
const VISIBILITY_INTERVAL_MS = 30_000;
const LOAD_RETRY_MS = 3_000;

// design D-popout: a popout window is opened at `/?solo=<id>` (see
// lib/popouts.js#openPopout) and never navigates elsewhere for the rest of
// its life, so this is stable for the whole lifetime of whichever branch
// of App a given mount takes below — reading it before any hooks run and
// branching on it is safe (every render of a single mounted instance takes
// the same branch; React's hooks-order rule is about a single instance,
// not about App-the-component-type in the abstract).
function readSoloId() {
  if (typeof window === "undefined") return null;
  return new URLSearchParams(window.location.search).get("solo");
}

export default function App() {
  const soloId = readSoloId();
  if (soloId) {
    return <SoloView id={soloId} />;
  }

  const [workspaces, setWorkspaces] = useState([]);
  const [sessions, setSessions] = useState([]);
  const [loadError, setLoadError] = useState(null);
  const [focusedWorkspace, setFocusedWorkspace] = useState(null);
  const [focusedSessionId, setFocusedSessionId] = useState(null);
  const [showAddWorkspace, setShowAddWorkspace] = useState(false);

  // ---- p4-layout-focus-workspace-ux: D-dim (reworked to column-level
  // semantics) — which of the three grid columns ('rail' | 'grid' | 'pane')
  // is the dimming spotlight's target. Tracked by last interaction: a
  // pointer mousedown inside a column's own wrapper (capture-phase so it
  // fires ahead of any inner stopPropagation — see the three
  // onMouseDownCapture handlers below) or a chrome keybinding that clearly
  // targets one column (see the keydown handler further down). Defaults to
  // 'grid' since that's the primary working surface on first load.
  const [activeColumn, setActiveColumn] = useState("grid");
  const activateRailColumn = useCallback(() => setActiveColumn("rail"), []);
  const activateGridColumn = useCallback(() => setActiveColumn("grid"), []);
  const activatePaneColumn = useCallback(() => setActiveColumn("pane"), []);

  // ---- p2-diff-review: changes pane + review mode state ----
  const [diffFiles, setDiffFiles] = useState([]);
  const [diffTruncated, setDiffTruncated] = useState(false);
  const [diffLoading, setDiffLoading] = useState(false);
  const [diffError, setDiffError] = useState(null);
  const [changesPaneCollapsed, setChangesPaneCollapsed] = useState(false);
  const [paneEmphasis, setPaneEmphasis] = useState("list"); // "list" | "diff" (Tab toggles)
  const [selectedFilePath, setSelectedFilePath] = useState(null);
  const [reviewMode, setReviewMode] = useState(false);
  const [viewedMap, setViewedMap] = useState({});
  const [editorError, setEditorError] = useState(null);
  // p3-restore-and-ship: D-help
  const [helpOpen, setHelpOpen] = useState(false);

  // ---- p4-layout-focus-workspace-ux: popout tracking (design D-popout) ----
  // `poppedOutIds` re-derives from localStorage on every change the
  // subscription notices (cross-window writes via `storage`, plus a 5s
  // poll for same-window writes and pure staleness timeouts — see
  // lib/popouts.js) so TerminalGrid always renders live-vs-placeholder
  // cells off the same source of truth a popout window itself reads.
  const [poppedOutIds, setPoppedOutIds] = useState(() => listPoppedOut());

  useEffect(() => {
    const unsubscribe = subscribePopouts(() => setPoppedOutIds(listPoppedOut()));
    return unsubscribe;
  }, []);

  // openPopout/clearPopout write synchronously but don't themselves fire a
  // `storage` event in *this* window (that only fires in other
  // tabs/windows) — so these wrappers refresh local state immediately
  // rather than waiting on the next poll tick.
  const handlePopOut = useCallback((id) => {
    openPopout(id);
    setPoppedOutIds(listPoppedOut());
  }, []);

  const handleReclaim = useCallback((id) => {
    clearPopout(id);
    setPoppedOutIds(listPoppedOut());
  }, []);

  // Hidden ids are plain in-memory React state, deliberately NOT persisted
  // anywhere (unlike poppedOutIds, which survives via localStorage) — the
  // spec is "removes the panel for this page run only": reloading or
  // reopening claude-garage must restore every hidden session. The rail
  // keeps listing hidden sessions (with a dim + "hidden" indicator — see
  // WorkspaceRail); only TerminalGrid's panel set excludes them (see its
  // `hiddenIds` prop). Declared here (rather than down by hideSession
  // itself) since selectSession below also needs to clear it.
  const [hiddenIds, setHiddenIds] = useState(() => new Set());

  const { flattenedGroups: groups } = useMemo(
    () => buildGroupTree(workspaces, sessions),
    [workspaces, sessions]
  );
  const [settings] = useSettings();

  // ---- initial load, with resilience against the daemon not being up yet ----
  useEffect(() => {
    let cancelled = false;
    let retryTimer;

    async function load() {
      try {
        const [ws, ss] = await Promise.all([fetchWorkspaces(), fetchSessions()]);
        if (cancelled) return;
        setWorkspaces(ws);
        setSessions(ss);
        setLoadError(null);
      } catch (e) {
        if (cancelled) return;
        setLoadError(e.message);
        retryTimer = setTimeout(load, LOAD_RETRY_MS);
      }
    }

    load();
    return () => {
      cancelled = true;
      clearTimeout(retryTimer);
    };
  }, []);

  const refreshSessions = useCallback(() => {
    fetchSessions()
      .then(setSessions)
      .catch((e) => setLoadError(e.message));
  }, []);

  const refreshWorkspaces = useCallback(() => {
    fetchWorkspaces()
      .then(setWorkspaces)
      .catch((e) => setLoadError(e.message));
  }, []);

  // ---- p2-diff-review: diff fetch, kept in refs so the SSE effect below
  // (which only wants to (re)subscribe on refreshSessions changing) can
  // read the *latest* focused workspace / session list without reopening
  // the EventSource every time focus moves (design D-freshness).
  const focusedWorkspaceRef = useRef(focusedWorkspace);
  useEffect(() => {
    focusedWorkspaceRef.current = focusedWorkspace;
  }, [focusedWorkspace]);

  const sessionsRef = useRef(sessions);
  useEffect(() => {
    sessionsRef.current = sessions;
  }, [sessions]);

  const fetchDiffForWorkspace = useCallback((name) => {
    if (!name) return;
    setDiffLoading(true);
    fetchDiff(name)
      .then((data) => {
        const files = data.files ?? [];
        setDiffFiles(files);
        setDiffTruncated(!!data.truncated);
        setDiffError(null);
        pruneViewed(name, files.map((f) => f.path));
      })
      .catch((e) => {
        setDiffError(e.message);
        setDiffFiles([]);
      })
      .finally(() => setDiffLoading(false));
  }, []);

  // Fetch on focused-workspace change (spec: "Pane updates when focus moves").
  useEffect(() => {
    if (focusedWorkspace) fetchDiffForWorkspace(focusedWorkspace);
  }, [focusedWorkspace, fetchDiffForWorkspace]);

  // ---- SSE: live status pushes drive the rail/grid without reload ----
  useEffect(() => {
    const es = new EventSource("/api/events");
    let hasOpenedBefore = false;

    es.addEventListener("open", () => {
      // First open is just the initial connection (we already did a GET
      // above). Any subsequent "open" means the browser reconnected after
      // a drop — resync via GET /api/sessions per design's D-push mitigation.
      if (hasOpenedBefore) refreshSessions();
      hasOpenedBefore = true;
    });

    es.addEventListener("status", (e) => {
      let payload;
      try {
        payload = JSON.parse(e.data);
      } catch {
        return;
      }
      const { id, status } = payload;
      // Diff freshness trigger 1/3 (design D-freshness): a session in the
      // *focused* workspace flipping to done refetches the diff. Read from
      // refs so this doesn't force the SSE connection to reopen on every
      // focus/session change.
      if (status === "done") {
        const session = sessionsRef.current.find((s) => s.id === id);
        if (session && session.workspace === focusedWorkspaceRef.current) {
          fetchDiffForWorkspace(focusedWorkspaceRef.current);
        }
      }
      setSessions((prev) => prev.map((s) => (s.id === id ? { ...s, status } : s)));
    });

    es.addEventListener("sessions", refreshSessions);

    return () => es.close();
  }, [refreshSessions, fetchDiffForWorkspace]);

  // ---- visibility reporting (contract: on load, on visibilitychange, every 30s while visible) ----
  useEffect(() => {
    const send = () => reportVisibility(CLIENT_ID, document.visibilityState === "visible");
    send();
    document.addEventListener("visibilitychange", send);
    const interval = setInterval(() => {
      if (document.visibilityState === "visible") send();
    }, VISIBILITY_INTERVAL_MS);
    return () => {
      document.removeEventListener("visibilitychange", send);
      clearInterval(interval);
    };
  }, []);

  // ---- keep focus valid as groups change (initial pick, or workspace/session disappearing) ----
  useEffect(() => {
    if (focusedWorkspace && groups.some((g) => g.name === focusedWorkspace)) return;
    if (groups.length > 0) setFocusedWorkspace(groups[0].name);
  }, [groups, focusedWorkspace]);

  // Falls back to the first *visible* session in the group where possible
  // (matters when the previously-focused session just got hidden and this
  // effect's own "still exists?" check passes trivially — hiding never
  // removes a session from `group.sessions`, only from the grid's panel
  // set) so this doesn't fight hideSession's own next-visible hand-off
  // below by re-focusing something hidden.
  useEffect(() => {
    const group = groups.find((g) => g.name === focusedWorkspace);
    if (!group) return;
    if (!group.sessions.some((s) => s.id === focusedSessionId)) {
      const firstVisible = group.sessions.find((s) => !hiddenIds.has(s.id));
      setFocusedSessionId(firstVisible?.id ?? group.sessions[0]?.id ?? null);
    }
  }, [groups, focusedWorkspace, focusedSessionId, hiddenIds]);

  const selectWorkspace = useCallback((name) => {
    setFocusedWorkspace(name);
  }, []);

  // Clicking a session row in the rail always focuses it — including a
  // hidden one, which this un-hides first (design: "clicking the session
  // row in the rail un-hides it and focuses it").
  const selectSession = useCallback((workspaceName, sessionId) => {
    setFocusedWorkspace(workspaceName);
    setFocusedSessionId(sessionId);
    setHiddenIds((prev) => {
      if (!prev.has(sessionId)) return prev;
      const next = new Set(prev);
      next.delete(sessionId);
      return next;
    });
  }, []);

  // Hide (– control): removes the session's panel from the grid for this
  // page run only (in-memory state above, never persisted). Hiding the
  // currently-focused cell hands focus to the next visible session in the
  // group — same wrap-around "next" semantics as the []/[ keybinding just
  // below — so focus never lingers on a cell that just vanished from the
  // grid; if every other session in the group is also hidden, focus falls
  // through to null (the group-effect above then has nothing visible to
  // pick either, until something gets unhidden).
  const hideSession = useCallback(
    (id) => {
      setHiddenIds((prev) => {
        if (prev.has(id)) return prev;
        const next = new Set(prev);
        next.add(id);
        return next;
      });

      setFocusedSessionId((current) => {
        if (current !== id) return current;
        const group = groups.find((g) => g.sessions.some((s) => s.id === id));
        if (!group) return current;
        const idx = group.sessions.findIndex((s) => s.id === id);
        for (let step = 1; step <= group.sessions.length; step++) {
          const candidate = group.sessions[(idx + step) % group.sessions.length];
          if (candidate.id !== id && !hiddenIds.has(candidate.id)) return candidate.id;
        }
        return null;
      });
    },
    [groups, hiddenIds]
  );

  // Skips hidden sessions — cycling onto a hidden one would leave the grid
  // showing no active panel for it (its panel doesn't exist), so "next
  // cell" here means "next visible cell", same set TerminalGrid renders.
  const cycleFocusedCell = useCallback(
    (direction) => {
      const group = groups.find((g) => g.name === focusedWorkspace);
      if (!group || group.sessions.length === 0) return;
      const visible = group.sessions.filter((s) => !hiddenIds.has(s.id));
      if (visible.length === 0) return;
      const idx = visible.findIndex((s) => s.id === focusedSessionId);
      const next = (idx + direction + visible.length) % visible.length;
      setFocusedSessionId(visible[next].id);
    },
    [groups, focusedWorkspace, focusedSessionId, hiddenIds]
  );

  const jumpToNeedsInput = useCallback(() => {
    for (const g of groups) {
      const hit = g.sessions.find((s) => s.status === "needs-input");
      if (hit) {
        setFocusedWorkspace(g.name);
        setFocusedSessionId(hit.id);
        // A needs-input session asking for attention should never be
        // stuck hidden — jumping to it un-hides it, same as clicking it in
        // the rail would.
        setHiddenIds((prev) => {
          if (!prev.has(hit.id)) return prev;
          const next = new Set(prev);
          next.delete(hit.id);
          return next;
        });
        return;
      }
    }
  }, [groups]);

  // Clicking rail/header/gutter returns to chrome-navigation mode by
  // blurring whatever terminal currently holds DOM focus (design D-keys:
  // there's no keyboard-only blur in P1, so this is the only way back).
  // Defined ahead of the keydown listener below since review-mode entry
  // (the "r" binding) also calls it.
  const blurActiveTerminal = useCallback(() => {
    const active = document.activeElement;
    if (active && active.closest(".xterm")) active.blur();
  }, []);

  // ---- p2-diff-review: selection, viewed-state, and editor helpers ----

  // Keep the selected file valid as the diff refetches (initial pick, or
  // the previously-selected path disappearing from the list).
  useEffect(() => {
    if (diffFiles.length === 0) {
      setSelectedFilePath(null);
      return;
    }
    if (!diffFiles.some((f) => f.path === selectedFilePath)) {
      setSelectedFilePath(diffFiles[0].path);
    }
  }, [diffFiles, selectedFilePath]);

  // Reload the viewed map whenever the workspace changes or the diff
  // refetches (pruneViewed above already reconciled stale entries).
  useEffect(() => {
    setViewedMap(loadViewedMap(focusedWorkspace));
  }, [focusedWorkspace, diffFiles]);

  const stepSelectedFile = useCallback(
    (direction) => {
      if (diffFiles.length === 0) return;
      const idx = diffFiles.findIndex((f) => f.path === selectedFilePath);
      const next = idx === -1 ? 0 : (idx + direction + diffFiles.length) % diffFiles.length;
      setSelectedFilePath(diffFiles[next].path);
    },
    [diffFiles, selectedFilePath]
  );

  // v (review-mode only): mark the selected file viewed, then advance to
  // the next not-yet-viewed file in rail order; if none remain, selection
  // stays put (spec: "Marking the last unviewed file leaves selection in place").
  const markViewedAndAdvance = useCallback(() => {
    if (!focusedWorkspace || !selectedFilePath) return;
    const idx = diffFiles.findIndex((f) => f.path === selectedFilePath);
    if (idx === -1) return;
    const file = diffFiles[idx];
    const hash = hashContent(file.diff ?? "");
    const nextMap = markViewed(focusedWorkspace, selectedFilePath, hash);
    setViewedMap(nextMap);

    for (let step = 1; step < diffFiles.length; step++) {
      const candidate = diffFiles[(idx + step) % diffFiles.length];
      if (nextMap[candidate.path] !== hashContent(candidate.diff ?? "")) {
        setSelectedFilePath(candidate.path);
        return;
      }
    }
  }, [focusedWorkspace, selectedFilePath, diffFiles]);

  const openSelectedFileInEditor = useCallback(() => {
    if (!focusedWorkspace || !selectedFilePath) return;
    const file = diffFiles.find((f) => f.path === selectedFilePath);
    if (!file) return;
    openEditor(focusedWorkspace, file.path, firstHunkLine(file.diff)).catch((e) =>
      setEditorError(e.message)
    );
  }, [focusedWorkspace, selectedFilePath, diffFiles]);

  const openWorkspaceRoot = useCallback((name) => {
    openEditor(name).catch((e) => setEditorError(e.message));
  }, []);

  // 501/400/404 from open-editor auto-dismiss so the toast doesn't linger.
  useEffect(() => {
    if (!editorError) return;
    const timer = setTimeout(() => setEditorError(null), 5_000);
    return () => clearTimeout(timer);
  }, [editorError]);

  // ---- single window keydown listener (spec: 3.3 / design D-keys, extended per p2 D-keys) ----
  useEffect(() => {
    function onKeyDown(e) {
      if (e.metaKey || e.ctrlKey || e.altKey) return;
      // The one rule design D-keys fixes: bindings are live only when DOM
      // focus is outside any terminal's content.
      if (document.activeElement && document.activeElement.closest(".xterm")) return;

      if (/^[1-9]$/.test(e.key)) {
        const idx = Number(e.key) - 1;
        if (groups[idx]) {
          e.preventDefault();
          selectWorkspace(groups[idx].name);
          // D-dim: workspace-select keybindings target the grid column —
          // selecting a workspace is a precursor to working in its cells.
          setActiveColumn("grid");
        }
        return;
      }
      if (e.key === "]") {
        e.preventDefault();
        cycleFocusedCell(1);
        setActiveColumn("grid");
        return;
      }
      if (e.key === "[") {
        e.preventDefault();
        cycleFocusedCell(-1);
        setActiveColumn("grid");
        return;
      }
      if (e.key === "a") {
        e.preventDefault();
        jumpToNeedsInput();
        setActiveColumn("grid");
        return;
      }
      if (e.key === "Tab") {
        // preventDefault (avoid focus-walk) only when the changes pane is
        // actually visible for it to act on (p2 D-keys).
        if (!reviewMode && !changesPaneCollapsed) {
          e.preventDefault();
          setPaneEmphasis((v) => (v === "list" ? "diff" : "list"));
        }
        // D-dim: Tab/j/k target the pane column whenever it's visible,
        // even if this particular press didn't toggle emphasis (e.g.
        // review mode is open) — they're unambiguously pane-directed keys.
        if (!changesPaneCollapsed) setActiveColumn("pane");
        return;
      }
      if (e.key === "j") {
        e.preventDefault();
        stepSelectedFile(1);
        if (!changesPaneCollapsed) setActiveColumn("pane");
        return;
      }
      if (e.key === "k") {
        e.preventDefault();
        stepSelectedFile(-1);
        if (!changesPaneCollapsed) setActiveColumn("pane");
        return;
      }
      if (e.key === "r") {
        e.preventDefault();
        if (!focusedWorkspace) return;
        // Review-mode entry always refetches unconditionally (design
        // D-freshness trigger 3/3), after blurring any terminal focus so
        // Esc works on the very next keypress (design D-keys).
        blurActiveTerminal();
        fetchDiffForWorkspace(focusedWorkspace);
        setReviewMode(true);
        return;
      }
      if (e.key === "Escape") {
        // Esc only acts when help or review mode is open — never intercept
        // Esc destined for a terminal (the suppression rule above already
        // guarantees no terminal has focus here regardless). design
        // D-help: if both are open, Esc closes the topmost (help) first.
        if (helpOpen) {
          e.preventDefault();
          setHelpOpen(false);
          return;
        }
        if (reviewMode) {
          e.preventDefault();
          setReviewMode(false);
        }
        return;
      }
      if (e.key === "?") {
        // design D-help: suppressed while a terminal has focus by the same
        // top-of-function rule as every other chrome binding.
        e.preventDefault();
        setHelpOpen((v) => !v);
        return;
      }
      if (e.key === "v") {
        // Review-mode only (p2 spec: "j/k navigate files within review mode").
        if (reviewMode) {
          e.preventDefault();
          markViewedAndAdvance();
        }
        return;
      }
      if (e.key === "o") {
        e.preventDefault();
        openSelectedFileInEditor();
      }
    }
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [
    groups,
    selectWorkspace,
    cycleFocusedCell,
    jumpToNeedsInput,
    reviewMode,
    helpOpen,
    changesPaneCollapsed,
    stepSelectedFile,
    markViewedAndAdvance,
    openSelectedFileInEditor,
    focusedWorkspace,
    blurActiveTerminal,
    fetchDiffForWorkspace,
  ]);

  const focusedGroup = groups.find((g) => g.name === focusedWorkspace) ?? null;

  return (
    // P4-WIRE: settings/focus-dim — useSettings() gates a `focus-dim`
    // class here (design D-dim, column semantics: `.focus-dim
    // [data-dim-zone]:not(.dim-exempt):not(.dim-focused) { opacity: .45 }`).
    // The header above is never a zone, so it's exempt from this whole
    // mechanism regardless of `activeColumn`.
    <main
      className={`flex h-screen flex-col bg-garage-bg font-mono text-sm text-garage-ink ${
        settings.focusDim ? "focus-dim" : ""
      }`}
    >
      <header
        onMouseDown={blurActiveTerminal}
        className="flex flex-none items-center gap-3 border-b border-garage-line bg-garage-panel px-4 py-2"
      >
        <span className="font-bold tracking-wide text-garage-amber">claude-garage</span>
        <span className="text-garage-dim">pit wall</span>
        <div className="relative ml-auto flex items-center gap-2">
          <SettingsPopover />
          {/* P4-WIRE: settings/focus-dim — gear button opens the settings
              popover (design D-settings) belongs here, left of "+ add
              workspace". */}
          <button
            type="button"
            onClick={() => setShowAddWorkspace((v) => !v)}
            className="border border-garage-line bg-garage-sel px-2 py-0.5 text-xs text-garage-ink hover:border-garage-amber"
          >
            + add workspace
          </button>
          {showAddWorkspace && (
            <AddWorkspaceForm
              onClose={() => setShowAddWorkspace(false)}
              existingNames={workspaces.map((w) => w.name)}
              onCreated={() => {
                refreshWorkspaces();
                setShowAddWorkspace(false);
              }}
            />
          )}
        </div>
      </header>

      {loadError && (
        <div className="flex-none border-b border-garage-line bg-garage-panel px-4 py-1 text-xs text-garage-red">
          {loadError} — retrying…
        </div>
      )}

      <HooksBanner sessions={sessions} />

      <div
        className={`grid min-h-0 flex-1 ${
          changesPaneCollapsed ? "grid-cols-[240px_1fr_32px]" : "grid-cols-[240px_1fr_360px]"
        }`}
      >
        <WorkspaceRail
          groups={groups}
          focusedWorkspace={focusedWorkspace}
          focusedSessionId={focusedSessionId}
          onSelectWorkspace={selectWorkspace}
          onSelectSession={selectSession}
          onSessionCreated={refreshSessions}
          onOpenRoot={openWorkspaceRoot}
          onBlurChrome={blurActiveTerminal}
          onSessionsRestored={refreshSessions}
          onWorkspaceRenamed={() => {
            refreshWorkspaces();
            refreshSessions();
          }}
          hiddenIds={hiddenIds}
          columnActive={activeColumn === "rail"}
          onActivateColumn={activateRailColumn}
        />
        <TerminalGrid
          group={focusedGroup}
          focusedSessionId={focusedSessionId}
          onFocusCell={setFocusedSessionId}
          onBlurChrome={blurActiveTerminal}
          onSessionsRestored={refreshSessions}
          poppedOutIds={poppedOutIds}
          onPopOut={handlePopOut}
          onReclaim={handleReclaim}
          hiddenIds={hiddenIds}
          onHideCell={hideSession}
          columnActive={activeColumn === "grid"}
          onActivateColumn={activateGridColumn}
        />
        <ChangesPane
          workspace={focusedWorkspace}
          files={diffFiles}
          truncated={diffTruncated}
          loading={diffLoading}
          error={diffError}
          collapsed={changesPaneCollapsed}
          onToggleCollapse={() => setChangesPaneCollapsed((v) => !v)}
          emphasis={paneEmphasis}
          selectedPath={selectedFilePath}
          onSelectFile={setSelectedFilePath}
          onRefresh={() => fetchDiffForWorkspace(focusedWorkspace)}
          onBlurChrome={blurActiveTerminal}
          columnActive={activeColumn === "pane"}
          onActivateColumn={activatePaneColumn}
        />
      </div>

      {reviewMode && (
        <ReviewMode
          workspace={focusedWorkspace}
          files={diffFiles}
          truncated={diffTruncated}
          selectedPath={selectedFilePath}
          onSelectFile={setSelectedFilePath}
          viewedMap={viewedMap}
          onOpenRoot={openWorkspaceRoot}
        />
      )}

      {helpOpen && <HelpOverlay onClose={() => setHelpOpen(false)} />}

      {editorError && (
        <div className="fixed bottom-4 right-4 z-[60] flex max-w-sm items-start gap-2 border border-garage-red bg-garage-panel px-3 py-2 text-xs text-garage-red shadow-lg">
          <span>{editorError}</span>
          <button
            type="button"
            onClick={() => setEditorError(null)}
            className="ml-auto text-garage-dim hover:text-garage-ink"
          >
            ×
          </button>
        </div>
      )}
    </main>
  );
}
