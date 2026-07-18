// Status vocabulary shared by the rail and the grid.
// Kept in one place so glyph/color choices never drift between them.

export const STATUS_GLYPH = {
  "needs-input": "●",
  working: "◐",
  done: "✓",
  idle: "○",
};

export const STATUS_COLOR = {
  "needs-input": "text-garage-amber",
  working: "text-garage-blue",
  done: "text-garage-green",
  idle: "text-garage-dim",
};

export function glyphFor(status) {
  return STATUS_GLYPH[status] ?? STATUS_GLYPH.idle;
}

export function colorFor(status) {
  return STATUS_COLOR[status] ?? STATUS_COLOR.idle;
}
