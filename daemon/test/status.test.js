import { test, beforeEach } from "node:test";
import assert from "node:assert/strict";
import { setStatus, getStatus, dropSession, statusEvents } from "../src/status.js";

const ID = "garage/test/one";

beforeEach(() => dropSession(ID));

test("unknown sessions read as idle", () => {
  assert.equal(getStatus("garage/never/heard-of"), "idle");
});

test("setStatus stores and getStatus reads", () => {
  setStatus(ID, "working");
  assert.equal(getStatus(ID), "working");
});

test("transition events fire once per actual change", () => {
  const seen = [];
  const onTransition = (e) => seen.push(e);
  statusEvents.on("transition", onTransition);
  setStatus(ID, "working");
  setStatus(ID, "working"); // no change — no event
  setStatus(ID, "needs-input");
  statusEvents.off("transition", onTransition);

  assert.deepEqual(
    seen.map((e) => `${e.from}->${e.to}`),
    ["idle->working", "working->needs-input"]
  );
});

test("done decays to idle after the decay window", (t) => {
  t.mock.timers.enable({ apis: ["setTimeout"] });
  setStatus(ID, "done");
  assert.equal(getStatus(ID), "done");
  t.mock.timers.tick(2 * 60 * 1000 + 1);
  assert.equal(getStatus(ID), "idle");
});

test("needs-input never auto-decays", (t) => {
  t.mock.timers.enable({ apis: ["setTimeout"] });
  setStatus(ID, "needs-input");
  t.mock.timers.tick(60 * 60 * 1000);
  assert.equal(getStatus(ID), "needs-input");
});

test("a new transition cancels a pending done-decay", (t) => {
  t.mock.timers.enable({ apis: ["setTimeout"] });
  setStatus(ID, "done");
  setStatus(ID, "working");
  t.mock.timers.tick(10 * 60 * 1000);
  assert.equal(getStatus(ID), "working");
});

test("dropSession forgets state and timers", (t) => {
  t.mock.timers.enable({ apis: ["setTimeout"] });
  setStatus(ID, "done");
  dropSession(ID);
  assert.equal(getStatus(ID), "idle");
  t.mock.timers.tick(10 * 60 * 1000); // decay firing after drop must not resurrect
  assert.equal(getStatus(ID), "idle");
});
