import React from "react";
import DiffList from "./DiffList.jsx";
import { isFileViewed } from "../lib/viewed.js";

// Full-screen overlay (design D-layout): position:fixed inset-0, z-50,
// mounted alongside the grid rather than replacing it — the grid/terminals
// underneath stay mounted and streaming, only hidden behind this layer.
export default function ReviewMode({
  workspace,
  files,
  truncated,
  selectedPath,
  onSelectFile,
  viewedMap,
  onOpenRoot,
}) {
  const viewedCount = files.filter((f) => isFileViewed(viewedMap, f)).length;

  return (
    <div className="fixed inset-0 z-50 flex bg-garage-bg font-mono text-sm text-garage-ink">
      <div className="flex w-[280px] flex-none flex-col border-r border-garage-line bg-garage-panel">
        <div className="flex h-12 flex-none items-center gap-2 border-b border-garage-line px-4">
          <span className="truncate text-[15px] font-semibold text-garage-ink">{workspace}</span>
          <button
            type="button"
            onClick={() => onOpenRoot(workspace)}
            title="open workspace root in editor"
            className="ml-auto shrink-0 rounded-md px-2 py-1 text-garage-dim hover:bg-garage-sel hover:text-garage-ink"
          >
            ⧉
          </button>
        </div>
        <div className="flex-none border-b border-garage-line px-3 py-1.5 text-xs text-garage-dim">
          {viewedCount}/{files.length} viewed
          {truncated && <span className="ml-2 text-garage-dim">(response truncated)</span>}
        </div>
        <div className="min-h-0 flex-1 overflow-y-auto">
          {files.length === 0 && <p className="px-3 py-2 text-xs text-garage-dim">no changes</p>}
          {files.map((f) => {
            const viewed = isFileViewed(viewedMap, f);
            const selected = f.path === selectedPath;
            return (
              <button
                key={f.path}
                type="button"
                onClick={() => onSelectFile(f.path)}
                className={`mx-1.5 flex w-[calc(100%-0.75rem)] items-center gap-2 rounded-md px-3 py-1.5 text-left text-[13px] ${
                  selected ? "bg-garage-sel" : "hover:bg-garage-sel"
                } ${viewed ? "opacity-60" : ""}`}
              >
                <span className={viewed ? "text-garage-green" : "text-garage-faint"}>
                  {viewed ? "✓" : "·"}
                </span>
                <span className="min-w-0 flex-1 truncate font-mono text-xs text-garage-dim">{f.path}</span>
              </button>
            );
          })}
        </div>
        <div className="flex-none border-t border-garage-line px-3 py-1.5 font-mono text-[11px] text-garage-dim">
          j/k files · v viewed · o editor · Esc exit
        </div>
      </div>

      <div className="min-h-0 flex-1 overflow-y-auto">
        <DiffList files={files} selectedPath={selectedPath} emptyMessage="no changes to review" />
      </div>
    </div>
  );
}
