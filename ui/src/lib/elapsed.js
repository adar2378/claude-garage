// redesign/light-minimal: elapsed-time formatting for session `since`
// timestamps (epoch ms from the daemon), plus a ticker hook so rendered
// timers actually advance instead of freezing at mount time. Pure, no
// dependencies beyond react. Render with `font-mono tabular-nums text-xs`
// on rail rows and cell headers (see DESIGN-CONTRACT.md section 6).

import { useEffect, useState } from "react";

/**
 * Formats the time elapsed since `sinceMs` (epoch ms).
 * - null/undefined/NaN -> "—"
 * - under an hour -> "M:SS"
 * - under a day -> "Hh MMm"
 * - a day or more -> "Nd"
 * Never negative — clamps to 0 (clock skew / future timestamps read as
 * "just now" rather than a negative duration).
 */
export function formatElapsed(sinceMs) {
  if (sinceMs === null || sinceMs === undefined || Number.isNaN(sinceMs)) return "—";
  const deltaMs = Math.max(0, Date.now() - sinceMs);
  const totalSeconds = Math.floor(deltaMs / 1000);

  if (totalSeconds < 3600) {
    const m = Math.floor(totalSeconds / 60);
    const s = totalSeconds % 60;
    return `${m}:${String(s).padStart(2, "0")}`;
  }

  if (totalSeconds < 86400) {
    const h = Math.floor(totalSeconds / 3600);
    const m = Math.floor((totalSeconds % 3600) / 60);
    return `${h}h ${String(m).padStart(2, "0")}m`;
  }

  const d = Math.floor(totalSeconds / 86400);
  return `${d}d`;
}

/**
 * Forces a re-render every `intervalMs` so components displaying
 * formatElapsed() output actually tick forward. Returns a counter value
 * (unused by callers beyond triggering the render); cleans up its
 * interval on unmount, no-op-safe.
 */
export function useTicker(intervalMs = 1000) {
  const [tick, setTick] = useState(0);
  useEffect(() => {
    const id = setInterval(() => setTick((t) => t + 1), intervalMs);
    return () => clearInterval(id);
  }, [intervalMs]);
  return tick;
}
