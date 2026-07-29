import { test, beforeEach } from "node:test";
import assert from "node:assert/strict";
import {
  setStatus,
  getStatus,
  getStatusEntry,
  dropSession,
  statusEvents,
} from "../src/status.js";

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

test("done holds indefinitely — no auto-decay to idle", (t) => {
  t.mock.timers.enable({ apis: ["setTimeout"] });
  setStatus(ID, "done");
  assert.equal(getStatus(ID), "done");
  t.mock.timers.tick(24 * 60 * 60 * 1000); // a full day — still no decay
  assert.equal(getStatus(ID), "done");
});

test("needs-input never auto-decays", (t) => {
  t.mock.timers.enable({ apis: ["setTimeout"] });
  setStatus(ID, "needs-input");
  t.mock.timers.tick(60 * 60 * 1000);
  assert.equal(getStatus(ID), "needs-input");
});

test("done only clears via an explicit setStatus transition", () => {
  setStatus(ID, "done");
  setStatus(ID, "working");
  assert.equal(getStatus(ID), "working");
});

test("dropSession forgets state", () => {
  setStatus(ID, "done");
  dropSession(ID);
  assert.equal(getStatus(ID), "idle");
});

test("getStatusEntry defaults to idle/null for unknown ids", () => {
  assert.deepEqual(getStatusEntry("garage/never/heard-of"), {
    state: "idle",
    since: null,
  });
});

test("getStatusEntry.since changes on a real transition and is preserved on a repeat setStatus with the same state", (t) => {
  t.mock.timers.enable({ apis: ["Date"] });
  setStatus(ID, "working");
  const firstSince = getStatusEntry(ID).since;

  t.mock.timers.tick(5000);
  setStatus(ID, "working"); // same state — since must NOT move
  assert.equal(getStatusEntry(ID).since, firstSince);

  t.mock.timers.tick(5000);
  setStatus(ID, "done"); // real transition — since must advance
  const secondSince = getStatusEntry(ID).since;
  assert.notEqual(secondSince, firstSince);
  assert.equal(getStatusEntry(ID).state, "done");
});
