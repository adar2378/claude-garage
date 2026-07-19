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

// xterm.js theme objects, one per palette. The light palette overrides
// the ANSI colors too — xterm's defaults are designed for dark
// backgrounds and are unreadable on ivory.
export const TERMINAL_THEMES = {
  garage: {
    background: "#0b0e14",
    foreground: "#c6cfdb",
    cursor: "#c6cfdb",
    selectionBackground: "#2a3646",
  },
  "claude-dark": {
    background: "#141413",
    foreground: "#c2c0b6",
    cursor: "#c2c0b6",
    selectionBackground: "#3a3937",
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
  },
};

export function terminalThemeFor(theme) {
  return TERMINAL_THEMES[theme] ?? TERMINAL_THEMES.garage;
}
