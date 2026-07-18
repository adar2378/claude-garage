import React, { useCallback, useEffect, useState } from "react";
import SessionTerminal from "./SessionTerminal.jsx";

// P0: single hardcoded session — the grid comes in P1.
const P0_SESSION = {
  workspace: "garage-dev",
  label: "main",
  dir: "/Users/saifulislam/Development/personal/claude-garage",
};

export default function App() {
  const [sessions, setSessions] = useState(null);
  const [error, setError] = useState(null);

  const refresh = useCallback(() => {
    fetch("/api/sessions")
      .then((r) => r.json())
      .then(setSessions)
      .catch((e) => setError(e.message));
  }, []);

  useEffect(refresh, [refresh]);

  const spawn = async () => {
    const res = await fetch("/api/sessions", {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify(P0_SESSION),
    });
    if (!res.ok) setError((await res.json()).error);
    refresh();
  };

  const shell =
    "flex h-screen flex-col bg-garage-bg font-mono text-sm text-garage-ink";

  if (error) {
    return <main className={shell}><p className="p-4 text-garage-red">error: {error}</p></main>;
  }
  if (sessions === null) {
    return <main className={shell}><p className="p-4 text-garage-dim">loading…</p></main>;
  }

  return (
    <main className={shell}>
      <header className="flex items-center gap-3 border-b border-garage-line bg-garage-panel px-4 py-2">
        <span className="text-garage-amber">claude-garage</span>
        {sessions.length === 0 ? (
          <button
            onClick={spawn}
            className="border border-garage-line bg-garage-sel px-2 py-0.5 text-garage-ink hover:border-garage-amber"
          >
            new session ({P0_SESSION.workspace}/{P0_SESSION.label})
          </button>
        ) : (
          <span className="text-garage-dim">{sessions[0].id}</span>
        )}
      </header>
      <div className="min-h-0 flex-1 p-2">
        {sessions.length > 0 && <SessionTerminal id={sessions[0].id} />}
      </div>
    </main>
  );
}
