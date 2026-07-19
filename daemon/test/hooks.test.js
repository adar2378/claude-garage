import { test } from "node:test";
import assert from "node:assert/strict";
import { mergeHookSnippet, isIdleReminder } from "../src/hooks.js";

const SNIPPET = {
  hooks: {
    Notification: [{ hooks: [{ type: "http", url: "http://127.0.0.1:4747/api/hooks/claude?token=abc" }] }],
    Stop: [{ hooks: [{ type: "http", url: "http://127.0.0.1:4747/api/hooks/claude?token=abc" }] }],
  },
  allowedHttpHookUrls: ["http://127.0.0.1:4747/*"],
};

test("merge preserves unrelated settings and hooks", () => {
  const settings = {
    model: "opus",
    hooks: { Stop: [{ hooks: [{ type: "command", command: "echo mine" }] }] },
  };
  const { merged, changed } = mergeHookSnippet(settings, SNIPPET);
  assert.equal(changed, true);
  assert.equal(merged.model, "opus");
  const stopCommands = merged.hooks.Stop.flatMap((g) => g.hooks).map((h) => h.command ?? h.url);
  assert.ok(stopCommands.includes("echo mine"));
  assert.equal(merged.hooks.Notification.length, 1);
  assert.deepEqual(merged.allowedHttpHookUrls, ["http://127.0.0.1:4747/*"]);
});

test("merge is idempotent — second application changes nothing", () => {
  const first = mergeHookSnippet({}, SNIPPET);
  const second = mergeHookSnippet(first.merged, SNIPPET);
  assert.equal(second.changed, false);
  assert.equal(second.merged.hooks.Notification.length, 1);
  assert.equal(second.merged.hooks.Stop.length, 1);
});

test("idle reminder is classified as noise; blockers and unknowns are not", () => {
  assert.equal(isIdleReminder("Claude is waiting for your input"), true);
  assert.equal(isIdleReminder("Claude needs your permission to use Bash"), false);
  assert.equal(isIdleReminder(undefined), false); // messageless → attention
  assert.equal(isIdleReminder(""), false);
});
