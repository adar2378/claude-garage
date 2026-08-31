import React, { useEffect, useRef, useState } from "react";
import { pickDirectory, putWorkspace } from "../lib/api.js";

// Header "add workspace" affordance (spec: 3.4, design D-picker).
// Picker-first: on mount this fires POST /api/pick-directory immediately
// (the "+ add workspace" click that mounted this component *is* the
// trigger — no extra click needed). Outcomes:
//   {dir}            -> derive a name, show an editable inline confirm row
//   {cancelled:true} -> close quietly (user dismissed the native dialog)
//   501/error        -> fall back to the manual name+dir form (non-darwin,
//                       SSH-forwarded browser, or picker failure)
// A "type a path instead" link is always available (even while the picker
// is open or a dir was already picked) so manual mode is never more than
// one click away.
//
// Props are unchanged from before this change (onClose, onCreated) plus
// one addition — `existingNames` — used only for client-side collision
// suffixing of the derived name; the daemon is still the source of truth
// (a stale/missing existingNames just means the -2/-3 suffix guess can be
// wrong, in which case the 409 from putWorkspace surfaces inline as usual).
export default function AddWorkspaceForm({ onClose, onCreated, existingNames }) {
  const [mode, setMode] = useState("picking"); // "picking" | "confirm" | "manual"
  const [dir, setDir] = useState("");
  const [name, setName] = useState("");
  const [error, setError] = useState(null);
  const [busy, setBusy] = useState(false);
  const attempted = useRef(false);

  useEffect(() => {
    if (attempted.current) return;
    attempted.current = true;
    let cancelled = false;
    pickDirectory()
      .then((result) => {
        if (cancelled) return;
        if (result?.cancelled) {
          onClose();
          return;
        }
        setDir(result.dir);
        setName(deriveName(result.dir, existingNames));
        setMode("confirm");
      })
      .catch(() => {
        // 501 (non-darwin) or any transport failure — fall back silently,
        // the manual form is the recovery path, not an error toast.
        if (!cancelled) setMode("manual");
      });
    return () => {
      cancelled = true;
    };
    // Intentionally runs once on mount — the picker is a one-shot action
    // triggered by this component appearing, not by prop changes.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  function stopKey(e) {
    e.stopPropagation();
    if (e.key === "Escape") onClose();
  }

  async function save(finalName, finalDir) {
    const trimmedName = finalName.trim();
    const trimmedDir = finalDir.trim();
    if (!trimmedName || !trimmedDir || busy) return;
    setBusy(true);
    setError(null);
    try {
      await putWorkspace(trimmedName, trimmedDir);
      onCreated();
    } catch (err) {
      setError(err.message);
      setBusy(false);
    }
  }

  function goManual() {
    setError(null);
    setMode("manual");
  }

  if (mode === "picking") {
    return (
      <div
        onKeyDown={stopKey}
        className="absolute right-0 top-full z-10 mt-2 flex w-72 flex-col gap-3 rounded-xl border border-garage-line bg-garage-bg p-4 shadow-sm"
      >
        <p className="text-xs text-garage-dim">opening folder picker…</p>
        <div className="flex items-center justify-between">
          <button
            type="button"
            onClick={goManual}
            className="rounded-md px-2 py-1 text-xs text-garage-dim underline hover:bg-garage-sel hover:text-garage-ink"
          >
            type a path instead
          </button>
          <button
            type="button"
            onClick={onClose}
            className="rounded-md px-3 py-1.5 text-[13px] text-garage-dim hover:bg-garage-sel hover:text-garage-ink"
          >
            cancel
          </button>
        </div>
      </div>
    );
  }

  if (mode === "confirm") {
    return (
      <form
        onSubmit={(e) => {
          e.preventDefault();
          save(name, dir);
        }}
        onKeyDown={stopKey}
        className="absolute right-0 top-full z-10 mt-2 flex w-80 flex-col gap-3 rounded-xl border border-garage-line bg-garage-bg p-4 shadow-sm"
      >
        <label className="flex flex-col gap-1.5 text-xs text-garage-dim">
          name
          <input
            autoFocus
            value={name}
            onChange={(e) => setName(e.target.value)}
            className="rounded-md border border-garage-line bg-garage-bg px-3 py-1.5 text-[13px] text-garage-ink focus:border-garage-dim focus:outline-none"
          />
        </label>
        <p className="truncate text-xs text-garage-faint" title={dir}>
          {dir}
        </p>
        {error && <p className="text-xs text-garage-red">{error}</p>}
        <div className="flex items-center justify-between">
          <button
            type="button"
            onClick={goManual}
            className="rounded-md px-2 py-1 text-xs text-garage-dim underline hover:bg-garage-sel hover:text-garage-ink"
          >
            type a path instead
          </button>
          <div className="flex gap-2">
            <button
              type="button"
              onClick={onClose}
              className="rounded-md px-3 py-1.5 text-[13px] text-garage-dim hover:bg-garage-sel hover:text-garage-ink"
            >
              cancel
            </button>
            <button
              type="submit"
              disabled={busy || !name.trim() || !dir.trim()}
              className="rounded-md bg-garage-ink px-3 py-1.5 text-[13px] text-garage-bg hover:opacity-90 disabled:opacity-40"
            >
              save
            </button>
          </div>
        </div>
      </form>
    );
  }

  // mode === "manual" — same shape as the pre-picker form, reached either
  // by falling back automatically (501/error) or by the "type a path
  // instead" link.
  return (
    <form
      onSubmit={(e) => {
        e.preventDefault();
        save(name, dir);
      }}
      onKeyDown={stopKey}
      className="absolute right-0 top-full z-10 mt-2 flex w-72 flex-col gap-3 rounded-xl border border-garage-line bg-garage-bg p-4 shadow-sm"
    >
      <label className="flex flex-col gap-1.5 text-xs text-garage-dim">
        name
        <input
          autoFocus
          value={name}
          onChange={(e) => setName(e.target.value)}
          placeholder="kowboy"
          className="rounded-md border border-garage-line bg-garage-bg px-3 py-1.5 text-[13px] text-garage-ink focus:border-garage-dim focus:outline-none"
        />
      </label>
      <label className="flex flex-col gap-1.5 text-xs text-garage-dim">
        dir
        <input
          value={dir}
          onChange={(e) => setDir(e.target.value)}
          placeholder="/Users/me/dev/kowboy"
          className="rounded-md border border-garage-line bg-garage-bg px-3 py-1.5 text-[13px] text-garage-ink focus:border-garage-dim focus:outline-none"
        />
      </label>
      {error && <p className="text-xs text-garage-red">{error}</p>}
      <div className="flex justify-end gap-2">
        <button
          type="button"
          onClick={onClose}
          className="rounded-md px-3 py-1.5 text-[13px] text-garage-dim hover:bg-garage-sel hover:text-garage-ink"
        >
          cancel
        </button>
        <button
          type="submit"
          disabled={busy || !name.trim() || !dir.trim()}
          className="rounded-md bg-garage-ink px-3 py-1.5 text-[13px] text-garage-bg hover:opacity-90 disabled:opacity-40"
        >
          register
        </button>
      </div>
    </form>
  );
}

// basename -> lowercase -> non-[a-z0-9]+ runs collapsed to "-" -> trimmed
// of leading/trailing "-"; collision against `existing` appends -2, -3, …
// (design D-picker). Pure/exported for testability even though only this
// module calls it today.
export function deriveName(dir, existing) {
  const names = existing instanceof Set ? existing : new Set(existing ?? []);
  const base = (dir ?? "").split("/").filter(Boolean).pop() ?? "workspace";
  let slug = base
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, "-")
    .replace(/^-+|-+$/g, "");
  if (!slug) slug = "workspace";
  if (!names.has(slug)) return slug;
  let suffix = 2;
  while (names.has(`${slug}-${suffix}`)) suffix++;
  return `${slug}-${suffix}`;
}
