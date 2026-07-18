import React, { useState } from "react";
import { createSession } from "../lib/api.js";

// Per-workspace "+" affordance (spec: pit-wall-ui "New-session affordance
// per workspace"). Expands into a label input on click; Escape cancels,
// Enter submits. 404 (unknown workspace) / 409 (duplicate label) surface
// inline rather than as a toast — the rail is small, keep the error close
// to the control that caused it.
export default function AddSessionControl({ workspaceName, onCreated }) {
  const [open, setOpen] = useState(false);
  const [label, setLabel] = useState("");
  const [error, setError] = useState(null);
  const [busy, setBusy] = useState(false);

  function reset() {
    setOpen(false);
    setLabel("");
    setError(null);
  }

  async function submit(e) {
    e.preventDefault();
    const trimmed = label.trim();
    if (!trimmed || busy) return;
    setBusy(true);
    setError(null);
    try {
      await createSession(workspaceName, trimmed);
      reset();
      onCreated();
    } catch (err) {
      setError(err.message);
      setBusy(false);
    }
  }

  if (!open) {
    return (
      <button
        type="button"
        onClick={() => setOpen(true)}
        title={`new session in ${workspaceName}`}
        className="shrink-0 px-1 text-garage-faint hover:text-garage-amber"
      >
        +
      </button>
    );
  }

  return (
    <form onSubmit={submit} className="flex shrink-0 items-center gap-1">
      <input
        autoFocus
        value={label}
        onChange={(e) => setLabel(e.target.value)}
        onKeyDown={(e) => {
          // Stop pit-wall keybindings (1-9/[/]/a) from firing while typing
          // a label — the global listener only excludes .xterm focus, so
          // our own inputs must opt out explicitly.
          e.stopPropagation();
          if (e.key === "Escape") reset();
        }}
        placeholder="label"
        className="w-16 border border-garage-line bg-garage-bg px-1 text-xs text-garage-ink outline-none focus:border-garage-amber"
      />
      <button type="submit" disabled={busy} className="text-garage-green disabled:opacity-40">
        ↵
      </button>
      {error && (
        <span className="text-garage-red" title={error}>
          !
        </span>
      )}
    </form>
  );
}
