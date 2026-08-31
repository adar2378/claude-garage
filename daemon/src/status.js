import { EventEmitter } from "node:events";

// D-status: StatusStore. Four states: needs-input | working | done | idle.
// setStatus is the single write path — API, SSE, and notifications only
// ever read (getStatus/getStatusEntry) or subscribe (statusEvents). There
// is NO auto-decay: `done` now holds until something else transitions it
// (poller sees 'busy', a hook fires, or dropSession removes the entry when
// the session dies). `needs-input` never auto-decayed either — same rule,
// it only clears via a new setStatus call. `since` records the epoch ms of
// the last actual state change (not touched by repeat setStatus calls with
// the same state), so the UI can render "how long in this state".
//
// p8 message capture: needs-input may carry the Notification hook's message
// text. Only hook-sourced writes pass one; the poller's coarse `waiting`
// observation passes none, and a messageless same-state write must not wipe
// a hook-set message (the 2s poller would otherwise erase it on the next
// tick). Any transition away from needs-input clears it.

const store = new Map(); // id -> { state, since, message }

export const statusEvents = new EventEmitter();
statusEvents.setMaxListeners(0);

// Unknown/never-signaled sessions read as idle.
export function getStatus(id) {
  return store.get(id)?.state ?? "idle";
}

// Same lookup as getStatus but also exposes `since` (epoch ms the current
// state began, or null for a never-signaled id) so callers can compute
// elapsed time without a second store.
export function getStatusEntry(id) {
  return store.get(id) ?? { state: "idle", since: null, message: null };
}

export function getAllStatuses() {
  return new Map(store);
}

export function setStatus(id, state, message = undefined) {
  const existing = store.get(id);
  const prev = existing?.state ?? "idle";
  const changed = prev !== state;
  const since = changed ? Date.now() : existing?.since ?? Date.now();

  // Only needs-input carries a message. A messageless needs-input write
  // (the poller) preserves what a hook already captured on a same-state
  // tick, but never invents one on a fresh transition.
  const nextMessage =
    state !== "needs-input"
      ? null
      : message !== undefined
        ? message
        : prev === "needs-input"
          ? existing?.message ?? null
          : null;

  store.set(id, { state, since, message: nextMessage });

  if (changed) {
    statusEvents.emit("transition", { id, from: prev, to: state });
  }
}

// Called when a session disappears from tmux (poller-detected death) so a
// future reuse of the same id starts clean.
export function dropSession(id) {
  store.delete(id);
}
