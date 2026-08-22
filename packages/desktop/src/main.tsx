import { StrictMode } from "react";
import { createRoot } from "react-dom/client";

import { App } from "./App";
import { SettingsProvider } from "./settings/SettingsContext";
import "./styles/global.css";

const root = document.getElementById("root");
if (!root) {
  throw new Error("Settings root element is missing");
}

createRoot(root).render(
  <StrictMode>
    <SettingsProvider>
      <App />
    </SettingsProvider>
  </StrictMode>,
);
