import React, { useEffect, useRef, useState } from "react";
import { useSettings } from "../lib/settings.js";
import { THEME_OPTIONS } from "../lib/theme.js";

// Header gear affordance (design D-settings). Self-contained: owns its own
// open/closed state and the gear trigger button, so wiring it into the app
// shell is a single `<SettingsPopover />` — no App-owned toggle state
// needed (unlike AddWorkspaceForm, which needs to coordinate a refetch on
// success).
//
// Rows are TUI-style `[x]`/`[ ]` mono toggles rather than native checkbox
// styling, matching the rest of the rail/grid chrome. New settings join the
// ROWS array below — each just needs a key into lib/settings.js's store,
// a label, and a one-line description.
const ROWS = [
  {
    key: "focusDim",
    label: "focus dimming",
    description: "dim everything except the focused surface; needs-input never dims",
  },
];

export default function SettingsPopover() {
  const [open, setOpen] = useState(false);
  const [settings, update] = useSettings();
  const containerRef = useRef(null);

  useEffect(() => {
    if (!open) return;
    function onDocMouseDown(e) {
      if (containerRef.current && !containerRef.current.contains(e.target)) {
        setOpen(false);
      }
    }
    function onKeyDown(e) {
      if (e.key === "Escape") {
        e.stopPropagation();
        setOpen(false);
      }
    }
    document.addEventListener("mousedown", onDocMouseDown);
    document.addEventListener("keydown", onKeyDown);
    return () => {
      document.removeEventListener("mousedown", onDocMouseDown);
      document.removeEventListener("keydown", onKeyDown);
    };
  }, [open]);

  return (
    <div ref={containerRef} className="relative">
      <button
        type="button"
        onClick={() => setOpen((v) => !v)}
        title="settings"
        aria-label="settings"
        aria-expanded={open}
        className="border border-garage-line bg-garage-sel px-2 py-0.5 text-xs text-garage-ink hover:border-garage-amber"
      >
        ⚙
      </button>
      {open && (
        <div
          role="menu"
          aria-label="settings"
          className="absolute right-0 top-full z-10 mt-1 w-72 border border-garage-line bg-garage-panel p-2 shadow-lg"
        >
          <p className="px-1 pb-1 text-[10px] uppercase tracking-wide text-garage-faint">
            settings
          </p>
          {ROWS.map((row) => {
            const checked = !!settings[row.key];
            return (
              <button
                key={row.key}
                type="button"
                role="menuitemcheckbox"
                aria-checked={checked}
                onClick={() => update({ [row.key]: !checked })}
                className="flex w-full items-start gap-2 px-1 py-1 text-left hover:bg-garage-sel"
              >
                {/* whitespace-nowrap + shrink-0: the marker contains a
                    space ("[ ]"), and a tight flex row will happily wrap
                    it across two lines otherwise. */}
                <span
                  className={`shrink-0 whitespace-nowrap font-mono ${
                    checked ? "text-garage-amber" : "text-garage-faint"
                  }`}
                >
                  {checked ? "[x]" : "[ ]"}
                </span>
                <span className="flex flex-col gap-0.5">
                  <span className="text-xs text-garage-ink">{row.label}</span>
                  <span className="text-[11px] text-garage-faint">{row.description}</span>
                </span>
              </button>
            );
          })}

          {/* p8-theming: radio-style theme rows, same TUI aesthetic as the
              checkbox rows above — (•) marks the active choice. */}
          <p className="px-1 pb-1 pt-2 text-[10px] uppercase tracking-wide text-garage-faint">
            theme
          </p>
          {THEME_OPTIONS.map((opt) => {
            const selected = (settings.theme ?? "garage") === opt.value;
            return (
              <button
                key={opt.value}
                type="button"
                role="menuitemradio"
                aria-checked={selected}
                onClick={() => update({ theme: opt.value })}
                className="flex w-full items-start gap-2 px-1 py-1 text-left hover:bg-garage-sel"
              >
                <span
                  className={`shrink-0 whitespace-nowrap font-mono ${
                    selected ? "text-garage-amber" : "text-garage-faint"
                  }`}
                >
                  {selected ? "(•)" : "( )"}
                </span>
                <span className="flex flex-col gap-0.5">
                  <span className="text-xs text-garage-ink">{opt.label}</span>
                  <span className="text-[11px] text-garage-faint">{opt.description}</span>
                </span>
              </button>
            );
          })}
        </div>
      )}
    </div>
  );
}
