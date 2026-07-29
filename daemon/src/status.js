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

const store = new Map(); // id -> { state, since }

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
  return store.get(id) ?? { state: "idle", since: null };
}

export function getAllStatuses() {
  return new Map(store);
}

export function setStatus(id, state) {
  const existing = store.get(id);
  const prev = existing?.state ?? "idle";
  const changed = prev !== state;
  const since = changed ? Date.now() : existing?.since ?? Date.now();

  store.set(id, { state, since });

  if (changed) {
    statusEvents.emit("transition", { id, from: prev, to: state });
  }
}

// Called when a session disappears from tmux (poller-detected death) so a
// future reuse of the same id starts clean.
export function dropSession(id) {
  store.delete(id);
}
