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
        <div className="flex flex-none items-center gap-2 border-b border-garage-line px-2 py-2 text-xs">
          <span className="truncate font-semibold text-garage-amber">{workspace}</span>
          <button
            type="button"
            onClick={() => onOpenRoot(workspace)}
            title="open workspace root in editor"
            className="ml-auto shrink-0 text-garage-dim hover:text-garage-amber"
          >
            ⧉
          </button>
        </div>
        <div className="flex-none border-b border-garage-line px-2 py-1 text-[11px] text-garage-dim">
          {viewedCount}/{files.length} viewed
          {truncated && <span className="ml-2 text-garage-amber">(response truncated)</span>}
        </div>
        <div className="min-h-0 flex-1 overflow-y-auto">
          {files.length === 0 && <p className="px-2 py-2 text-[11px] text-garage-dim">no changes</p>}
          {files.map((f) => {
            const viewed = isFileViewed(viewedMap, f);
            const selected = f.path === selectedPath;
            return (
              <button
                key={f.path}
                type="button"
                onClick={() => onSelectFile(f.path)}
                className={`flex w-full items-center gap-2 px-2 py-1 text-left text-[11px] ${
                  selected ? "bg-garage-sel" : "hover:bg-garage-sel"
                }`}
              >
                <span className={viewed ? "text-garage-green" : "text-garage-faint"}>
                  {viewed ? "✓" : "·"}
                </span>
                <span className="min-w-0 flex-1 truncate">{f.path}</span>
              </button>
            );
          })}
        </div>
        <div className="flex-none border-t border-garage-line px-2 py-1 text-[10px] text-garage-faint">
          j/k files · v viewed · o editor · Esc exit
        </div>
      </div>

      <div className="min-h-0 flex-1 overflow-y-auto">
        <DiffList files={files} selectedPath={selectedPath} emptyMessage="no changes to review" />
      </div>
    </div>
  );
}
