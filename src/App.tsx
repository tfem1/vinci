import { useEffect, useState } from "react";
import { Player } from "./components/Player";
import { Sidebar, type Page } from "./components/Sidebar";
import { TitleBar } from "./components/TitleBar";
import { Toasts } from "./components/Toasts";
import { EngineProvider, useEngine } from "./hooks/engine";
import type { Clip } from "./lib/types";
import { Capture } from "./pages/Capture";
import { Dashboard } from "./pages/Dashboard";
import { Library } from "./pages/Library";
import { Settings } from "./pages/Settings";

const TITLES: Record<Page, string> = { dashboard: "Dashboard", library: "Library", capture: "Capture", settings: "Settings" };

function Shell() {
  const initial = (new URLSearchParams(location.search).get("page") as Page) || "dashboard";
  const [page, setPage] = useState<Page>(TITLES[initial] ? initial : "dashboard");
  const [open, setOpen] = useState<Clip | null>(null);
  const { toasts } = useEngine();

  useEffect(() => {
    const k = (e: KeyboardEvent) => {
      const t = e.target as HTMLElement;
      if (t.tagName === "INPUT" || t.tagName === "SELECT" || t.tagName === "TEXTAREA" || open) return;
      const map: Record<string, Page> = { "1": "dashboard", "2": "library", "3": "capture", "4": "settings" };
      if (!e.ctrlKey && !e.altKey && !e.metaKey && map[e.key]) setPage(map[e.key]);
    };
    window.addEventListener("keydown", k);
    return () => window.removeEventListener("keydown", k);
  }, [open]);

  return (
    <div className="app">
      <TitleBar page={TITLES[page]} />
      <Sidebar page={page} onPage={setPage} />
      <main className="main">
        {page === "dashboard" && <Dashboard onOpen={setOpen} onPage={setPage} />}
        {page === "library" && <Library onOpen={setOpen} />}
        {page === "capture" && <Capture />}
        {page === "settings" && <Settings />}
      </main>
      {open && <Player clip={open} onClose={() => setOpen(null)} />}
      <Toasts items={toasts} />
    </div>
  );
}

export default function App() {
  return (
    <EngineProvider>
      <Shell />
    </EngineProvider>
  );
}
