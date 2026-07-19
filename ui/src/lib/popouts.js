// Popout registry (design D-popout): tracks which sessions are currently
// displayed in a separate pop-out browser window via a localStorage
// heartbeat protocol. There is no message-passing here on purpose — the
// main window and any number of popout windows only ever agree through
// `localStorage['garage-popouts']`, a plain `{id: lastBeatTs}` map:
//
//   - a popout window calls `registerHeartbeat(id)` on mount, which writes
//     an initial beat and then re-beats every BEAT_MS while it stays open,
//     clearing its entry on a clean close (`pagehide`/`beforeunload`).
//   - the main window treats an id as "popped out" while its last beat is
//     newer than STALE_MS old (`isPoppedOut`/`listPoppedOut`) — this is
//     what makes a force-killed window self-heal: once its beat goes
//     silent for STALE_MS, the main grid reclaims the cell on its own.
//   - `subscribe(fn)` lets the main window re-check on both `storage`
//     events (fired in *other* tabs/windows when the map changes — i.e.
//     exactly the popout-window-side writes) and a BEAT_MS interval, since
//     a same-window write (e.g. this window's own `openPopout`/
//     `clearPopout` calls) never fires `storage` locally, and staleness
//     ticking over STALE_MS is a pure time-based event with no write at
//     all to hang a listener off of.

const STORAGE_KEY = "garage-popouts";
const BEAT_MS = 5_000;
const STALE_MS = 15_000;

function readAll() {
  try {
    const raw = localStorage.getItem(STORAGE_KEY);
    if (!raw) return {};
    const parsed = JSON.parse(raw);
    return parsed && typeof parsed === "object" ? parsed : {};
  } catch {
    return {};
  }
}

function writeAll(map) {
  try {
    localStorage.setItem(STORAGE_KEY, JSON.stringify(map));
  } catch {
    // best-effort — a full/blocked localStorage just means popout tracking
    // degrades to "always treat as live", which is the safe direction to
    // fail in (worst case: a stale placeholder never appears).
  }
}

// Records/refreshes `id`'s heartbeat timestamp.
export function beat(id) {
  const map = readAll();
  map[id] = Date.now();
  writeAll(map);
}

// Clears `id`'s entry — called on a clean popout close, or by the main
// window's reclaim control.
export function clearPopout(id) {
  const map = readAll();
  if (id in map) {
    delete map[id];
    writeAll(map);
  }
}

export function isPoppedOut(id, now = Date.now()) {
  const ts = readAll()[id];
  return typeof ts === "number" && now - ts < STALE_MS;
}

// Snapshot of every currently-live (non-stale) popped-out session id, as a
// Set — the shape TerminalGrid wants for `.has(id)` checks per cell.
export function listPoppedOut(now = Date.now()) {
  const map = readAll();
  const ids = new Set();
  for (const [id, ts] of Object.entries(map)) {
    if (typeof ts === "number" && now - ts < STALE_MS) ids.add(id);
  }
  return ids;
}

// Opens a session's terminal in its own browser window (design D-popout).
// Seeds the heartbeat synchronously so the main window's very next
// re-check (its own call, not a `storage` event — see file comment) sees
// it live immediately rather than waiting on the popout's own first beat.
export function openPopout(id) {
  beat(id);
  window.open(`/?solo=${encodeURIComponent(id)}`, "_blank", "width=980,height=720");
}

// Called by SoloView on mount for the session it's displaying. Beats
// immediately, then every BEAT_MS while mounted; clears the entry on
// unmount *and* on pagehide/beforeunload, since a hard window close can
// skip React's unmount cleanup but not both of those.
export function registerHeartbeat(id) {
  beat(id);
  const interval = setInterval(() => beat(id), BEAT_MS);
  const clear = () => clearPopout(id);
  window.addEventListener("pagehide", clear);
  window.addEventListener("beforeunload", clear);
  return () => {
    clearInterval(interval);
    window.removeEventListener("pagehide", clear);
    window.removeEventListener("beforeunload", clear);
    clear();
  };
}

// Subscribes `fn` to "the popped-out set may have changed" — cross-window
// changes via the `storage` event, plus a BEAT_MS poll to catch same-window
// writes and pure staleness timeouts (neither of which fire `storage`).
// Returns an unsubscribe function.
export function subscribe(fn) {
  const onStorage = (e) => {
    if (!e.key || e.key === STORAGE_KEY) fn();
  };
  window.addEventListener("storage", onStorage);
  const interval = setInterval(fn, BEAT_MS);
  return () => {
    window.removeEventListener("storage", onStorage);
    clearInterval(interval);
  };
}
