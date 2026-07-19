import React from "react";
import { STATUS_GLYPH, STATUS_COLOR } from "../lib/status.js";

// design D-help: full-screen static overlay listing every keybinding
// across P0-P7, rendered above ReviewMode's own z-50 overlay (z-[70] here)
// so it's reachable even while review mode is open. Opened by "?" and
// closed by "?" or Esc in App.jsx's single keydown listener — this
// component owns no keyboard handling of its own beyond the click-outside
// convenience below.
//
// p7 (spec: pit-wall-ui "Help overlay"): also carries the status-glyph
// legend — the product's core vocabulary was previously never explained
// anywhere in the UI.
const BINDINGS = [
  { keys: "1 – 9", action: "switch focused workspace to the Nth in rail order" },
  { keys: "[ / ]", action: "cycle focused terminal within the current workspace" },
  { keys: "a", action: "jump to a session that needs input (any workspace)" },
  { keys: "\\", action: "split the focused cell right (spawns a new session there)" },
  { keys: "m", action: "maximize the focused cell ⇄ restore the grid" },
  { keys: "Tab", action: "changes pane: toggle list ⇄ diff emphasis" },
  { keys: "j / k", action: "next / previous changed file" },
  { keys: "r", action: "enter full-screen review mode" },
  { keys: "Esc", action: "close this help, else exit review mode" },
  { keys: "v", action: "review mode: mark file viewed, advance to next unviewed" },
  { keys: "o", action: "open selected file — or the workspace root if none — in the editor" },
  { keys: "Ctrl + `", action: "release keys from the focused terminal → back to garage" },
  { keys: "?", action: "toggle this help overlay" },
];

const STATUS_LEGEND = [
  { status: "needs-input", name: "needs input", meaning: "Claude is waiting on you — permission prompt, question, or plan approval" },
  { status: "working", name: "working", meaning: "Claude is running — nothing to do yet" },
  { status: "done", name: "done", meaning: "finished its turn since you last looked at it" },
  { status: "idle", name: "idle", meaning: "waiting for your next prompt" },
  { status: "restorable", name: "restorable", meaning: "tmux session died (e.g. reboot) — the conversation can be resumed" },
];

export default function HelpOverlay({ onClose }) {
  return (
    <div
      className="fixed inset-0 z-[70] flex items-center justify-center bg-garage-bg/90 font-mono text-sm text-garage-ink"
      onMouseDown={(e) => {
        if (e.target === e.currentTarget) onClose();
      }}
    >
      <div className="max-h-[92vh] w-[480px] max-w-[90vw] overflow-y-auto border border-garage-line bg-garage-panel shadow-lg">
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
        <div className="border-t border-garage-line px-3 pb-1 pt-2 text-[10px] uppercase tracking-widest text-garage-faint">
          status legend
        </div>
        <table className="w-full border-collapse text-xs">
          <tbody>
            {STATUS_LEGEND.map((row) => (
              <tr key={row.status} className="border-b border-garage-line last:border-b-0">
                <td className={`w-8 px-3 py-1.5 text-center align-top ${STATUS_COLOR[row.status]}`}>
                  {STATUS_GLYPH[row.status]}
                </td>
                <td className="whitespace-nowrap px-1 py-1.5 align-top font-semibold text-garage-ink">
                  {row.name}
                </td>
                <td className="px-3 py-1.5 text-garage-dim">{row.meaning}</td>
              </tr>
            ))}
          </tbody>
        </table>
      </div>
    </div>
  );
}
