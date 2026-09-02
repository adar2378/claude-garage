import { test, beforeEach } from "node:test";
import assert from "node:assert/strict";
import { applyAgentStatus, diffSessionState } from "../src/poller.js";
import { setStatus, getStatus, getStatusEntry, dropSession } from "../src/status.js";

const ID = "garage/test/poll";
const HOST = "my-mac";

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

test("poller-sourced needs-input carries no message", () => {
  applyAgentStatus(ID, "waiting");
  assert.equal(getStatus(ID), "needs-input");
  assert.equal(getStatusEntry(ID).message, null);
});

test("same-state poller tick must not clear a hook-set message", () => {
  setStatus(ID, "needs-input", "Claude needs your permission to use Bash"); // as the Notification hook would
  applyAgentStatus(ID, "waiting"); // poller tick while the prompt sits open
  assert.equal(getStatusEntry(ID).message, "Claude needs your permission to use Bash");
});

// p10 wave-2 fix: diffSessionState is the pure id+title diff pollBody() uses to
// decide whether to emit "sessions-changed" — exercised directly here with
// plain data so title-only-change coverage doesn't need a live tmux server.
const session = (id) => ({ id, workspace: "ws", label: id.split("/").pop() });

test("diffSessionState: a pane-title-only change is reported changed, session set untouched", () => {
  const sessions = [session(ID)];
  const prevIds = new Set([ID]);
  const prevTitles = new Map([[ID, "✳ old summary"]]);
  const panePids = new Map([[ID, { pid: 123, title: "✳ new summary" }]]);

  const { changed, ids, titles } = diffSessionState(sessions, panePids, HOST, prevIds, prevTitles);

  assert.equal(changed, true);
  assert.deepEqual([...ids], [ID]); // the id set itself never changed
  assert.equal(titles.get(ID), "✳ new summary");
});

test("diffSessionState: unchanged id set and unchanged title reports no change", () => {
  const sessions = [session(ID)];
  const prevIds = new Set([ID]);
  const prevTitles = new Map([[ID, "✳ same summary"]]);
  const panePids = new Map([[ID, { pid: 123, title: "✳ same summary" }]]);

  const { changed } = diffSessionState(sessions, panePids, HOST, prevIds, prevTitles);

  assert.equal(changed, false);
});

test("diffSessionState: a title that normalizes the same (default noise) reports no change", () => {
  // e.g. the pane title flaps between the hostname and the bare shell name
  // across ticks — both normalize to null, so comparing raw strings would
  // false-positive here; the diff must compare normalized values.
  const sessions = [session(ID)];
  const prevIds = new Set([ID]);
  const prevTitles = new Map([[ID, null]]); // previously normalized from "zsh"
  const panePids = new Map([[ID, { pid: 123, title: HOST }]]); // now the hostname

  const { changed, titles } = diffSessionState(sessions, panePids, HOST, prevIds, prevTitles);

  assert.equal(changed, false);
  assert.equal(titles.get(ID), null);
});

test("diffSessionState: a new session (never seen before) reports changed via the id set, not just the title map", () => {
  const sessions = [session(ID)];
  const prevIds = new Set(); // nothing tracked yet
  const prevTitles = new Map();
  const panePids = new Map([[ID, { pid: 123, title: null }]]);

  const { changed, ids } = diffSessionState(sessions, panePids, HOST, prevIds, prevTitles);

  assert.equal(changed, true);
  assert.deepEqual([...ids], [ID]);
});

test("diffSessionState: session death (in prevIds, absent from sessions now) still reports changed", () => {
  const sessions = []; // ID is gone
  const prevIds = new Set([ID]);
  const prevTitles = new Map([[ID, "✳ was working on something"]]);
  const panePids = new Map();

  const { changed, ids, titles } = diffSessionState(sessions, panePids, HOST, prevIds, prevTitles);

  assert.equal(changed, true);
  assert.equal(ids.size, 0);
  assert.equal(titles.size, 0); // rebuilt wholesale from (now-empty) sessions
});

test("diffSessionState: a session with no matching pane entry normalizes to a null title without throwing", () => {
  const sessions = [session(ID)];
  const prevIds = new Set([ID]);
  const prevTitles = new Map([[ID, null]]);
  const panePids = new Map(); // mid-spawn — pane not observed yet

  const { changed, titles } = diffSessionState(sessions, panePids, HOST, prevIds, prevTitles);

  assert.equal(changed, false);
  assert.equal(titles.get(ID), null);
});
