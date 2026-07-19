// p8-theming: theme registry + resolution. One token set
// (--color-garage-*) painted by three palettes — the original garage
// dark, and the two Claude Desktop palettes extracted verbatim in
// CLAUDE-THEME.md — plus a "system" setting that follows the OS between
// the Claude pair. The CSS side lives in index.css
// (:root[data-theme="..."] blocks); this module owns resolution, the
// data-theme application, and the xterm.js color objects (xterm paints a
// canvas — CSS vars can't reach it, so each palette carries an explicit
// terminal theme).

import { useEffect, useState } from "react";
import { useSettings } from "./settings.js";

export const THEME_OPTIONS = [
  { value: "garage", label: "garage", description: "the original pit-wall dark" },
  { value: "claude-dark", label: "claude dark", description: "Claude Desktop's dark palette" },
  { value: "claude-light", label: "claude light", description: "Claude Desktop's ivory light palette" },
  { value: "system", label: "system", description: "follow the OS — claude dark ⇄ claude light" },
];

export function resolveTheme(setting) {
  if (setting === "garage" || setting === "claude-dark" || setting === "claude-light") {
    return setting;
  }
  if (setting === "system" && typeof window !== "undefined") {
    return window.matchMedia("(prefers-color-scheme: dark)").matches
      ? "claude-dark"
      : "claude-light";
  }
  return "garage";
}

/** Effective theme for a setting value, live-updating while "system". */
export function useEffectiveTheme(setting) {
  const [theme, setTheme] = useState(() => resolveTheme(setting));
  useEffect(() => {
    setTheme(resolveTheme(setting));
    if (setting !== "system") return;
    const mq = window.matchMedia("(prefers-color-scheme: dark)");
    const onChange = () => setTheme(resolveTheme("system"));
    mq.addEventListener("change", onChange);
    return () => mq.removeEventListener("change", onChange);
  }, [setting]);
  return theme;
}

/**
 * Reads the theme setting and stamps `data-theme` on <html> (absent for
 * the garage default). Called once per window root (App, SoloView) —
 * popout windows follow the main window automatically because settings.js
 * already syncs across windows via `storage` events.
 */
export function useApplyTheme() {
  const [settings] = useSettings();
  const theme = useEffectiveTheme(settings.theme);
  useEffect(() => {
    if (theme === "garage") delete document.documentElement.dataset.theme;
    else document.documentElement.dataset.theme = theme;
  }, [theme]);
  return theme;
}

// Default font stack (p8-theming): Google Sans Code (bundled via
// @fontsource, OFL-licensed — see CLAUDE-THEME.md's licensing note for
// why Anthropic Sans can NOT be bundled the same way), falling back to
// the system mono stack.
export const MONO_STACK =
  '"Google Sans Code", ui-monospace, "SF Mono", SFMono-Regular, Menlo, Monaco, Consolas, monospace';

// xterm.js theme objects, one per palette. EVERY palette overrides the 16
// ANSI colors — terminal content (Claude Code's diffs, ls, test output)
// renders through this palette, and xterm's defaults are garishly
// saturated against all three of our grounds (user feedback 2026-07-19:
// diff green/red screaming out of an otherwise muted theme). Each also
// remaps the classic 256-palette diff-background indices via
// extendedAnsi. Note the honest limit: if a program emits truecolor RGB,
// no terminal palette can remap it — that content colors itself.
function extendedAnsi(map) {
  const arr = [];
  for (const [idx, color] of Object.entries(map)) arr[Number(idx) - 16] = color;
  return arr;
}

export const TERMINAL_THEMES = {
  garage: {
    background: "#0b0e14",
    foreground: "#c6cfdb",
    cursor: "#c6cfdb",
    selectionBackground: "#2a3646",
    black: "#10141d",
    red: "#cc6b6b",
    green: "#79b26e",
    yellow: "#e2a75e",
    blue: "#6e9ecc",
    magenta: "#a98bc4",
    cyan: "#6fb3a8",
    white: "#c6cfdb",
    brightBlack: "#556070",
    brightRed: "#d98d8d",
    brightGreen: "#93c489",
    brightYellow: "#edbf85",
    brightBlue: "#8ab4dd",
    brightMagenta: "#bda1d6",
    brightCyan: "#8ac4ba",
    brightWhite: "#e6ecf3",
    // muted diff-background slots (256-color indices git/Claude diffs use)
    extendedAnsi: extendedAnsi({
      22: "#1c2b19", // dark green bg
      28: "#243c20",
      65: "#33452e",
      52: "#301b1b", // dark red bg
      88: "#3e211f",
      124: "#4d2a26",
    }),
  },
  "claude-dark": {
    background: "#141413",
    foreground: "#c2c0b6",
    cursor: "#c2c0b6",
    selectionBackground: "#3a3937",
    black: "#1f1e1d",
    red: "#dd5353",
    green: "#6aa84f",
    yellow: "#d97757",
    blue: "#2c84db",
    magenta: "#a179c9",
    cyan: "#5ba8a0",
    white: "#c2c0b6",
    brightBlack: "#6f6d64",
    brightRed: "#e57e7e",
    brightGreen: "#86bd6d",
    brightYellow: "#e39a80",
    brightBlue: "#5fa1e4",
    brightMagenta: "#b494d6",
    brightCyan: "#7abcb5",
    brightWhite: "#faf9f5",
    extendedAnsi: extendedAnsi({
      22: "#20301c",
      28: "#294024",
      65: "#37452f",
      52: "#362020",
      88: "#452824",
      124: "#54312a",
    }),
  },
  "claude-light": {
    background: "#faf9f5",
    foreground: "#3d3d3a",
    cursor: "#3d3d3a",
    cursorAccent: "#faf9f5",
    selectionBackground: "#e0dcc9",
    black: "#3d3d3a",
    red: "#b53232",
    green: "#2f7613",
    yellow: "#8f6c00",
    blue: "#1a6fc4",
    magenta: "#8a4fbf",
    cyan: "#0e7c86",
    white: "#73726c",
    brightBlack: "#73726c",
    brightRed: "#dd5353",
    brightGreen: "#519313",
    brightYellow: "#a87f00",
    brightBlue: "#2c84db",
    brightMagenta: "#a06ad0",
    brightCyan: "#12939f",
    brightWhite: "#141413",
    extendedAnsi: extendedAnsi({
      22: "#dcebd2", // light diff-green bg
      28: "#cfe3c2",
      194: "#e3eeda",
      52: "#f0d2cd", // light diff-red bg
      88: "#e9c6bf",
      224: "#f5ddd6",
    }),
  },
};

export function terminalThemeFor(theme) {
  return TERMINAL_THEMES[theme] ?? TERMINAL_THEMES.garage;
}
