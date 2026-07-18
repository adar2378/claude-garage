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
