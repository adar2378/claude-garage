// Status vocabulary shared by the rail and the grid.
// Kept in one place so glyph/color choices never drift between them.

export const STATUS_GLYPH = {
  "needs-input": "●",
  working: "◐",
  done: "✓",
  idle: "○",
  restorable: "⟳",
};

export const STATUS_COLOR = {
  "needs-input": "text-garage-amber",
  working: "text-garage-blue",
  done: "text-garage-green",
  idle: "text-garage-dim",
  restorable: "text-garage-dim",
};

// p7 (spec: pit-wall-ui "Help overlay" / rail tooltips): one-line meaning
// per status, used as the tooltip on rail glyphs and echoed by the help
// overlay's legend.
export const STATUS_TIP = {
  "needs-input": "needs input — Claude is waiting on you",
  working: "working — Claude is running",
  done: "done — finished since you last looked",
  idle: "idle — waiting for your next prompt",
  restorable: "restorable — tmux died, the conversation can be resumed",
};

export function glyphFor(status) {
  return STATUS_GLYPH[status] ?? STATUS_GLYPH.idle;
}

export function tipFor(status) {
  return STATUS_TIP[status] ?? STATUS_TIP.idle;
}

export function colorFor(status) {
  return STATUS_COLOR[status] ?? STATUS_COLOR.idle;
}
