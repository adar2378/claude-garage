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
//
// redesign/light-minimal: bindings grouped under section labels so the
// two-column list reads as a definition list rather than one long table.
const BINDING_GROUPS = [
  {
    label: "workspace",
    items: [
      { keys: "1 – 9", action: "switch focused workspace to the Nth in rail order" },
      { keys: "a", action: "jump to a session that needs input (any workspace)" },
    ],
  },
  {
    label: "grid",
    items: [
      { keys: "[ / ]", action: "cycle focused terminal within the current workspace" },
      { keys: "\\", action: "split the focused cell right (spawns a new session there)" },
      { keys: "m", action: "maximize the focused cell ⇄ restore the grid" },
      { keys: "Ctrl + `", action: "release keys from the focused terminal → back to garage" },
    ],
  },
  {
    label: "review",
    items: [
      { keys: "Tab", action: "changes pane: toggle list ⇄ diff emphasis" },
      { keys: "j / k", action: "next / previous changed file" },
      { keys: "r", action: "enter full-screen review mode" },
      { keys: "v", action: "review mode: mark file viewed, advance to next unviewed" },
      { keys: "o", action: "open selected file — or the workspace root if none — in the editor" },
    ],
  },
  {
    label: "general",
    items: [
      { keys: "?", action: "toggle this help overlay" },
      { keys: "Esc", action: "close this help, else exit review mode" },
    ],
  },
];

const STATUS_LEGEND = [
  { status: "needs-input", name: "needs input", meaning: "Claude is waiting on you — permission prompt, question, or plan approval" },
  { status: "working", name: "running", meaning: "Claude is working — nothing to do yet" },
  { status: "done", name: "responded", meaning: "finished its turn since you last looked at it" },
  { status: "idle", name: "idle", meaning: "waiting for your next prompt" },
  { status: "restorable", name: "restorable", meaning: "tmux session died (e.g. reboot) — the conversation can be resumed" },
];

export default function HelpOverlay({ onClose }) {
  return (
    <div
      className="fixed inset-0 z-[70] flex items-center justify-center bg-black/20"
      onMouseDown={(e) => {
        if (e.target === e.currentTarget) onClose();
      }}
    >
      <div className="max-h-[92vh] w-[560px] max-w-[90vw] overflow-y-auto rounded-xl border border-garage-line bg-garage-bg p-6 shadow-sm">
        <div className="mb-5 flex items-center gap-3">
          <span className="text-[15px] font-semibold text-garage-ink">keybindings</span>
          <span className="text-xs text-garage-dim">? or Esc to close</span>
          <button
            type="button"
            onClick={onClose}
            className="ml-auto rounded-md px-2 py-1 text-[13px] text-garage-dim hover:bg-garage-sel hover:text-garage-ink"
          >
            ×
          </button>
        </div>

        {BINDING_GROUPS.map((group) => (
          <div key={group.label} className="mb-5 last:mb-0">
            <p className="mb-2 text-[11px] uppercase tracking-wider text-garage-faint">{group.label}</p>
            <dl className="grid grid-cols-[auto_1fr] items-baseline gap-x-5 gap-y-2">
              {group.items.map((b) => (
                <React.Fragment key={b.keys}>
                  <dt>
                    <kbd className="font-mono text-[11px] text-garage-dim">{b.keys}</kbd>
                  </dt>
                  <dd className="text-[13px] text-garage-ink">{b.action}</dd>
                </React.Fragment>
              ))}
            </dl>
          </div>
        ))}

        <div className="mt-6 border-t border-garage-line pt-5">
          <p className="mb-2 text-[11px] uppercase tracking-wider text-garage-faint">status legend</p>
          <div className="flex flex-col gap-2.5">
            {STATUS_LEGEND.map((row) => (
              <div key={row.status} className="flex items-start gap-3">
                <span className={`w-4 shrink-0 text-center text-[13px] ${STATUS_COLOR[row.status]}`}>
                  {STATUS_GLYPH[row.status]}
                </span>
                <span className="w-24 shrink-0 text-[13px] font-medium text-garage-ink">{row.name}</span>
                <span className="text-xs text-garage-dim">{row.meaning}</span>
              </div>
            ))}
          </div>
        </div>
      </div>
    </div>
  );
}
