import React, { useCallback, useEffect, useMemo, useState } from "react";
import WorkspaceRail from "./components/WorkspaceRail.jsx";
import TerminalGrid from "./components/TerminalGrid.jsx";
import HooksBanner from "./components/HooksBanner.jsx";
import AddWorkspaceForm from "./components/AddWorkspaceForm.jsx";
import { fetchSessions, fetchWorkspaces, reportVisibility } from "./lib/api.js";
import { buildGroups } from "./lib/groups.js";

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
      setSessions((prev) => prev.map((s) => (s.id === id ? { ...s, status } : s)));
    });

    es.addEventListener("sessions", refreshSessions);

    return () => es.close();
  }, [refreshSessions]);

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

  // ---- single window keydown listener (spec: 3.3 / design D-keys) ----
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
      }
    }
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [groups, selectWorkspace, cycleFocusedCell, jumpToNeedsInput]);

  // Clicking rail/header/gutter returns to chrome-navigation mode by
  // blurring whatever terminal currently holds DOM focus (design D-keys:
  // there's no keyboard-only blur in P1, so this is the only way back).
  const blurActiveTerminal = useCallback(() => {
    const active = document.activeElement;
    if (active && active.closest(".xterm")) active.blur();
  }, []);

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

      <div className="grid min-h-0 flex-1 grid-cols-[240px_1fr]">
        <WorkspaceRail
          groups={groups}
          focusedWorkspace={focusedWorkspace}
          focusedSessionId={focusedSessionId}
          onSelectWorkspace={selectWorkspace}
          onSelectSession={selectSession}
          onSessionCreated={refreshSessions}
          onBlurChrome={blurActiveTerminal}
        />
        <TerminalGrid
          group={focusedGroup}
          focusedSessionId={focusedSessionId}
          onFocusCell={setFocusedSessionId}
          onBlurChrome={blurActiveTerminal}
        />
      </div>
    </main>
  );
}
