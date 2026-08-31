import { timingSafeEqual } from "node:crypto";

// Shared by hooks.js and statusline.js — both authenticate a URL query-param
// token (Claude Code's HTTP hooks and the statusline wrapper are both
// URL-only config) against the same per-install hook token from
// registry.js. Constant-time compare so response timing can't leak the
// expected token a byte at a time.
export function tokenMatches(given, expected) {
  if (typeof given !== "string") return false;
  const a = Buffer.from(given);
  const b = Buffer.from(expected);
  return a.length === b.length && timingSafeEqual(a, b);
}
