import "@fontsource-variable/nunito";
import "./index.css";
import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import App from "./App";
import { applyTheme, storedTheme } from "./components/FamilyView";
import { api } from "./lib/api";

applyTheme(storedTheme());

window.addEventListener(
  "error",
  (e) => void api.log("error", `${e.message} at ${e.filename}:${e.lineno}`).catch(() => {}),
);
window.addEventListener(
  "unhandledrejection",
  (e) => void api.log("error", `unhandled: ${String(e.reason)}`).catch(() => {}),
);

createRoot(document.getElementById("root")!).render(
  <StrictMode>
    <App />
  </StrictMode>,
);
