import React, { useEffect, useRef, useState } from "react";
import { useSettings } from "../lib/settings.js";
import { THEME_OPTIONS } from "../lib/theme.js";
import { PET_OPTIONS } from "../lib/pet.js";

// Header gear affordance (design D-settings). Self-contained: owns its own
// open/closed state and the gear trigger button, so wiring it into the app
// shell is a single `<SettingsPopover />` — no App-owned toggle state
// needed (unlike AddWorkspaceForm, which needs to coordinate a refetch on
// success).
//
// Rows below are simple toggle rows (label + description + a bracket
// marker as the checkbox). New settings join the ROWS array below — each
// just needs a key into lib/settings.js's store, a label, and a one-line
// description.
const ROWS = [
  {
    key: "focusDim",
    label: "focus dimming",
    description: "dim everything except the focused surface; needs-input never dims",
  },
  {
    key: "notifyBrowser",
    label: "browser notifications",
    description:
      "notify when a session needs input while this tab is hidden — clicking jumps to it",
    // Notification permission must be requested from a user gesture;
    // enabling the toggle IS that gesture.
    onEnable: () => {
      if (typeof Notification !== "undefined" && Notification.permission === "default") {
        Notification.requestPermission().catch(() => {});
      }
    },
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
        className="rounded-md px-2 py-1 text-[13px] text-garage-dim hover:bg-garage-sel hover:text-garage-ink"
      >
        ⚙
      </button>
      {open && (
        <div
          role="menu"
          aria-label="settings"
          className="absolute right-0 top-full z-10 mt-2 w-80 rounded-xl border border-garage-line bg-garage-bg p-4 shadow-sm"
        >
          <p className="px-1 pb-1.5 text-[11px] uppercase tracking-wider text-garage-faint">
            settings
          </p>
          {ROWS.map((row, i) => {
            const checked = !!settings[row.key];
            return (
              <button
                key={row.key}
                type="button"
                role="menuitemcheckbox"
                aria-checked={checked}
                onClick={() => {
                  if (!checked) row.onEnable?.();
                  update({ [row.key]: !checked });
                }}
                className={`flex w-full items-start gap-3 rounded-md px-1 py-2 text-left hover:bg-garage-sel ${
                  i < ROWS.length - 1 ? "border-b border-garage-line" : ""
                }`}
              >
                {/* whitespace-nowrap + shrink-0: the marker contains a
                    space ("[ ]"), and a tight flex row will happily wrap
                    it across two lines otherwise. */}
                <span
                  className={`shrink-0 whitespace-nowrap font-mono text-[13px] ${
                    checked ? "text-garage-ink" : "text-garage-faint"
                  }`}
                >
                  {checked ? "[x]" : "[ ]"}
                </span>
                <span className="flex flex-col gap-0.5">
                  <span className="text-[13px] text-garage-ink">{row.label}</span>
                  <span className="text-xs text-garage-dim">{row.description}</span>
                </span>
              </button>
            );
          })}

          {/* p8-theming: radio-style theme rows — (•) marks the active
              choice. THEME_OPTIONS is rendered generically (map over the
              array) so the registry stays the single source of truth. */}
          <p className="px-1 pb-1.5 pt-4 text-[11px] uppercase tracking-wider text-garage-faint">
            theme
          </p>
          {THEME_OPTIONS.map((opt, i) => {
            const selected = (settings.theme ?? "light") === opt.value;
            return (
              <button
                key={opt.value}
                type="button"
                role="menuitemradio"
                aria-checked={selected}
                onClick={() => update({ theme: opt.value })}
                className={`flex w-full items-start gap-3 rounded-md px-1 py-2 text-left hover:bg-garage-sel ${
                  i < THEME_OPTIONS.length - 1 ? "border-b border-garage-line" : ""
                }`}
              >
                <span
                  className={`shrink-0 whitespace-nowrap font-mono text-[13px] ${
                    selected ? "text-garage-ink" : "text-garage-faint"
                  }`}
                >
                  {selected ? "(•)" : "( )"}
                </span>
                <span className="flex flex-col gap-0.5">
                  <span className="text-[13px] text-garage-ink">{opt.label}</span>
                  <span className="text-xs text-garage-dim">{opt.description}</span>
                </span>
              </button>
            );
          })}

          {/* pit-pet roster (lib/pet.js) — same radio aesthetic. */}
          <p className="px-1 pb-1.5 pt-4 text-[11px] uppercase tracking-wider text-garage-faint">
            pit pet
          </p>
          {PET_OPTIONS.map((opt, i) => {
            const selected = (settings.pet ?? "off") === opt.value;
            return (
              <button
                key={opt.value}
                type="button"
                role="menuitemradio"
                aria-checked={selected}
                onClick={() => update({ pet: opt.value })}
                className={`flex w-full items-start gap-3 rounded-md px-1 py-2 text-left hover:bg-garage-sel ${
                  i < PET_OPTIONS.length - 1 ? "border-b border-garage-line" : ""
                }`}
              >
                <span
                  className={`shrink-0 whitespace-nowrap font-mono text-[13px] ${
                    selected ? "text-garage-ink" : "text-garage-faint"
                  }`}
                >
                  {selected ? "(•)" : "( )"}
                </span>
                <span className="flex flex-col gap-0.5">
                  <span className="text-[13px] text-garage-ink">{opt.label}</span>
                  <span className="text-xs text-garage-dim">{opt.description}</span>
                </span>
              </button>
            );
          })}
        </div>
      )}
    </div>
  );
}
