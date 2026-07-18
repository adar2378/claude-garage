import React, { useEffect, useRef } from "react";
import { Terminal } from "@xterm/xterm";
import { FitAddon } from "@xterm/addon-fit";
import "@xterm/xterm/css/xterm.css";

export default function SessionTerminal({ id }) {
  const hostRef = useRef(null);

  useEffect(() => {
    const term = new Terminal({
      fontFamily: "SF Mono, Menlo, monospace",
      fontSize: 13,
      cursorBlink: true,
      // xterm theme is JS-side; keep in sync with --color-garage-bg in index.css
      theme: { background: "#0b0e14" },
    });
    const fit = new FitAddon();
    term.loadAddon(fit);
    term.open(hostRef.current);
    fit.fit();

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

    const proto = location.protocol === "https:" ? "wss" : "ws";
    const ws = new WebSocket(
      `${proto}://${location.host}/term/${encodeURIComponent(id)}`
    );
    ws.binaryType = "arraybuffer";

    const sendResize = () => {
      if (ws.readyState === WebSocket.OPEN) {
        ws.send(JSON.stringify({ type: "resize", cols: term.cols, rows: term.rows }));
      }
    };

    ws.onopen = sendResize;
    ws.onmessage = (e) => term.write(new Uint8Array(e.data));
    ws.onclose = () => term.write("\r\n[detached]\r\n");

    const encoder = new TextEncoder();
    const dataSub = term.onData((d) => {
      if (ws.readyState === WebSocket.OPEN) ws.send(encoder.encode(d));
    });

    const ro = new ResizeObserver(() => {
      fit.fit();
      sendResize();
    });
    ro.observe(hostRef.current);

    return () => {
      ro.disconnect();
      dataSub.dispose();
      ws.close();
      term.dispose();
    };
  }, [id]);

  return <div ref={hostRef} style={{ height: "100%", width: "100%" }} />;
}
