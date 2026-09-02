// p16-restart: pure-function coverage for POST /api/sessions/restart's
// target-selection step (planRestartTargets — no tmux/IO), the
// pre-plan-poll decision (needsPollBeforePlan — a fake hasStatus, no real
// store), the missing-claudeSessionId branch of the shared restore/restart
// spawn helper (spawnClaudeResumed — also no tmux call on that branch,
// since it returns before ever calling hasSession), and the daemon
// self-restart port-handoff wait (waitForPredecessor — a fake probe/clock,
// no real daemon or time).
import { test } from "node:test";
import assert from "node:assert/strict";
import { planRestartTargets, needsPollBeforePlan, spawnClaudeResumed } from "../src/sessions.js";
import { waitForPredecessor } from "../src/daemon-restart.js";

// ---------------------------------------------------------------------
// planRestartTargets
// ---------------------------------------------------------------------

test("idle and done sessions are targets by default", () => {
  const { targets, skipped } = planRestartTargets(
    [
      { id: "garage/ws/a", status: "idle" },
      { id: "garage/ws/b", status: "done" },
    ],
    false
  );
  assert.deepEqual(targets, ["garage/ws/a", "garage/ws/b"]);
  assert.deepEqual(skipped, []);
});

test("working and needs-input sessions are skipped without force", () => {
  const { targets, skipped } = planRestartTargets(
    [
      { id: "garage/ws/a", status: "working" },
      { id: "garage/ws/b", status: "needs-input" },
      { id: "garage/ws/c", status: "idle" },
    ],
    false
  );
  assert.deepEqual(targets, ["garage/ws/c"]);
  assert.deepEqual(skipped, [
    { id: "garage/ws/a", status: "working" },
    { id: "garage/ws/b", status: "needs-input" },
  ]);
});

test("force includes working and needs-input sessions", () => {
  const { targets, skipped } = planRestartTargets(
    [
      { id: "garage/ws/a", status: "working" },
      { id: "garage/ws/b", status: "needs-input" },
    ],
    true
  );
  assert.deepEqual(targets, ["garage/ws/a", "garage/ws/b"]);
  assert.deepEqual(skipped, []);
});

test("restorable sessions never appear — the caller only passes live ones", () => {
  // planRestartTargets has no notion of "restorable" at all: the route
  // scopes candidates to listSessions() (live tmux sessions only) before
  // calling this — a restorable entry (tmux-dead, resume-metadata-only)
  // simply never makes it into the array this function sees.
  const { targets, skipped } = planRestartTargets([{ id: "garage/ws/a", status: "idle" }], false);
  assert.deepEqual(targets, ["garage/ws/a"]);
  assert.deepEqual(skipped, []);
});

test("unknown status (poller never observed this id) is skipped without force", () => {
  // p16-restart follow-up: "unknown" is the route's stand-in for "hasStatus
  // was false even after a fresh poll" — never planned as if idle.
  const { targets, skipped } = planRestartTargets(
    [
      { id: "garage/ws/a", status: "unknown" },
      { id: "garage/ws/b", status: "idle" },
    ],
    false
  );
  assert.deepEqual(targets, ["garage/ws/b"]);
  assert.deepEqual(skipped, [{ id: "garage/ws/a", status: "unknown" }]);
});

test("force includes an unknown-status session too", () => {
  const { targets, skipped } = planRestartTargets([{ id: "garage/ws/a", status: "unknown" }], true);
  assert.deepEqual(targets, ["garage/ws/a"]);
  assert.deepEqual(skipped, []);
});

// ---------------------------------------------------------------------
// needsPollBeforePlan
// ---------------------------------------------------------------------

test("no poll needed when every id already has a status entry", () => {
  const hasStatus = (id) => id !== "garage/ws/missing";
  assert.equal(
    needsPollBeforePlan(["garage/ws/a", "garage/ws/b"], hasStatus),
    false
  );
});

test("poll needed when any id has no status entry yet", () => {
  const hasStatus = (id) => id !== "garage/ws/missing";
  assert.equal(
    needsPollBeforePlan(["garage/ws/a", "garage/ws/missing"], hasStatus),
    true
  );
});

test("empty id list never needs a poll", () => {
  assert.equal(
    needsPollBeforePlan([], () => {
      throw new Error("hasStatus must not be called for an empty id list");
    }),
    false
  );
});

// ---------------------------------------------------------------------
// spawnClaudeResumed — missing claudeSessionId branch (no tmux call)
// ---------------------------------------------------------------------

test("no claudeSessionId spawns plain claude and reports resumed:false", async () => {
  const calls = [];
  const spawn = async (id, dir, command, extraArgs = []) => {
    calls.push({ id, dir, command, extraArgs });
  };
  const resumed = await spawnClaudeResumed(spawn, "garage/ws/one", "/tmp/one", null);
  assert.equal(resumed, false);
  assert.deepEqual(calls, [
    { id: "garage/ws/one", dir: "/tmp/one", command: "claude", extraArgs: [] },
  ]);
});

// ---------------------------------------------------------------------
// waitForPredecessor
// ---------------------------------------------------------------------

test("returns as soon as the probe reports the predecessor gone", async () => {
  let calls = 0;
  const probe = async () => {
    calls += 1;
    return calls < 3; // "still there" twice, then "gone"
  };
  const sleeps = [];
  await waitForPredecessor({
    port: 4747,
    pid: 123,
    probe,
    sleep: async (ms) => sleeps.push(ms),
  });
  assert.equal(calls, 3);
  assert.deepEqual(sleeps, [200, 200]);
});

test("returns immediately when the predecessor is already gone", async () => {
  const probe = async () => false;
  const sleep = async () => assert.fail("must not sleep when the first probe already says gone");
  await waitForPredecessor({ port: 4747, pid: 123, probe, sleep });
});

test("throws, naming the port and pid, once the deadline passes", async () => {
  const probe = async () => true; // predecessor never lets go
  let now = 0;
  await assert.rejects(
    waitForPredecessor({
      port: 4747,
      pid: 999,
      probe,
      now: () => now,
      sleep: async () => {
        now += 200;
      },
      deadlineMs: 1000,
    }),
    (err) => {
      assert.match(err.message, /4747/);
      assert.match(err.message, /999/);
      return true;
    }
  );
});
