import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import "./styles/app.css";
import App from "./App";
import { Overlay } from "./Overlay";
import { initFileSrc, isTauri } from "./lib/api";

async function boot() {
  await initFileSrc();
  let label = "main";
  if (isTauri) {
    const { getCurrentWindow } = await import("@tauri-apps/api/window");
    label = getCurrentWindow().label;
  }
  // Disable the default browser context menu & text selection feel of a web page.
  window.addEventListener("contextmenu", (e) => {
    if (!(e.target as HTMLElement).closest("input, textarea")) e.preventDefault();
  });
  // While you play, the VINCI window is unfocused: stop its animations so the
  // webview does not keep repainting at the screen's refresh rate next to the game.
  if (label !== "overlay") {
    const idle = () => document.documentElement.classList.toggle("idle", document.hidden || !document.hasFocus());
    window.addEventListener("focus", idle);
    window.addEventListener("blur", idle);
    document.addEventListener("visibilitychange", idle);
    idle();
  }
  createRoot(document.getElementById("root")!).render(<StrictMode>{label === "overlay" ? <Overlay /> : <App />}</StrictMode>);
}

boot();
