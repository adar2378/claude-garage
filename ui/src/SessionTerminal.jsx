import React, { useEffect, useRef } from "react";
import { Terminal } from "@xterm/xterm";
import { FitAddon } from "@xterm/addon-fit";
import { WebLinksAddon } from "@xterm/addon-web-links";
import "@xterm/xterm/css/xterm.css";
import { fetchSessions, openEditor } from "./lib/api.js";
import { useSettings } from "./lib/settings.js";
import { useEffectiveTheme, terminalThemeFor, MONO_STACK } from "./lib/theme.js";

// p7 connection-resilience (design D-reconnect): reconnect backoff caps.
const BACKOFF_BASE_MS = 500;
const BACKOFF_MAX_MS = 8000;

// File-reference detection for terminal output, VS Code style. Two shapes
// only, to keep false positives down: a token containing at least one `/`
// (optionally ./ ../ or absolute), or a bare `name.ext` that carries an
// explicit `:line` suffix. Both accept trailing `:line[:col]`.
const FILE_LINK_RE =
  /(?:\.{1,2}\/|~\/|\/)?[\w.@+-]+(?:\/[\w.@+-]+)+(?::\d+(?::\d+)?)?|[\w@+-]+\.[A-Za-z0-9]{1,8}:\d+(?::\d+)?/g;

// `path:line:col` → { file, line }. Bare paths default to line 1.
function parseFileLink(text) {
  const m = text.match(/^(.*?):(\d+)(?::\d+)?$/);
  if (m) return { file: m[1], line: Number(m[2]) };
  return { file: text, line: 1 };
}

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

    // p9 terminal-links: VS Code-terminal parity. URLs via the stock
    // web-links addon; both it and the file provider below require
    // cmd/ctrl+click (same muscle memory as VS Code) so plain clicks stay
    // free for selection.
    term.loadAddon(
      new WebLinksAddon((e, uri) => {
        if (e.metaKey || e.ctrlKey) window.open(uri, "_blank", "noopener");
      })
    );

    // File references (`ui/src/App.jsx:42`, `./x.py`, `/abs/inside.ts:7:3`)
    // route through the existing /api/open-editor endpoint, which resolves
    // against the workspace dir and rejects escapes — the terminal doesn't
    // validate paths, the daemon does. The session's workspace name isn't a
    // prop (render sites only know `id`), so it's resolved lazily from
    // fetchSessions() on first activation and cached for the mount.
    let wsName;
    const openFileLink = async (text) => {
      const { file, line } = parseFileLink(text);
      try {
        if (wsName === undefined) {
          const sessions = await fetchSessions();
          wsName = sessions.find((s) => s.id === id)?.workspace ?? null;
        }
        if (!wsName) return;
        await openEditor(wsName, file.replace(/^~\//, ""), line);
      } catch (err) {
        // No inline error surface in a terminal cell; 400 (outside the
        // workspace) and 501 (editor CLI missing) just log.
        console.warn(`open-editor link failed: ${err.message}`);
      }
    };
    const linkProvider = term.registerLinkProvider({
      provideLinks(lineNo, cb) {
        const bufLine = term.buffer.active.getLine(lineNo - 1);
        if (!bufLine) return cb(undefined);
        const lineText = bufLine.translateToString(true);
        const links = [];
        for (const m of lineText.matchAll(FILE_LINK_RE)) {
          links.push({
            text: m[0],
            // xterm ranges are 1-based and end-inclusive.
            range: {
              start: { x: m.index + 1, y: lineNo },
              end: { x: m.index + m[0].length, y: lineNo },
            },
            activate(e, text) {
              if (e.metaKey || e.ctrlKey) openFileLink(text);
            },
          });
        }
        cb(links.length ? links : undefined);
      },
    });

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
      // Shift+Enter -> newline in Claude Code's composer. A pty can't see
      // Shift — Enter is just \r — which is why /terminal-setup exists: it
      // teaches iTerm2/VS Code to send Claude Code's escaped-newline
      // sequence (backslash + CR) instead. xterm.js is neither, so we do
      // the same mapping here. IMPORTANT: this handler fires for keydown
      // AND keypress AND keyup of the same chord — every one of them must
      // return false, or xterm processes the leftover keypress and sends a
      // bare \r that submits the prompt anyway. Bytes go out on keydown
      // only. (`ws`/`encoder` are declared below in this effect; the
      // handler only fires after the effect has fully run.)
      if (e.key === "Enter" && e.shiftKey && !e.ctrlKey && !e.metaKey) {
        if (e.type === "keydown" && ws && ws.readyState === WebSocket.OPEN) {
          ws.send(encoder.encode("\\\r"));
        }
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
      linkProvider.dispose();
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
