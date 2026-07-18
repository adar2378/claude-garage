import React from "react";
import DiffList from "./DiffList.jsx";

const CHANGE_GLYPH = {
  modified: "M",
  added: "A",
  deleted: "D",
  renamed: "R",
  untracked: "?",
};

const CHANGE_COLOR = {
  modified: "text-garage-blue",
  added: "text-garage-green",
  deleted: "text-garage-red",
  renamed: "text-garage-blue",
  untracked: "text-garage-dim",
};

// Third grid column (design D-layout, spec: diff-review-ui "Changes pane").
// Collapsed to a thin toggle strip via `collapsed`; App.jsx sizes the grid
// track to match (32px / 360px) — this component just fills whatever width
// the grid gives it.
export default function ChangesPane({
  workspace,
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
}) {
  if (collapsed) {
    return (
      <button
        type="button"
        onClick={onToggleCollapse}
        title="expand changes pane"
        aria-label="expand changes pane"
        className="flex h-full w-full flex-col items-center gap-2 border-l border-garage-line bg-garage-panel py-2 text-garage-dim hover:text-garage-amber"
      >
        <span aria-hidden="true">«</span>
        <span className="[writing-mode:vertical-rl]">changes</span>
      </button>
    );
  }

  return (
    <div
      onMouseDown={onBlurChrome}
      className="flex h-full min-h-0 flex-col border-l border-garage-line bg-garage-panel"
    >
      <div className="flex flex-none items-center gap-2 border-b border-garage-line px-2 py-1 text-xs">
        <span className="truncate font-semibold text-garage-ink">{workspace ?? "—"}</span>
        <button
          type="button"
          onClick={onRefresh}
          title="refresh diff"
          className="ml-auto text-garage-dim hover:text-garage-amber"
        >
          ↻
        </button>
        <button
          type="button"
          onClick={onToggleCollapse}
          title="collapse changes pane"
          className="text-garage-dim hover:text-garage-amber"
        >
          »
        </button>
      </div>

      {error && (
        <div className="flex-none border-b border-garage-line px-2 py-1 text-[11px] text-garage-red">
          {error}
        </div>
      )}
      {truncated && (
        <div className="flex-none border-b border-garage-line bg-garage-sel px-2 py-1 text-[11px] text-garage-amber">
          response truncated — some files show stats only
        </div>
      )}

      <div
        className={`min-h-0 overflow-y-auto border-b border-garage-line ${
          emphasis === "list" ? "flex-[2]" : "flex-1"
        }`}
      >
        {loading && files.length === 0 && (
          <p className="px-2 py-2 text-[11px] text-garage-dim">loading…</p>
        )}
        {!loading && files.length === 0 && (
          <p className="px-2 py-2 text-[11px] text-garage-dim">no changes</p>
        )}
        {files.map((f) => (
          <button
            key={f.path}
            type="button"
            onClick={() => onSelectFile(f.path)}
            className={`flex w-full items-center gap-2 px-2 py-1 text-left text-[11px] ${
              f.path === selectedPath ? "bg-garage-sel" : "hover:bg-garage-sel"
            }`}
          >
            <span className={CHANGE_COLOR[f.changeType] ?? "text-garage-dim"}>
              {CHANGE_GLYPH[f.changeType] ?? "?"}
            </span>
            <span className="min-w-0 flex-1 truncate">{f.path}</span>
            {f.binary && <span className="shrink-0 text-garage-faint">bin</span>}
            {f.truncated && <span className="shrink-0 text-garage-amber">trunc</span>}
            {!f.binary && (
              <span className="shrink-0 text-garage-faint">
                <span className="text-garage-green">+{f.additions ?? 0}</span>{" "}
                <span className="text-garage-red">−{f.deletions ?? 0}</span>
              </span>
            )}
          </button>
        ))}
      </div>

      <div className={`min-h-0 overflow-y-auto ${emphasis === "diff" ? "flex-[2]" : "flex-1"}`}>
        <DiffList files={files} selectedPath={selectedPath} />
      </div>
    </div>
  );
}
