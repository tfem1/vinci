import { createContext, useCallback, useContext, useEffect, useMemo, useRef, useState, type ReactNode } from "react";
import { api, errorText, isTauri, onEngineEvent } from "../lib/api";
import type { AppInfo, EngineEvent, EngineStatus, HwState, ResourceStats, Settings } from "../lib/types";
import type { Toast } from "../components/Toasts";
import { formatBytes, formatSpan } from "../lib/format";

type Ctx = {
  status: EngineStatus | null;
  settings: Settings | null;
  hw: HwState | null;
  info: AppInfo | null;
  /** Apply a change to settings (optimistic, persisted by the engine). */
  update: (fn: (s: Settings) => void) => Promise<void>;
  refreshHardware: (force: boolean) => Promise<void>;
  toast: (t: Omit<Toast, "id">) => void;
  toasts: Toast[];
  /** Increments whenever the library changes. */
  libraryVersion: number;
  subscribe: (cb: (e: EngineEvent) => void) => () => void;
};

const EngineCtx = createContext<Ctx | null>(null);

export function EngineProvider({ children }: { children: ReactNode }) {
  const [status, setStatus] = useState<EngineStatus | null>(null);
  const [settings, setSettings] = useState<Settings | null>(null);
  const [hw, setHw] = useState<HwState | null>(null);
  const [info, setInfo] = useState<AppInfo | null>(null);
  const [toasts, setToasts] = useState<Toast[]>([]);
  const [libraryVersion, setLibraryVersion] = useState(0);
  const listeners = useRef(new Set<(e: EngineEvent) => void>());
  const settingsRef = useRef<Settings | null>(null);
  settingsRef.current = settings;

  const toast = useCallback((t: Omit<Toast, "id">) => {
    const id = Date.now() + Math.random();
    setToasts((x) => [...x.slice(-3), { ...t, id }]);
    setTimeout(() => setToasts((x) => x.filter((y) => y.id !== id)), t.kind === "error" ? 6500 : 3800);
  }, []);

  useEffect(() => {
    // Subscriptions resolve asynchronously: if the effect is cleaned up first (React
    // StrictMode double-mount), release them as soon as they resolve.
    let cancelled = false;
    const cleanups: (() => void)[] = [];
    const keep = (u: () => void) => (cancelled ? u() : cleanups.push(u));
    api.status().then(setStatus).catch(() => {});
    api.settings().then(setSettings).catch((e) => toast({ kind: "error", title: "Could not load settings", detail: errorText(e) }));
    api.appInfo().then(setInfo).catch(() => {});
    api.hardware(false).then(setHw).catch(() => {});
    onEngineEvent((e) => {
      switch (e.type) {
        case "status": {
          const { type: _t, ...st } = e;
          void _t;
          setStatus(st);
          break;
        }
        case "clip_saved":
          toast({ kind: "ok", title: `Clip saved · ${formatSpan(e.clip.duration)}`, detail: `${e.clip.title} — ${formatBytes(e.clip.size)}` });
          setLibraryVersion((v) => v + 1);
          break;
        case "clip_failed":
          toast({ kind: "error", title: "Clip not saved", detail: e.message });
          break;
        case "library_changed":
          setLibraryVersion((v) => v + 1);
          break;
        case "job_done":
          if (e.error) toast({ kind: "error", title: e.kind === "trim" ? "Trim failed" : "Compression failed", detail: e.error });
          else if (e.clip) toast({ kind: "ok", title: e.kind === "trim" ? "Clip trimmed" : "Clip compressed", detail: `${e.clip.title} — ${formatBytes(e.clip.size)}` });
          break;
      }
      listeners.current.forEach((l) => l(e));
    }).then(keep);
    if (isTauri) {
      import("@tauri-apps/api/event").then(({ listen }) =>
        listen<{ version: string }>("vinci://update-available", (e) => toast({ kind: "info", title: `VINCI ${e.payload.version} is available`, detail: "Install it from Settings → Updates." })).then(keep),
      );
    }
    return () => {
      cancelled = true;
      cleanups.forEach((u) => u());
    };
  }, [toast]);

  const update = useCallback(
    async (fn: (s: Settings) => void) => {
      const cur = settingsRef.current;
      if (!cur) return;
      const next = structuredClone(cur);
      fn(next);
      setSettings(next);
      try {
        const saved = await api.updateSettings(next);
        setSettings(saved);
      } catch (e) {
        toast({ kind: "error", title: "Settings not applied", detail: errorText(e) });
        setSettings(cur);
      }
    },
    [toast],
  );

  const refreshHardware = useCallback(async (force: boolean) => {
    setHw(await api.hardware(force));
  }, []);

  const subscribe = useCallback((cb: (e: EngineEvent) => void) => {
    listeners.current.add(cb);
    return () => listeners.current.delete(cb);
  }, []);

  const value = useMemo(() => ({ status, settings, hw, info, update, refreshHardware, toast, toasts, libraryVersion, subscribe }), [status, settings, hw, info, update, refreshHardware, toast, toasts, libraryVersion, subscribe]);
  return <EngineCtx.Provider value={value}>{children}</EngineCtx.Provider>;
}

export function useEngine(): Ctx {
  const c = useContext(EngineCtx);
  if (!c) throw new Error("useEngine outside EngineProvider");
  return c;
}

// One shared poller: CPU usage is a delta between two samples, so several components
// polling independently would corrupt each other's readings. Polling only runs while
// at least one component is subscribed and the window is visible.
let statsValue: ResourceStats | null = null;
const statsSubs = new Set<(s: ResourceStats) => void>();
let statsTimer: ReturnType<typeof setInterval> | null = null;
let statsInterval = 2000;

function ensureStatsTimer(intervalMs: number) {
  if (statsTimer && intervalMs === statsInterval) return;
  if (statsTimer) clearInterval(statsTimer);
  statsInterval = intervalMs;
  const tick = () => {
    if (document.visibilityState !== "visible") return;
    api.stats().then((x) => {
      statsValue = x;
      statsSubs.forEach((f) => f(x));
    }).catch(() => {});
  };
  tick();
  statsTimer = setInterval(tick, intervalMs);
}

export function useStats(intervalMs: number): ResourceStats | null {
  const [s, setS] = useState<ResourceStats | null>(statsValue);
  useEffect(() => {
    statsSubs.add(setS);
    ensureStatsTimer(intervalMs);
    return () => {
      statsSubs.delete(setS);
      if (statsSubs.size === 0 && statsTimer) {
        clearInterval(statsTimer);
        statsTimer = null;
      }
    };
  }, [intervalMs]);
  return s;
}
