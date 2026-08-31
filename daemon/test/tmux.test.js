// p10: pure-function coverage for the pane-title plumbing added to
// listPanePaths — no live tmux server touched here (parsePaneLine and
// normalizeTitle are both pure).
import { test } from "node:test";
import assert from "node:assert/strict";
import { parsePaneLine, normalizeTitle } from "../src/tmux.js";

test("parsePaneLine splits the three fixed fields", () => {
  assert.deepEqual(
    parsePaneLine("garage/ws/one\t/Users/me/repo\t✳ Refactoring the poller"),
    { name: "garage/ws/one", panePath: "/Users/me/repo", title: "✳ Refactoring the poller" }
  );
});

test("parsePaneLine rejoins an embedded separator into the title, not the earlier fields", () => {
  // pane_title is free text — tmux doesn't forbid a literal tab in it, even
  // though real-world titles typically don't contain one. The title field
  // is last in the format string specifically so this can't shift
  // session_name/pane_current_path.
  const line = "garage/ws/one\t/Users/me/repo\tbefore\tafter";
  assert.deepEqual(parsePaneLine(line), {
    name: "garage/ws/one",
    panePath: "/Users/me/repo",
    title: "before\tafter",
  });
});

test("parsePaneLine handles an empty title (trailing tab with nothing after)", () => {
  assert.deepEqual(parsePaneLine("garage/ws/one\t/Users/me/repo\t"), {
    name: "garage/ws/one",
    panePath: "/Users/me/repo",
    title: "",
  });
});

test("normalizeTitle: empty/whitespace-only title is null", () => {
  assert.equal(normalizeTitle("", "myhost"), null);
  assert.equal(normalizeTitle("   ", "myhost"), null);
  assert.equal(normalizeTitle(null, "myhost"), null);
  assert.equal(normalizeTitle(undefined, "myhost"), null);
});

test("normalizeTitle: the machine hostname is null (case-insensitive)", () => {
  assert.equal(normalizeTitle("My-Mac", "my-mac"), null);
  assert.equal(normalizeTitle("MY-MAC", "my-mac"), null);
  assert.equal(normalizeTitle("my-mac", "MY-MAC"), null);
});

test("normalizeTitle: the short hostname (before the first dot) is null", () => {
  // tmux's default pane title on macOS is often the short hostname while
  // os.hostname() returns the fully-qualified form (or vice versa).
  assert.equal(normalizeTitle("my-mac", "my-mac.local"), null);
  assert.equal(normalizeTitle("MY-MAC", "my-mac.local"), null);
  assert.equal(normalizeTitle("my-mac.local", "my-mac.local"), null);
});

test("normalizeTitle: bare shell/process names are null", () => {
  for (const name of ["zsh", "bash", "sh", "fish", "-zsh", "-bash", "-sh", "-fish", "tmux"]) {
    assert.equal(normalizeTitle(name, "my-mac"), null, `expected "${name}" to normalize to null`);
    assert.equal(
      normalizeTitle(name.toUpperCase(), "my-mac"),
      null,
      `expected "${name.toUpperCase()}" to normalize to null`
    );
  }
});

test("normalizeTitle: a real title is returned trimmed", () => {
  assert.equal(normalizeTitle("✳ Refactoring the poller", "my-mac"), "✳ Refactoring the poller");
  assert.equal(normalizeTitle("  ✳ Refactoring the poller  ", "my-mac"), "✳ Refactoring the poller");
});

test("normalizeTitle: a title that merely contains the hostname as a substring is kept", () => {
  // Only an exact match (full or short hostname) is filtered — a title that
  // happens to mention the machine name is real signal, not tmux noise.
  assert.equal(normalizeTitle("ssh my-mac", "my-mac"), "ssh my-mac");
});

test("normalizeTitle: works with no hostname supplied (skips that check)", () => {
  assert.equal(normalizeTitle("some title", undefined), "some title");
  assert.equal(normalizeTitle("zsh", undefined), null);
});
