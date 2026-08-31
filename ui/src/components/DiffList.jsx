import React, { useEffect, useRef } from "react";
import FileDiff from "./FileDiff.jsx";

// Continuous, per-file-sectioned diff — shared by ChangesPane (inline) and
// ReviewMode (full width). Selecting a file (j/k, or clicking its row)
// scrolls that file's section into view rather than swapping content, so
// both surfaces behave the same way (design D-diff-render: one renderer,
// two layouts).
export default function DiffList({ files, selectedPath, emptyMessage = "no changes" }) {
  const refs = useRef({});

  useEffect(() => {
    const el = refs.current[selectedPath];
    if (el) el.scrollIntoView({ block: "nearest" });
  }, [selectedPath]);

  if (files.length === 0) {
    return <p className="px-3 py-2 text-xs text-garage-dim">{emptyMessage}</p>;
  }

  return (
    <div>
      {files.map((f) => (
        <div
          key={f.path}
          ref={(el) => {
            refs.current[f.path] = el;
          }}
        >
          <div
            className={`sticky top-0 z-10 truncate border-b border-t border-garage-line bg-garage-panel px-3 py-1.5 font-mono text-xs ${
              f.path === selectedPath ? "text-garage-ink" : "text-garage-dim"
            }`}
          >
            {f.renamedFrom || f.oldPath ? `${f.oldPath ?? f.renamedFrom} → ${f.path}` : f.path}
          </div>
          <FileDiff file={f} />
        </div>
      ))}
    </div>
  );
}
