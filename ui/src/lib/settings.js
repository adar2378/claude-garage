// Tiny localStorage-backed settings store (design D-settings). Settings are
// per-browser viewer preferences, not daemon truth — nothing here crosses
// to ~/.garage. Popout windows follow the main window via the `storage`
// event, so a single toggle applies everywhere without any message-passing
// of its own.
//
// Future settings (theme, decay timeout display, notification muting) join
// the DEFAULTS object below — everything else in this module is generic.

import { useSyncExternalStore } from "react";

const STORAGE_KEY = "garage-settings";

const DEFAULTS = Object.freeze({
  focusDim: false,
  // p8-theming: "garage" | "claude-dark" | "claude-light" | "system"
  // (system follows the OS between the two Claude palettes) — see
  // lib/theme.js for resolution and index.css for the palettes.
  theme: "garage",
});

function readFromStorage() {
  try {
    const raw = localStorage.getItem(STORAGE_KEY);
    if (!raw) return { ...DEFAULTS };
    const parsed = JSON.parse(raw);
    return { ...DEFAULTS, ...parsed };
  } catch {
    // Corrupt/blocked storage — fall back to defaults rather than throw.
    return { ...DEFAULTS };
  }
}

let cached = readFromStorage();
const listeners = new Set();

function notify() {
  for (const listener of listeners) listener();
}

// One module-scope `storage` listener (not one per hook subscriber) —
// `storage` only fires in *other* tabs/windows, so this is what makes a
// popout or a second tab pick up a setting changed in the main window.
if (typeof window !== "undefined") {
  window.addEventListener("storage", (e) => {
    if (e.key !== null && e.key !== STORAGE_KEY) return;
    cached = readFromStorage();
    notify();
  });
}

function subscribe(callback) {
  listeners.add(callback);
  return () => listeners.delete(callback);
}

function getSnapshot() {
  return cached;
}

/** Read current settings without subscribing (e.g. one-off checks outside React). */
export function getSettings() {
  return cached;
}

/**
 * Merge `patch` (object, or a function of the previous settings returning
 * a partial object) into the persisted settings and notify subscribers in
 * this window immediately — `storage` events never fire in the window that
 * made the change, so same-window listeners rely on this explicit notify.
 */
export function updateSettings(patch) {
  const partial = typeof patch === "function" ? patch(cached) : patch;
  cached = { ...cached, ...partial };
  try {
    localStorage.setItem(STORAGE_KEY, JSON.stringify(cached));
  } catch {
    // best-effort persistence only — a blocked/full localStorage still
    // gets the in-memory update via `cached`/notify() below.
  }
  notify();
}

/** React 18 external-store hook. Returns [settings, update] — `update` has the same shape as updateSettings. */
export function useSettings() {
  const settings = useSyncExternalStore(subscribe, getSnapshot, () => DEFAULTS);
  return [settings, updateSettings];
}
