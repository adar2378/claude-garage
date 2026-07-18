import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";
import tailwindcss from "@tailwindcss/vite";

const DAEMON = "http://127.0.0.1:4747";

export default defineConfig({
  plugins: [react(), tailwindcss()],
  server: {
    host: "127.0.0.1",
    proxy: {
      "/api": { target: DAEMON },
      "/term": { target: DAEMON, ws: true },
    },
  },
});
