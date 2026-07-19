import { test } from "node:test";
import assert from "node:assert/strict";
import { computeViews, viewOf, deriveViewName, MAIN_VIEW } from "../src/lib/views.js";

const s = (id, status = "idle") => ({ id, status });

test("no assignments -> one main view holding everything", () => {
  const views = computeViews([s("a"), s("b")], {});
  assert.equal(views.length, 1);
  assert.equal(views[0].name, MAIN_VIEW);
  assert.equal(views[0].sessions.length, 2);
});

test("assignments split into views; empty views vanish", () => {
  const views = computeViews([s("a"), s("c")], { c: "solo", dead: "ghost-view" });
  assert.deepEqual(views.map((v) => v.name), [MAIN_VIEW, "solo"]);
  assert.deepEqual(views[1].sessions.map((x) => x.id), ["c"]);
});

test("main itself vanishes when every session is detached", () => {
  const views = computeViews([s("a")], { a: "solo" });
  assert.deepEqual(views.map((v) => v.name), ["solo"]);
});

test("needsCount aggregates per view — the strip/rail attention dot", () => {
  const views = computeViews([s("a", "needs-input"), s("b"), s("c", "needs-input")], { c: "solo" });
  assert.equal(views.find((v) => v.name === MAIN_VIEW).needsCount, 1);
  assert.equal(views.find((v) => v.name === "solo").needsCount, 1);
});

test("viewOf defaults to main", () => {
  assert.equal(viewOf({}, "x"), MAIN_VIEW);
  assert.equal(viewOf({ x: "solo" }, "x"), "solo");
});

test("deriveViewName avoids collisions and the reserved main name", () => {
  assert.equal(deriveViewName("test", ["main"]), "test");
  assert.equal(deriveViewName("test", ["main", "test"]), "test-2");
  assert.equal(deriveViewName("main", ["main"]), "main-2");
});
