import React from "react";
import { createRoot } from "react-dom/client";
import App from "./App.jsx";
// p8-theming: Google Sans Code, bundled (OFL) — 400/600/700 cover the
// regular/semibold/bold weights the UI and terminals use.
import "@fontsource/google-sans-code/400.css";
import "@fontsource/google-sans-code/600.css";
import "@fontsource/google-sans-code/700.css";
import "./index.css";

createRoot(document.getElementById("root")).render(<App />);
