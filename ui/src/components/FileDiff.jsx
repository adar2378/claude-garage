import React from "react";
import { parseFileDiff } from "../lib/diff.js";

// Single-file unified diff renderer (design D-diff-render). The one shared
// component both ChangesPane and ReviewMode use — no second rendering path.
// Tokens: additions text-garage-green (with a faint bg tint), deletions
// text-garage-red, hunk headers text-garage-dim, context text-garage-ink,
// line numbers text-garage-faint. Mono, text-xs.
export default function FileDiff({ file }) {
  if (!file) return null;

  if (file.binary) {
    return <div className="px-3 py-2 text-xs text-garage-dim">binary file</div>;
  }

  const hunks = parseFileDiff(file.diff);

  return (
    <div className="font-mono text-xs">
      {file.truncated && (
        <div className="border-b border-garage-line bg-garage-sel px-3 py-1 text-garage-amber">
          diff truncated — press "o" to open the full file in your editor
        </div>
      )}
      {hunks.length === 0 && (
        <div className="px-3 py-2 text-garage-dim">no diff content</div>
      )}
      {hunks.map((hunk, hi) => (
        <div key={hi}>
          <div className="px-3 py-1 text-garage-dim">{hunk.header}</div>
          {hunk.lines.map((line, li) => (
            <div key={li} className={`flex px-3 ${lineClass(line.type)}`}>
              <span className="mr-3 w-8 shrink-0 select-none text-right text-garage-faint">
                {line.lineNumber ?? ""}
              </span>
              <span className="whitespace-pre-wrap break-all">{line.content}</span>
            </div>
          ))}
        </div>
      ))}
    </div>
  );
}

function lineClass(type) {
  if (type === "add") return "bg-garage-green/10 text-garage-green";
  if (type === "del") return "text-garage-red";
  return "text-garage-ink";
}
