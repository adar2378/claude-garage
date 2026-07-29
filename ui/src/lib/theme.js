// redesign/light-minimal: theme registry + resolution. One token set
// (--color-garage-*) painted by two palettes — light (the default) and a
// neutral zinc dark — plus a "system" setting that follows the OS between
// the two. The CSS side lives in index.css (:root[data-theme="..."]
// blocks); this module owns resolution, the data-theme application, and
// the xterm.js color objects (xterm paints a canvas — CSS vars can't
// reach it, so each palette carries an explicit terminal theme).

import { useEffect, useState } from "react";
import { useSettings } from "./settings.js";

export const THEME_OPTIONS = [
  { value: "light", label: "light", description: "the default — white ground, ink text" },
  { value: "dark", label: "dark", description: "neutral zinc dark" },
  { value: "system", label: "system", description: "follow the OS — light ⇄ dark" },
];

// Legacy values from the pre-redesign palette (garage dark, Claude
// Desktop's dark/light pair) get migrated so existing users' localStorage
// settings keep resolving to something sane instead of falling through.
const LEGACY_MAP = {
  garage: "dark",
  "claude-dark": "dark",
  "claude-light": "light",
};

export function resolveTheme(setting) {
  if (setting === "light" || setting === "dark") return setting;
  if (setting in LEGACY_MAP) return LEGACY_MAP[setting];
  if (setting === "system" && typeof window !== "undefined") {
    return window.matchMedia("(prefers-color-scheme: dark)").matches ? "dark" : "light";
  }
  return "light";
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
 * Reads the theme setting and stamps `data-theme` on <html> — always,
 * for both "light" and "dark". index.css now defines an explicit
 * `[data-theme="light"]` block (identical to the no-attribute default),
 * so there's no need to delete the attribute for light anymore. Called
 * once per window root (App, SoloView) — popout windows follow the main
 * window automatically because settings.js already syncs across windows
 * via `storage` events.
 */
export function useApplyTheme() {
  const [settings] = useSettings();
  const theme = useEffectiveTheme(settings.theme);
  useEffect(() => {
    document.documentElement.dataset.theme = theme;
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
// saturated against both of our grounds (user feedback 2026-07-19: diff
// green/red screaming out of an otherwise muted theme). Each also remaps
// the classic 256-palette diff-background indices via extendedAnsi. Note
// the honest limit: if a program emits truecolor RGB, no terminal palette
// can remap it — that content colors itself.
function extendedAnsi(map) {
  const arr = [];
  for (const [idx, color] of Object.entries(map)) arr[Number(idx) - 16] = color;
  return arr;
}

export const TERMINAL_THEMES = {
  light: {
    background: "#ffffff",
    foreground: "#27272a",
    cursor: "#27272a",
    cursorAccent: "#ffffff",
    selectionBackground: "#e4e4e7",
    black: "#27272a",
    red: "#b91c1c",
    green: "#15803d",
    yellow: "#a16207",
    blue: "#1d4ed8",
    magenta: "#7e22ce",
    cyan: "#0e7490",
    white: "#52525b",
    brightBlack: "#71717a",
    brightRed: "#991b1b",
    brightGreen: "#166534",
    brightYellow: "#854d0e",
    brightBlue: "#1e40af",
    brightMagenta: "#6b21a8",
    brightCyan: "#155e75",
    brightWhite: "#18181b",
    // pale diff-background slots (256-color indices git/Claude diffs use),
    // legible on a white ground
    extendedAnsi: extendedAnsi({
      22: "#dcfce7", // pale diff-green bg
      28: "#bbf7d0",
      194: "#dcfce7",
      52: "#fee2e2", // pale diff-red bg
      88: "#fecaca",
      224: "#fee2e2",
    }),
  },
  dark: {
    background: "#09090b",
    foreground: "#e4e4e7",
    cursor: "#e4e4e7",
    cursorAccent: "#09090b",
    selectionBackground: "#27272a",
    black: "#131316",
    red: "#f87171",
    green: "#4ade80",
    yellow: "#f59e0b",
    blue: "#a1a1aa",
    magenta: "#c4b5fd",
    cyan: "#67e8f9",
    white: "#e4e4e7",
    brightBlack: "#71717a",
    brightRed: "#fca5a5",
    brightGreen: "#86efac",
    brightYellow: "#fbbf24",
    brightBlue: "#d4d4d8",
    brightMagenta: "#ddd6fe",
    brightCyan: "#a5f3fc",
    brightWhite: "#fafafa",
    extendedAnsi: extendedAnsi({
      22: "#14291c",
      28: "#1a3524",
      65: "#234a30",
      52: "#2c1a1a",
      88: "#3a2222",
      124: "#4a2a2a",
    }),
  },
};

export function terminalThemeFor(theme) {
  return TERMINAL_THEMES[theme] ?? TERMINAL_THEMES.light;
}
