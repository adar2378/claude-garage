// Status vocabulary shared by the rail and the grid.
// Kept in one place so glyph/color/label choices never drift between them.
//
// redesign/light-minimal: the salience ladder from DESIGN-CONTRACT.md
// section 5 — needs-input is loud (the only status that gets amber),
// everything else is quiet grey. `done`/diff additions keep green.
// `working` is deliberately colourless (garage-dim): it's the
// "ignore me, nothing needs you" state and doesn't deserve a hue.

export const STATUS_GLYPH = {
  "needs-input": "●",
  working: "●",
  done: "●",
  idle: "○",
  restorable: "○",
};

export const STATUS_COLOR = {
  "needs-input": "text-garage-amber",
  done: "text-garage-green",
  working: "text-garage-dim",
  idle: "text-garage-faint",
  restorable: "text-garage-faint",
};

export const STATUS_LABEL = {
  "needs-input": "needs input",
  done: "responded",
  working: "running",
  idle: "idle",
  restorable: "restorable",
};

// Sort weight — lower is louder. Lists that want attention first should
// sort by this (needs-input surfaces above everything else).
export const SALIENCE = {
  "needs-input": 0,
  done: 1,
  working: 2,
  idle: 3,
  restorable: 4,
};

// p7 (spec: pit-wall-ui "Help overlay" / rail tooltips): one-line meaning
// per status, used as the tooltip on rail glyphs and echoed by the help
// overlay's legend.
export const STATUS_TIP = {
  "needs-input": "needs input — Claude is waiting on you",
  done: "responded — finished since you last looked",
  working: "running — Claude is working",
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

export function labelFor(status) {
  return STATUS_LABEL[status] ?? STATUS_LABEL.idle;
}

export function salienceOf(status) {
  return SALIENCE[status] ?? 9;
}
