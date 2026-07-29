import { test } from "node:test";
import assert from "node:assert/strict";
import { formatElapsed } from "../src/lib/elapsed.js";

test("formatElapsed handles missing/invalid input", () => {
  assert.equal(formatElapsed(null), "—");
  assert.equal(formatElapsed(undefined), "—");
  assert.equal(formatElapsed(NaN), "—");
});

test("formatElapsed under an hour reads M:SS", () => {
  const now = Date.now();
  assert.equal(formatElapsed(now), "0:00");
  assert.equal(formatElapsed(now - 6 * 1000), "0:06");
  assert.equal(formatElapsed(now - (6 * 60 + 12) * 1000), "6:12");
  assert.equal(formatElapsed(now - 59 * 60 * 1000 - 59 * 1000), "59:59");
});

test("formatElapsed under a day reads Hh MMm", () => {
  const now = Date.now();
  assert.equal(formatElapsed(now - 3600 * 1000), "1h 00m");
  assert.equal(formatElapsed(now - (3600 + 4 * 60) * 1000), "1h 04m");
  assert.equal(formatElapsed(now - 23 * 3600 * 1000 - 59 * 60 * 1000), "23h 59m");
});

test("formatElapsed a day or more reads Nd", () => {
  const now = Date.now();
  assert.equal(formatElapsed(now - 86400 * 1000), "1d");
  assert.equal(formatElapsed(now - 3 * 86400 * 1000), "3d");
});

test("formatElapsed clamps future timestamps to zero, never negative", () => {
  const future = Date.now() + 60 * 1000;
  assert.equal(formatElapsed(future), "0:00");
});
