import React, { useCallback, useEffect, useRef, useState } from "react";
import DiffList from "./DiffList.jsx";
import { loadPaneSizes, savePaneSizes, startDrag } from "../lib/panes.js";

const CHANGE_GLYPH = {
  modified: "M",
  added: "A",
  deleted: "D",
  renamed: "R",
  untracked: "?",
};

// Third grid column (design D-layout, spec: diff-review-ui "Changes pane").
// Collapsed to a thin toggle strip via `collapsed`; App.jsx sizes the grid
// track to match (32px / 360px) — this component just fills whatever width
// the grid gives it.
//
// D-dim (column semantics): unlike the rail/grid (many small zones), the
// whole pane is a single dim zone — one `data-dim-zone` on the outer
// container, `dim-focused` whenever the pane is the `activeColumn`
// (`columnActive`). No needs-input exemption inside: that signal lives on
// rail rows / grid cells, not diff content.
//
// p6-drag-resize: the list/diff vertical split is `paneSplit`, local state
// initialized from localStorage (lib/panes.js) — App.jsx no longer owns the
// split itself. `emphasis` ("list" | "diff", still owned by App and toggled
// by the Tab key) is now just a *preset trigger*: whenever it changes, an
// effect snaps `paneSplit` to a matching preset (0.65 / 0.25) and persists
// it. A manual drag of the row divider overrides that split immediately and
// sticks until the next Tab press changes `emphasis` again.
export default function ChangesPane({
  workspace,
  branch,
  files,
  truncated,
  loading,
  error,
  collapsed,
  onToggleCollapse,
  emphasis,
  selectedPath,
  onSelectFile,
  onRefresh,
  onBlurChrome,
  columnActive,
  onActivateColumn,
  onEnterReview,
}) {
  const [paneSplit, setPaneSplit] = useState(() => loadPaneSizes().paneSplit);
  const paneSplitRef = useRef(paneSplit);
  useEffect(() => {
    paneSplitRef.current = paneSplit;
  }, [paneSplit]);

  // Skip the very first run so mounting doesn't discard a persisted custom
  // split just because App's initial `emphasis` happens to be "list" —
  // only actual *changes* to emphasis (a Tab press) should snap the split.
  const isFirstEmphasisRun = useRef(true);
  useEffect(() => {
    if (isFirstEmphasisRun.current) {
      isFirstEmphasisRun.current = false;
      return;
    }
    const preset = emphasis === "diff" ? 0.25 : 0.65;
    setPaneSplit(preset);
    savePaneSizes({ paneSplit: preset });
  }, [emphasis]);

  const splitContainerRef = useRef(null);
  const [isDraggingSplit, setIsDraggingSplit] = useState(false);

  const handleSplitMouseDown = useCallback((e) => {
    e.stopPropagation();
    const containerHeight = splitContainerRef.current?.getBoundingClientRect().height || 1;
    const startSplit = paneSplitRef.current;
    setIsDraggingSplit(true);
    // Drag-closure latest, not the React-synced ref — a same-frame mouseup
    // would persist one render stale (see App.jsx divider handlers).
    let latest = startSplit;
    startDrag(e, {
      onMove: (_dx, dy) => {
        latest = Math.min(0.85, Math.max(0.15, startSplit + dy / containerHeight));
        setPaneSplit(latest);
      },
      onEnd: () => {
        setIsDraggingSplit(false);
        savePaneSizes({ paneSplit: latest });
      },
    });
  }, []);

  if (collapsed) {
    return (
      <button
        type="button"
        onClick={onToggleCollapse}
        onMouseDownCapture={onActivateColumn}
        title="expand changes pane"
        aria-label="expand changes pane"
        className="flex h-full w-full flex-col items-center gap-2 border-l border-garage-line bg-garage-panel py-2 text-garage-dim hover:bg-garage-sel hover:text-garage-ink"
      >
        <span aria-hidden="true">«</span>
        <span className="[writing-mode:vertical-rl]">changes</span>
      </button>
    );
  }

  return (
    <div
      onMouseDown={onBlurChrome}
      onMouseDownCapture={onActivateColumn}
      data-dim-zone
      className={`flex h-full min-h-0 flex-col border-l border-garage-line bg-garage-panel ${
        columnActive ? "dim-focused" : ""
      }`}
    >
      <div className="flex h-10 flex-none items-center gap-2 border-b border-garage-line px-3">
        <span className="truncate text-[13px] font-medium text-garage-ink">{workspace ?? "—"}</span>
        {branch && (
          // design D-wt-diff: the diff root is overridden to a worktree
          // session's cwd — the response carries `branch` in that case
          // only, so this only ever renders when review is actually
          // scoped to a worktree rather than the workspace root.
          <span className="shrink-0 truncate font-mono text-[11px] text-garage-faint" title={branch}>
            ⎇ {branch}
          </span>
        )}
        {/* p7 (spec: "Visible review-mode entry"): review mode was
            keyboard-only (`r`) — one of the product's four pillars with
            zero mouse discoverability. Same entry path as the binding. */}
        <button
          type="button"
          onClick={onEnterReview}
          title="full-screen review mode (r)"
          className="ml-auto rounded-md px-2.5 py-1 text-xs text-garage-dim hover:bg-garage-sel hover:text-garage-ink"
        >
          review
        </button>
        <button
          type="button"
          onClick={onRefresh}
          title="refresh diff"
          className="rounded-md px-2.5 py-1 text-xs text-garage-dim hover:bg-garage-sel hover:text-garage-ink"
        >
          ↻
        </button>
        <button
          type="button"
          onClick={onToggleCollapse}
          title="collapse changes pane"
          className="rounded-md px-2.5 py-1 text-xs text-garage-dim hover:bg-garage-sel hover:text-garage-ink"
        >
          »
        </button>
      </div>

      {error && (
        <div className="flex-none border-b border-garage-line px-3 py-1 text-[11px] text-garage-red">
          {error}
        </div>
      )}
      {truncated && (
        <div className="flex-none border-b border-garage-line bg-garage-sel px-3 py-1 text-[11px] text-garage-dim">
          response truncated — some files show stats only
        </div>
      )}

      <div ref={splitContainerRef} className="flex min-h-0 flex-1 flex-col">
        <div
          className="min-h-0 overflow-y-auto"
          style={{ flexBasis: `${paneSplit * 100}%`, flexGrow: 0, flexShrink: 0 }}
        >
          {loading && files.length === 0 && (
            <p className="px-3 py-2 text-xs text-garage-dim">loading…</p>
          )}
          {!loading && files.length === 0 && (
            <p className="px-3 py-2 text-xs text-garage-dim">no changes</p>
          )}
          {files.map((f) => (
            <button
              key={f.path}
              type="button"
              onClick={() => onSelectFile(f.path)}
              className={`mx-1.5 flex w-[calc(100%-0.75rem)] items-center gap-2 rounded-md px-3 py-1.5 text-left text-[13px] ${
                f.path === selectedPath ? "bg-garage-sel" : "hover:bg-garage-sel"
              }`}
            >
              <span className="font-mono text-[11px] text-garage-faint">
                {CHANGE_GLYPH[f.changeType] ?? "?"}
              </span>
              <span className="min-w-0 flex-1 truncate font-mono text-xs text-garage-dim">{f.path}</span>
              {f.binary && <span className="shrink-0 text-xs text-garage-faint">bin</span>}
              {f.truncated && <span className="shrink-0 text-xs text-garage-dim">trunc</span>}
              {!f.binary && (
                <span className="shrink-0 font-mono text-xs tabular-nums">
                  <span className="text-garage-green">+{f.additions ?? 0}</span>{" "}
                  <span className="text-garage-red">−{f.deletions ?? 0}</span>
                </span>
              )}
            </button>
          ))}
        </div>

        {/* p6-drag-resize: not a dim-zone. stopPropagation only stops the
            bubble-phase (keeps onBlurChrome on the outer container from
            firing on every drag-start) — the outer container's
            onMouseDownCapture still runs first regardless, same as it
            would for any other mousedown inside the pane. */}
        <div
          role="separator"
          aria-orientation="horizontal"
          className={`row-divider ${isDraggingSplit ? "is-dragging" : ""}`}
          onMouseDown={handleSplitMouseDown}
        />

        <div className="min-h-0 flex-1 overflow-y-auto">
          <DiffList files={files} selectedPath={selectedPath} />
        </div>
      </div>
    </div>
  );
}
