import { test } from "node:test";
import assert from "node:assert/strict";
import { buildDefault, reconcile, registerPlacementHint } from "../src/lib/layout.js";

// Minimal fake DockviewApi: records addPanel calls, tracks the panel set.
function fakeApi(existing = []) {
  const panels = existing.map((p) => ({
    id: p.id,
    api: { width: p.w ?? 100, height: p.h ?? 100 },
  }));
  const added = [];
  return {
    panels,
    added,
    getPanel(id) {
      return panels.find((p) => p.id === id) ?? null;
    },
    addPanel({ id, position }) {
      added.push({ id, position });
      panels.push({ id, api: { width: 100, height: 100 } });
    },
    removePanel(panel) {
      const i = panels.findIndex((p) => p.id === panel.id);
      if (i !== -1) panels.splice(i, 1);
    },
  };
}

test("buildDefault: 4 sessions form a true 2x2 (below the COLUMN neighbor)", () => {
  const api = fakeApi();
  buildDefault(api, ["a", "b", "c", "d"]);
  assert.deepEqual(api.added, [
    { id: "a", position: undefined },
    { id: "b", position: { referencePanel: "a", direction: "right" } },
    // the p7 e2e bug: c/d must dock below their column neighbor, never
    // "right of the previous" (which subdivides the wrong cell)
    { id: "c", position: { referencePanel: "a", direction: "below" } },
    { id: "d", position: { referencePanel: "b", direction: "below" } },
  ]);
});

test("buildDefault: 5 sessions build 3 columns, row-major", () => {
  const api = fakeApi();
  buildDefault(api, ["a", "b", "c", "d", "e"]);
  const pos = Object.fromEntries(api.added.map((x) => [x.id, x.position]));
  assert.equal(pos.b.direction, "right");
  assert.equal(pos.c.direction, "right");
  assert.deepEqual(pos.d, { referencePanel: "a", direction: "below" });
  assert.deepEqual(pos.e, { referencePanel: "b", direction: "below" });
});

test("reconcile: joining session splits the largest panel along its longer axis", () => {
  const wide = fakeApi([{ id: "big", w: 400, h: 100 }, { id: "small", w: 100, h: 100 }]);
  reconcile(wide, ["big", "small", "new1"]);
  assert.deepEqual(wide.added.at(-1).position, { referencePanel: "big", direction: "right" });

  const tall = fakeApi([{ id: "big", w: 100, h: 400 }, { id: "small", w: 100, h: 100 }]);
  reconcile(tall, ["big", "small", "new2"]);
  assert.deepEqual(tall.added.at(-1).position, { referencePanel: "big", direction: "below" });
});

test("reconcile: an explicit placement hint wins over the heuristic, once", () => {
  const api = fakeApi([{ id: "target", w: 100, h: 100 }, { id: "huge", w: 900, h: 900 }]);
  registerPlacementHint("hinted", "target", "below");
  reconcile(api, ["target", "huge", "hinted"]);
  assert.deepEqual(api.added.at(-1).position, { referencePanel: "target", direction: "below" });

  // consumed — a later join with the same id falls back to the heuristic
  const api2 = fakeApi([{ id: "target", w: 100, h: 100 }, { id: "huge", w: 900, h: 900 }]);
  reconcile(api2, ["target", "huge", "hinted"]);
  assert.equal(api2.added.at(-1).position.referencePanel, "huge");
});

test("reconcile: drops panels whose sessions vanished", () => {
  const api = fakeApi([{ id: "keep" }, { id: "gone" }]);
  reconcile(api, ["keep"]);
  assert.deepEqual(api.panels.map((p) => p.id), ["keep"]);
});
