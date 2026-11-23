import React from "react";
import ReactDOM from "react-dom/client";

import App from "./App";
import { applyTheme, storedTheme } from "./lib/themes";
import "./styles/index.css";

// Apply the saved theme before the first paint so the window never flashes the
// default palette.
applyTheme(storedTheme());

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <React.StrictMode>
    <App />
  </React.StrictMode>,
);
