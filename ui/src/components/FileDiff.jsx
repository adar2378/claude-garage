import React from "react";
import { parseFileDiff } from "../lib/diff.js";

// Single-file unified diff renderer (design D-diff-render). The one shared
// component both ChangesPane and ReviewMode use — no second rendering path.
// redesign/light-minimal: on a white ground the add/delete signal moved
// from coloured TEXT to a pale background wash with near-black text —
// coloured text on white was either too pale to read or too saturated to
// sit beside the rest of the UI. Tokens: add/del rows get
// bg-garage-green/10 | bg-garage-red/10 with text-garage-ink, context is
// text-garage-dim, hunk headers sit on bg-garage-sel, line numbers are
// text-garage-faint. Mono, text-xs.
export default function FileDiff({ file }) {
  if (!file) return null;

  if (file.binary) {
    return <div className="rounded-lg border border-garage-line px-3 py-2 text-xs text-garage-dim">binary file</div>;
  }

  const hunks = parseFileDiff(file.diff);

  return (
    <div className="overflow-hidden rounded-lg border border-garage-line font-mono text-xs leading-relaxed">
      {file.truncated && (
        <div className="border-b border-garage-line bg-garage-sel px-3 py-1.5 text-garage-dim">
          diff truncated — press "o" to open the full file in your editor
        </div>
      )}
      {hunks.length === 0 && (
        <div className="px-3 py-2 text-garage-dim">no diff content</div>
      )}
      {hunks.map((hunk, hi) => (
        <div key={hi}>
          <div className="bg-garage-sel px-3 py-1.5 text-[11px] text-garage-dim">{hunk.header}</div>
          {hunk.lines.map((line, li) => (
            <div key={li} className={`flex px-3 ${lineClass(line.type)}`}>
              <span className="mr-3 w-8 shrink-0 select-none text-right tabular-nums text-garage-faint">
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

// Pale washes, not saturated fills — must stay legible on a white ground.
// The leading +/- character (already part of `line.content`) stays visible
// so the add/delete signal never relies on colour alone.
function lineClass(type) {
  if (type === "add") return "bg-garage-green/10 text-garage-ink";
  if (type === "del") return "bg-garage-red/10 text-garage-ink";
  return "text-garage-dim";
}
