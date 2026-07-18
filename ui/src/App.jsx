import React, { useCallback, useEffect, useMemo, useRef, useState } from "react";
import WorkspaceRail from "./components/WorkspaceRail.jsx";
import TerminalGrid from "./components/TerminalGrid.jsx";
import HooksBanner from "./components/HooksBanner.jsx";
import AddWorkspaceForm from "./components/AddWorkspaceForm.jsx";
import ChangesPane from "./components/ChangesPane.jsx";
import ReviewMode from "./components/ReviewMode.jsx";
import { fetchSessions, fetchWorkspaces, reportVisibility, fetchDiff, openEditor } from "./lib/api.js";
import { buildGroups } from "./lib/groups.js";
import { loadViewedMap, markViewed, pruneViewed, hashContent } from "./lib/viewed.js";
import { firstHunkLine } from "./lib/diff.js";

// Kept for the page's lifetime (module scope, not persisted) — see API
// contract for POST /api/ui/visibility.
const CLIENT_ID = crypto.randomUUID();
const VISIBILITY_INTERVAL_MS = 30_000;
const LOAD_RETRY_MS = 3_000;

export default function App() {
  const [workspaces, setWorkspaces] = useState([]);
  const [sessions, setSessions] = useState([]);
  const [loadError, setLoadError] = useState(null);
  const [focusedWorkspace, setFocusedWorkspace] = useState(null);
  const [focusedSessionId, setFocusedSessionId] = useState(null);
  const [showAddWorkspace, setShowAddWorkspace] = useState(false);

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

  const groups = useMemo(() => buildGroups(workspaces, sessions), [workspaces, sessions]);

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

  useEffect(() => {
    const group = groups.find((g) => g.name === focusedWorkspace);
    if (!group) return;
    if (!group.sessions.some((s) => s.id === focusedSessionId)) {
      setFocusedSessionId(group.sessions[0]?.id ?? null);
    }
  }, [groups, focusedWorkspace, focusedSessionId]);

  const selectWorkspace = useCallback((name) => {
    setFocusedWorkspace(name);
  }, []);

  const selectSession = useCallback((workspaceName, sessionId) => {
    setFocusedWorkspace(workspaceName);
    setFocusedSessionId(sessionId);
  }, []);

  const cycleFocusedCell = useCallback(
    (direction) => {
      const group = groups.find((g) => g.name === focusedWorkspace);
      if (!group || group.sessions.length === 0) return;
      const idx = group.sessions.findIndex((s) => s.id === focusedSessionId);
      const next = (idx + direction + group.sessions.length) % group.sessions.length;
      setFocusedSessionId(group.sessions[next].id);
    },
    [groups, focusedWorkspace, focusedSessionId]
  );

  const jumpToNeedsInput = useCallback(() => {
    for (const g of groups) {
      const hit = g.sessions.find((s) => s.status === "needs-input");
      if (hit) {
        setFocusedWorkspace(g.name);
        setFocusedSessionId(hit.id);
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
        }
        return;
      }
      if (e.key === "]") {
        e.preventDefault();
        cycleFocusedCell(1);
        return;
      }
      if (e.key === "[") {
        e.preventDefault();
        cycleFocusedCell(-1);
        return;
      }
      if (e.key === "a") {
        e.preventDefault();
        jumpToNeedsInput();
        return;
      }
      if (e.key === "Tab") {
        // preventDefault (avoid focus-walk) only when the changes pane is
        // actually visible for it to act on (p2 D-keys).
        if (!reviewMode && !changesPaneCollapsed) {
          e.preventDefault();
          setPaneEmphasis((v) => (v === "list" ? "diff" : "list"));
        }
        return;
      }
      if (e.key === "j") {
        e.preventDefault();
        stepSelectedFile(1);
        return;
      }
      if (e.key === "k") {
        e.preventDefault();
        stepSelectedFile(-1);
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
        // Esc only acts when review mode is open — never intercept Esc
        // destined for a terminal (the suppression rule above already
        // guarantees no terminal has focus here regardless).
        if (reviewMode) {
          e.preventDefault();
          setReviewMode(false);
        }
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
    <main className="flex h-screen flex-col bg-garage-bg font-mono text-sm text-garage-ink">
      <header
        onMouseDown={blurActiveTerminal}
        className="flex flex-none items-center gap-3 border-b border-garage-line bg-garage-panel px-4 py-2"
      >
        <span className="font-bold tracking-wide text-garage-amber">claude-garage</span>
        <span className="text-garage-dim">pit wall</span>
        <div className="relative ml-auto">
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
        />
        <TerminalGrid
          group={focusedGroup}
          focusedSessionId={focusedSessionId}
          onFocusCell={setFocusedSessionId}
          onBlurChrome={blurActiveTerminal}
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
