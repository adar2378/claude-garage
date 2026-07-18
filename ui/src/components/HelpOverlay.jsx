import React from "react";

// design D-help: full-screen static overlay listing every keybinding
// across P0-P3, rendered above ReviewMode's own z-50 overlay (z-[70] here)
// so it's reachable even while review mode is open. Opened by "?" and
// closed by "?" or Esc in App.jsx's single keydown listener — this
// component owns no keyboard handling of its own beyond the click-outside
// convenience below.
const BINDINGS = [
  { keys: "1 – 9", action: "switch focused workspace to the Nth in rail order" },
  { keys: "[ / ]", action: "cycle focused terminal within the current workspace" },
  { keys: "a", action: "jump to a session that needs input (any workspace)" },
  { keys: "Tab", action: "changes pane: toggle list ⇄ diff emphasis" },
  { keys: "j / k", action: "next / previous changed file" },
  { keys: "r", action: "enter full-screen review mode" },
  { keys: "Esc", action: "close this help, else exit review mode" },
  { keys: "v", action: "review mode: mark file viewed, advance to next unviewed" },
  { keys: "o", action: "open selected file (or workspace root) in the editor" },
  { keys: "Ctrl + `", action: "blur the focused terminal → chrome-navigation mode" },
  { keys: "?", action: "toggle this help overlay" },
];

export default function HelpOverlay({ onClose }) {
  return (
    <div
      className="fixed inset-0 z-[70] flex items-center justify-center bg-garage-bg/90 font-mono text-sm text-garage-ink"
      onMouseDown={(e) => {
        if (e.target === e.currentTarget) onClose();
      }}
    >
      <div className="w-[440px] max-w-[90vw] border border-garage-line bg-garage-panel shadow-lg">
        <div className="flex items-center gap-2 border-b border-garage-line px-3 py-2">
          <span className="font-semibold tracking-wide text-garage-amber">keybindings</span>
          <span className="text-garage-dim">? or Esc to close</span>
          <button
            type="button"
            onClick={onClose}
            className="ml-auto text-garage-dim hover:text-garage-ink"
          >
            ×
          </button>
        </div>
        <table className="w-full border-collapse text-xs">
          <tbody>
            {BINDINGS.map((b) => (
              <tr key={b.keys} className="border-b border-garage-line last:border-b-0">
                <td className="whitespace-nowrap px-3 py-1.5 align-top font-semibold text-garage-amber">
                  {b.keys}
                </td>
                <td className="px-3 py-1.5 text-garage-ink">{b.action}</td>
              </tr>
            ))}
          </tbody>
        </table>
      </div>
    </div>
  );
}
