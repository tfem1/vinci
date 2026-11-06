import { Aperture, Film, LayoutGrid, SlidersHorizontal } from "lucide-react";
import { useEngine, useStats } from "../hooks/engine";
import { formatBytes, formatDuration, formatPercent } from "../lib/format";
import { Bar } from "./ui";
import { Wordmark } from "./Logo";

export type Page = "dashboard" | "library" | "capture" | "settings";

const NAV: { id: Page; label: string; icon: typeof LayoutGrid; key: string }[] = [
  { id: "dashboard", label: "Dashboard", icon: LayoutGrid, key: "1" },
  { id: "library", label: "Library", icon: Film, key: "2" },
  { id: "capture", label: "Capture", icon: Aperture, key: "3" },
  { id: "settings", label: "Settings", icon: SlidersHorizontal, key: "4" },
];

export function Sidebar({ page, onPage }: { page: Page; onPage: (p: Page) => void }) {
  const { status, settings, info } = useEngine();
  const stats = useStats(settings?.performance.stats_interval_ms ?? 2000);
  const running = status?.state === "running";
  const pct = status ? (status.buffered_secs / Math.max(1, status.buffer_target_secs)) * 100 : 0;
  const stateLabel = !status ? "…" : status.state === "running" ? "Replay on" : status.state === "starting" ? "Starting" : status.state === "restarting" ? "Recovering" : status.state === "error" ? "Error" : "Replay off";

  return (
    <aside className="sidebar">
      <Wordmark />
      {NAV.map((n) => (
        <button key={n.id} className={`nav-item ${page === n.id ? "active" : ""}`} onClick={() => onPage(n.id)}>
          <n.icon size={16} strokeWidth={1.8} />
          {n.label}
          <span className="kbd">
            <span>{n.key}</span>
          </span>
        </button>
      ))}
      <div className="spacer" />
      <div className="side-status">
        <div className="row">
          <span style={{ display: "flex", alignItems: "center", gap: 8, fontWeight: 550 }}>
            <span className={`dot ${running ? "live" : status?.state === "restarting" || status?.state === "starting" ? "warn" : ""}`} />
            {stateLabel}
          </span>
          <span className="label num">{status ? formatDuration(status.buffered_secs) : "0:00"}</span>
        </div>
        <Bar value={pct} thin muted={!running} />
        <div className="side-meters">
          <div>
            CPU
            <b className="num">{formatPercent(stats?.app_cpu_percent)}</b>
          </div>
          <div>
            RAM
            <b className="num">{stats ? formatBytes(stats.app_ram_bytes, 0) : "—"}</b>
          </div>
        </div>
      </div>
      <div className="faint" style={{ fontSize: 10.5, padding: "8px 8px 0" }}>
        v{info?.version ?? "0.1.0"}
      </div>
    </aside>
  );
}
