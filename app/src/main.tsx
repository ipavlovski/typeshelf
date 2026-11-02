import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { App } from "./App";
import "./styles.css";
import { openUrl } from "@tauri-apps/plugin-opener";

// Web links (release notes, GitHub) open in the default browser, not inside the app.
document.addEventListener("click", (e) => {
  const a = (e.target as Element).closest?.("a[href]") as HTMLAnchorElement | null;
  if (a && /^https?:/.test(a.href)) {
    e.preventDefault();
    void openUrl(a.href);
  }
});

createRoot(document.getElementById("root")!).render(
  <StrictMode>
    <App />
  </StrictMode>,
);
