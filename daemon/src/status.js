import { EventEmitter } from "node:events";

// D-status: StatusStore. Four states: needs-input | working | done | idle.
// setStatus is the single write path — API, SSE, and notifications only
// ever read (getStatus) or subscribe (statusEvents). `done` decays to `idle`
// after DECAY_MS with no further transition; `needs-input` never
// auto-decays — it only clears via a new setStatus call (poller 'busy' or a
// hook Stop/new activity).
const DECAY_MS = 2 * 60 * 1000;

const store = new Map(); // id -> { state, since }
const decayTimers = new Map(); // id -> Timeout

export const statusEvents = new EventEmitter();
statusEvents.setMaxListeners(0);

function clearDecay(id) {
  const timer = decayTimers.get(id);
  if (timer) {
    clearTimeout(timer);
    decayTimers.delete(id);
  }
}

// Unknown/never-signaled sessions read as idle.
export function getStatus(id) {
  return store.get(id)?.state ?? "idle";
}

export function getAllStatuses() {
  return new Map(store);
}

export function setStatus(id, state) {
  const existing = store.get(id);
  const prev = existing?.state ?? "idle";
  const changed = prev !== state;
  const since = changed ? Date.now() : existing?.since ?? Date.now();

  clearDecay(id);
  store.set(id, { state, since });

  if (state === "done") {
    const timer = setTimeout(() => setStatus(id, "idle"), DECAY_MS);
    timer.unref?.();
    decayTimers.set(id, timer);
  }

  if (changed) {
    statusEvents.emit("transition", { id, from: prev, to: state });
  }
}

// Called when a session disappears from tmux (poller-detected death) so
// timers don't leak and a future reuse of the same id starts clean.
export function dropSession(id) {
  clearDecay(id);
  store.delete(id);
}
