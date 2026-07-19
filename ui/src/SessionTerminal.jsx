import React, { useEffect, useRef } from "react";
import { Terminal } from "@xterm/xterm";
import { FitAddon } from "@xterm/addon-fit";
import "@xterm/xterm/css/xterm.css";
import { fetchSessions } from "./lib/api.js";
import { useSettings } from "./lib/settings.js";
import { useEffectiveTheme, terminalThemeFor, MONO_STACK } from "./lib/theme.js";

// p7 connection-resilience (design D-reconnect): reconnect backoff caps.
const BACKOFF_BASE_MS = 500;
const BACKOFF_MAX_MS = 8000;

export default function SessionTerminal({ id, onConnectionChange, reconnectSignal = 0 }) {
  const hostRef = useRef(null);
  // Imperative per-mount bundle so the reconnectSignal effect below can
  // trigger an immediate attempt without tearing the terminal down.
  const connRef = useRef(null);
  // Latest callback without making it an effect dep — the whole
  // terminal/WS lifecycle must key on `id` alone.
  const onConnectionChangeRef = useRef(onConnectionChange);
  useEffect(() => {
    onConnectionChangeRef.current = onConnectionChange;
  }, [onConnectionChange]);

  // p8-theming: xterm paints a canvas — CSS variables can't reach it, so
  // the theme object comes from lib/theme.js and is swapped live below
  // without recreating the terminal.
  const [settings] = useSettings();
  const theme = useEffectiveTheme(settings.theme);
  const termRef = useRef(null);
  const themeRef = useRef(theme);
  useEffect(() => {
    themeRef.current = theme;
  }, [theme]);

  useEffect(() => {
    const term = new Terminal({
      fontFamily: MONO_STACK,
      fontSize: 13,
      cursorBlink: true,
      theme: terminalThemeFor(themeRef.current),
    });
    termRef.current = term;
    const fit = new FitAddon();
    term.loadAddon(fit);
    term.open(hostRef.current);
    fit.fit();

    // Google Sans Code loads async (@fontsource @font-face) — if xterm
    // measured cell metrics against the fallback font before it arrived,
    // refit once fonts settle so glyph widths are correct.
    if (document.fonts?.ready) {
      document.fonts.ready.then(() => {
        if (!disposed) {
          fit.fit();
          sendResize();
        }
      });
    }

    // design D-blur-chord: Ctrl+` is the only keyboard-only path back to
    // chrome-navigation mode while a terminal has DOM focus. App.jsx's
    // window-level keydown listener can't intercept this — xterm.js
    // handles keydown on its own internal node before that listener would
    // ever see it — so it has to be caught here, per xterm.js instance,
    // via attachCustomKeyEventHandler. Returning false suppresses xterm's
    // own processing of that one keystroke; every other key returns true
    // and is left untouched.
    term.attachCustomKeyEventHandler((e) => {
      if (e.type === "keydown" && e.ctrlKey && e.key === "`") {
        document.activeElement?.blur();
        return false;
      }
      return true;
    });

    // ---- p7 connection-resilience: the WS is no longer one-shot. On
    // close (sleep, daemon restart) we retry with capped exponential
    // backoff instead of writing a terminal "[detached]" end state that
    // used to persist until a full page reload. The xterm instance
    // survives across reconnects (scrollback intact); tmux repaints the
    // pane on reattach.
    let ws = null;
    let disposed = false;
    let attempt = 0;
    let retryTimer = null;

    const sendResize = () => {
      if (ws && ws.readyState === WebSocket.OPEN) {
        ws.send(JSON.stringify({ type: "resize", cols: term.cols, rows: term.rows }));
      }
    };

    const encoder = new TextEncoder();
    const dataSub = term.onData((d) => {
      if (ws && ws.readyState === WebSocket.OPEN) ws.send(encoder.encode(d));
    });

    function connect() {
      if (disposed) return;
      const proto = location.protocol === "https:" ? "wss" : "ws";
      ws = new WebSocket(`${proto}://${location.host}/term/${encodeURIComponent(id)}`);
      ws.binaryType = "arraybuffer";
      ws.onopen = () => {
        attempt = 0;
        onConnectionChangeRef.current?.(true);
        sendResize();
      };
      ws.onmessage = (e) => term.write(new Uint8Array(e.data));
      ws.onclose = () => {
        if (disposed) return;
        onConnectionChangeRef.current?.(false);
        scheduleRetry();
      };
    }

    function scheduleRetry() {
      const delay = Math.min(BACKOFF_MAX_MS, BACKOFF_BASE_MS * 2 ** attempt);
      attempt += 1;
      clearTimeout(retryTimer);
      retryTimer = setTimeout(async () => {
        if (disposed) return;
        // Existence gate (design D-reconnect): if the daemon is reachable
        // but no longer lists this session, stop retrying — the session
        // was killed and reconcile() is about to drop this panel anyway.
        // A *failed* fetch means the daemon itself is down, which is
        // exactly the case worth retrying, so fall through on error.
        try {
          const sessions = await fetchSessions();
          if (disposed) return;
          if (!sessions.some((s) => s.id === id)) return;
        } catch {
          // daemon unreachable — keep retrying the socket
        }
        connect();
      }, delay);
    }

    connRef.current = {
      // Manual "reconnect now" from the cell overlay: skip the pending
      // backoff wait and try immediately (unless a socket is already
      // open/connecting).
      reconnectNow() {
        if (disposed) return;
        if (ws && (ws.readyState === WebSocket.OPEN || ws.readyState === WebSocket.CONNECTING)) {
          return;
        }
        clearTimeout(retryTimer);
        attempt = 0;
        connect();
      },
    };

    connect();

    const ro = new ResizeObserver(() => {
      fit.fit();
      sendResize();
    });
    ro.observe(hostRef.current);

    return () => {
      disposed = true;
      clearTimeout(retryTimer);
      connRef.current = null;
      termRef.current = null;
      ro.disconnect();
      dataSub.dispose();
      if (ws) ws.close();
      term.dispose();
    };
  }, [id]);

  // Live theme swap — xterm re-renders in place when options.theme is
  // reassigned; no terminal recreation, scrollback intact.
  useEffect(() => {
    if (termRef.current) termRef.current.options.theme = terminalThemeFor(theme);
  }, [theme]);

  useEffect(() => {
    if (reconnectSignal > 0) connRef.current?.reconnectNow();
  }, [reconnectSignal]);

  return <div ref={hostRef} style={{ height: "100%", width: "100%" }} />;
}
