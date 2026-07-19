import { test, beforeEach } from "node:test";
import assert from "node:assert/strict";
import { applyAgentStatus } from "../src/poller.js";
import { setStatus, getStatus, dropSession } from "../src/status.js";

const ID = "garage/test/poll";

beforeEach(() => dropSession(ID));

test("busy maps to working", () => {
  applyAgentStatus(ID, "busy");
  assert.equal(getStatus(ID), "working");
});

test("waiting maps to needs-input — hookless installs must still get the core signal", () => {
  // This exact mapping was the pre-publish bug: `waiting` (agent blocked
  // on the user, per claude agents --json) was collapsed into idle, so
  // without hooks the product's core state never occurred.
  applyAgentStatus(ID, "waiting");
  assert.equal(getStatus(ID), "needs-input");
});

test("idle maps to idle for a session with no precision state", () => {
  applyAgentStatus(ID, "busy");
  applyAgentStatus(ID, "idle");
  assert.equal(getStatus(ID), "idle");
});

test("coarse idle must not stomp hook-sourced needs-input", () => {
  setStatus(ID, "needs-input"); // as the Notification hook would
  applyAgentStatus(ID, "idle"); // poller tick while the prompt sits open
  assert.equal(getStatus(ID), "needs-input");
});

test("coarse idle must not stomp done before its decay", () => {
  setStatus(ID, "done"); // as the Stop hook would
  applyAgentStatus(ID, "idle");
  assert.equal(getStatus(ID), "done");
});

test("busy clears needs-input — real activity is the release", () => {
  setStatus(ID, "needs-input");
  applyAgentStatus(ID, "busy");
  assert.equal(getStatus(ID), "working");
});
