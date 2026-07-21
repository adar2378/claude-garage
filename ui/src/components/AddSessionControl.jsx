import React, { useState } from "react";
import { createSession } from "../lib/api.js";

// localStorage key for this workspace's remembered worktree default (design
// D-wt-ui). Scoped per-workspace, not global — someone running throwaway
// experiments in one repo and long-lived sessions in another shouldn't
// have one habit leak into the other.
const wtDefaultKey = (workspaceName) => `garage-wt-default:${workspaceName}`;

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
  // Lazy-init from localStorage: this component instance is keyed by
  // workspace (WorkspaceRail renders one per `group.name`), so reading once
  // at mount is enough — it never needs to re-read for a different
  // workspace under the same instance.
  const [worktree, setWorktree] = useState(() => {
    try {
      return localStorage.getItem(wtDefaultKey(workspaceName)) === "1";
    } catch {
      return false;
    }
  });

  function reset() {
    setOpen(false);
    setLabel("");
    setError(null);
    setBusy(false);
  }

  async function submit(e) {
    e.preventDefault();
    const trimmed = label.trim();
    if (!trimmed || busy) return;
    setBusy(true);
    setError(null);
    try {
      await createSession(workspaceName, trimmed, { worktree });
      try {
        localStorage.setItem(wtDefaultKey(workspaceName), worktree ? "1" : "0");
      } catch {
        // best-effort only — losing the remembered default isn't fatal
      }
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
        className="shrink-0 px-1.5 py-0.5 text-garage-faint hover:text-garage-amber"
      >
        +
      </button>
    );
  }

  // p7 (spec: "Inline session-creation errors and worktree label"): the
  // toggle spells out "worktree", and failures render as readable inline
  // text under the row — never a hover-only glyph.
  return (
    <form onSubmit={submit} className="flex shrink-0 flex-col gap-0.5">
      <div className="flex items-center gap-1">
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
        <label
          title="spawn in an isolated git worktree (garage/<label> branch)"
          className="flex shrink-0 items-center gap-1 text-[10px] text-garage-dim"
        >
          <input
            type="checkbox"
            checked={worktree}
            onChange={(e) => setWorktree(e.target.checked)}
            onKeyDown={(e) => e.stopPropagation()}
            className="h-3 w-3 accent-garage-amber"
          />
          worktree
        </label>
        <button
          type="submit"
          disabled={busy}
          title="create session"
          className="px-1 text-garage-green disabled:opacity-40"
        >
          ↵
        </button>
      </div>
      {error && (
        <p
          role="alert"
          className="max-w-[200px] whitespace-normal text-[10px] leading-tight text-garage-red"
        >
          {error}
        </p>
      )}
    </form>
  );
}
