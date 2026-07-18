import React, { useState } from "react";
import { putWorkspace } from "../lib/api.js";

// Header "add workspace" affordance (spec: 3.4). Name + dir → PUT
// /api/workspaces; 400 (invalid name/dir) surfaces inline.
export default function AddWorkspaceForm({ onClose, onCreated }) {
  const [name, setName] = useState("");
  const [dir, setDir] = useState("");
  const [error, setError] = useState(null);
  const [busy, setBusy] = useState(false);

  async function submit(e) {
    e.preventDefault();
    const trimmedName = name.trim();
    const trimmedDir = dir.trim();
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

  function stopKey(e) {
    e.stopPropagation();
    if (e.key === "Escape") onClose();
  }

  return (
    <form
      onSubmit={submit}
      className="absolute right-0 top-full z-10 mt-1 flex w-72 flex-col gap-2 border border-garage-line bg-garage-panel p-3 shadow-lg"
    >
      <label className="flex flex-col gap-1 text-xs text-garage-dim">
        name
        <input
          autoFocus
          value={name}
          onChange={(e) => setName(e.target.value)}
          onKeyDown={stopKey}
          placeholder="kowboy"
          className="border border-garage-line bg-garage-bg px-2 py-1 text-garage-ink outline-none focus:border-garage-amber"
        />
      </label>
      <label className="flex flex-col gap-1 text-xs text-garage-dim">
        dir
        <input
          value={dir}
          onChange={(e) => setDir(e.target.value)}
          onKeyDown={stopKey}
          placeholder="/Users/me/dev/kowboy"
          className="border border-garage-line bg-garage-bg px-2 py-1 text-garage-ink outline-none focus:border-garage-amber"
        />
      </label>
      {error && <p className="text-xs text-garage-red">{error}</p>}
      <div className="flex justify-end gap-2">
        <button
          type="button"
          onClick={onClose}
          className="px-2 py-0.5 text-xs text-garage-dim hover:text-garage-ink"
        >
          cancel
        </button>
        <button
          type="submit"
          disabled={busy || !name.trim() || !dir.trim()}
          className="border border-garage-line bg-garage-sel px-2 py-0.5 text-xs text-garage-amber disabled:opacity-40"
        >
          register
        </button>
      </div>
    </form>
  );
}
