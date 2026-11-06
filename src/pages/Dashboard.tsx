import { AlertTriangle, ArrowRight, Copy, Gamepad2, Gauge, Loader2, MonitorPlay, Pause, Play, RotateCw } from "lucide-react";
import { useEffect, useState } from "react";
import { ClipCard } from "../components/ClipCard";
import { Kbd } from "../components/ui";
import { useEngine, useStats } from "../hooks/engine";
import { api, errorText } from "../lib/api";
import { codecLabel, encoderLabel, formatBytes, formatDuration, formatPercent, formatSpan, resolutionLabel } from "../lib/format";
import type { Clip } from "../lib/types";
import type { Page } from "../components/Sidebar";

const PRESETS = [30, 60, 120, 180, 300];
const BACKEND_LABEL: Record<string, string> = { wgc: "Windows Graphics Capture", dda: "Desktop Duplication", x11: "X11", portal: "PipeWire (Wayland)", test: "Test pattern", auto: "Auto" };

export function Dashboard({ onOpen, onPage }: { onOpen: (c: Clip) => void; onPage: (p: Page) => void }) {
  const { status, settings, toast, libraryVersion } = useEngine();
  const stats = useStats(settings?.performance.stats_interval_ms ?? 2000);
  const [recent, setRecent] = useState<Clip[]>([]);
  const [busy, setBusy] = useState(false);

  useEffect(() => {
    api.listClips({ sort: "newest", limit: 4 }).then(setRecent).catch(() => {});
  }, [libraryVersion]);

  const state = status?.state ?? "stopped";
  const running = state === "running";
  const active = state === "running" || state === "starting" || state === "restarting";
  const target = status?.buffer_target_secs ?? settings?.replay.buffer_secs ?? 120;
  const buffered = Math.min(status?.buffered_secs ?? 0, target);
  const pct = (buffered / Math.max(1, target)) * 100;
  const s = status?.session;
  const def = settings?.replay.default_clip_secs ?? 30;

  const toggle = async () => {
    setBusy(true);
    try {
      await api.toggleReplay();
    } catch (e) {
      toast({ kind: "error", title: "Replay buffer", detail: errorText(e) });
    }
    setBusy(false);
  };

  const retry = async () => {
    setBusy(true);
    try {
      await api.startReplay();
    } catch (e) {
      toast({ kind: "error", title: "Replay buffer", detail: errorText(e) });
    }
    setBusy(false);
  };

  const label = { running: "Instant replay is on", starting: "Starting capture…", restarting: "Recovering capture…", error: "Capture error", stopped: "Instant replay is off" }[state];
  const gpu = stats?.gpus[0];

  return (
    <div className="page">
      <div className="page-head">
        <div>
          <h1>Dashboard</h1>
          <p>Everything that matters, at a glance.</p>
        </div>
        <button className={`btn lg ${active ? "" : "primary"}`} onClick={toggle} disabled={busy}>
          {busy ? <Loader2 size={15} className="spin" /> : active ? <Pause size={15} /> : <Play size={15} />}
          {active ? "Stop replay" : "Start replay"}
          {settings?.hotkeys.toggle_replay && <Kbd accel={settings.hotkeys.toggle_replay} />}
        </button>
      </div>

      <div className="dash">
        <div className="card hero">
          <div className="hero-top">
            <div>
              <div className="hero-state">
                <span className={`dot ${running ? "live" : state === "starting" || state === "restarting" ? "warn" : ""}`} />
                {label}
              </div>
              <div className="hero-big num">
                {formatDuration(buffered)}
                <small>/ {formatSpan(target)} buffered</small>
              </div>
            </div>
            <div style={{ textAlign: "right", display: "grid", gap: 6, justifyItems: "end" }}>
              <span className="pill solid num">{status?.buffer_storage === "disk" ? "Disk ring" : "RAM"} · {formatBytes(status?.buffer_bytes ?? 0)}</span>
              {s && <span className="pill">{resolutionLabel(s.width, s.height)} · {s.fps} fps · {codecLabel(s.codec)}</span>}
            </div>
          </div>

          <div className="timeline">
            <div className="track">
              <div className="fill" style={{ width: `${pct}%` }} />
              {running && <div className="head" />}
            </div>
            <div className="ticks num">
              <span>−{formatSpan(target)}</span>
              <span>−{formatSpan(target / 2)}</span>
              <span>now</span>
            </div>
          </div>

          {state === "error" && status?.message && (
            <div className="note error fix">
              <AlertTriangle size={14} />
              <div className="fix-body">
                <b>VINCI can’t record your screen yet</b>
                <span>{status.message}</span>
                {status.fix_command && (
                  <>
                    <span>Run this in a terminal, then click Retry:</span>
                    <div className="fix-cmd">
                      <code>{status.fix_command}</code>
                      <button className="btn sm" onClick={() => copyText(status.fix_command!, toast)}>
                        <Copy size={13} /> Copy
                      </button>
                    </div>
                  </>
                )}
                <div>
                  <button className="btn sm primary" onClick={retry} disabled={busy}>
                    {busy ? <Loader2 size={13} className="spin" /> : <RotateCw size={13} />} Retry
                  </button>
                </div>
              </div>
            </div>
          )}
          {status?.fallback_note && (
            <div className="note">
              <AlertTriangle size={14} />
              <span>{status.fallback_note}</span>
            </div>
          )}

          <div className="save-row">
            {PRESETS.map((p) => (
              <button
                key={p}
                className={`save-btn ${p === def ? "default" : ""}`}
                disabled={!running || p > target}
                title={p > target ? `Increase the buffer to ${formatSpan(p)} in Capture` : `Save the last ${formatSpan(p)}`}
                onClick={() => api.saveClip(p).catch((e) => toast({ kind: "error", title: "Clip not saved", detail: errorText(e) }))}
              >
                <b>{formatSpan(p)}</b>
                <span>{p === def ? "default" : buffered >= p ? "ready" : `${formatSpan(Math.max(0, p - buffered))} left`}</span>
              </button>
            ))}
          </div>
          <div className="hint">
            {settings?.hotkeys.save_default ? (
              <>
                Press <Kbd accel={settings.hotkeys.save_default} /> in game to save the last {formatSpan(def)}.
              </>
            ) : (
              "Assign a hotkey in Settings to clip without leaving your game."
            )}
            {status?.saving && <span className="pill solid" style={{ marginLeft: "auto" }}>Saving…</span>}
          </div>
        </div>

        <div className="grid">
          <div className="card pad">
            <div className="card-title">
              <span className="icon">
                <Gamepad2 size={14} /> Game
              </span>
            </div>
            <div className="game-card">
              <div className="game-icon">{status?.game ? status.game.name.slice(0, 1).toUpperCase() : "—"}</div>
              <div style={{ minWidth: 0 }}>
                <b style={{ display: "block", fontWeight: 560, whiteSpace: "nowrap", overflow: "hidden", textOverflow: "ellipsis" }}>{status?.game?.name ?? "No game detected"}</b>
                <span className="muted" style={{ fontSize: 12 }}>
                  {status?.game ? `${status.game.exe}${status.game.foreground ? " · in focus" : ""}` : settings?.general.auto_replay_on_game ? "Replay starts when a game launches" : "Clips are filed under the game you play"}
                </span>
              </div>
            </div>
          </div>

          <div className="card pad">
            <div className="card-title">
              <span className="icon">
                <MonitorPlay size={14} /> Capture
              </span>
              <button className="btn sm ghost" onClick={() => onPage("capture")}>
                Configure
              </button>
            </div>
            <dl className="kv">
              <dt>Source</dt>
              <dd>{s ? `${BACKEND_LABEL[s.backend] ?? s.backend}` : "—"}</dd>
              <dt>Encoder</dt>
              <dd>{s ? encoderLabel(s.encoder) : "—"}</dd>
              <dt>Output</dt>
              <dd className="num">{s ? `${s.width}×${s.height} @ ${s.fps}` : "—"}</dd>
              <dt>Uptime</dt>
              <dd className="num">{status?.since_ms ? formatDuration((Date.now() - status.since_ms) / 1000) : "—"}</dd>
            </dl>
          </div>

          <div className="card pad">
            <div className="card-title">
              <span className="icon">
                <Gauge size={14} /> Resources
              </span>
              <span className="faint" style={{ fontSize: 11 }}>
                VINCI incl. encoder
              </span>
            </div>
            <Meter label="VINCI CPU" value={stats?.app_cpu_percent} right={formatPercent(stats?.app_cpu_percent)} />
            <Meter label="VINCI memory" value={stats ? (stats.app_ram_bytes / 1.5e9) * 100 : undefined} right={stats ? formatBytes(stats.app_ram_bytes) : "—"} />
            <Meter label="System CPU" value={stats?.sys_cpu_percent} right={formatPercent(stats?.sys_cpu_percent)} muted />
            {gpu ? (
              <Meter label={gpu.encoder_percent != null ? `GPU · encoder ${formatPercent(gpu.encoder_percent)}` : "GPU"} value={gpu.util_percent ?? 0} right={formatPercent(gpu.util_percent)} muted />
            ) : (
              <Meter label="GPU" value={0} right="n/a" muted />
            )}
          </div>
        </div>
      </div>

      <div style={{ display: "flex", alignItems: "center", justifyContent: "space-between", margin: "28px 2px 14px" }}>
        <h2 style={{ fontSize: 14, fontWeight: 600, margin: 0 }}>Recent clips</h2>
        <button className="btn sm ghost" onClick={() => onPage("library")}>
          Library <ArrowRight size={13} />
        </button>
      </div>
      {recent.length ? (
        <div className="clip-row">
          {recent.map((c) => (
            <ClipCard key={c.id} clip={c} onOpen={onOpen} />
          ))}
        </div>
      ) : (
        <div className="card empty">
          <b>No clips yet</b>
          <span>Start the replay buffer, play, and save the moment with your hotkey.</span>
        </div>
      )}
    </div>
  );
}

function Meter({ label, value, right, muted }: { label: string; value: number | undefined; right: string; muted?: boolean }) {
  return (
    <div className="meter">
      <div className="top">
        <span>{label}</span>
        <b className="num">{right}</b>
      </div>
      <div className={`bar ${muted ? "muted" : ""}`}>
        <i style={{ width: `${Math.max(1.5, Math.min(100, value ?? 0))}%` }} />
      </div>
    </div>
  );
}

function copyText(text: string, toast: (t: { kind: "ok" | "error" | "info"; title: string; detail?: string }) => void) {
  navigator.clipboard
    .writeText(text)
    .then(() => toast({ kind: "ok", title: "Command copied", detail: "Paste it in a terminal." }))
    .catch(() => toast({ kind: "error", title: "Could not copy", detail: "Select the command and copy it by hand." }));
}
