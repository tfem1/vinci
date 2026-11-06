import { AlertTriangle, ArrowRight, ChevronRight, Copy, FolderOpen, Gamepad2, Gauge, Keyboard, Loader2, MonitorPlay, Pause, Play, RotateCw, Scissors } from "lucide-react";
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
  const starting = state === "starting" || state === "restarting";
  const active = running || starting;
  const target = status?.buffer_target_secs ?? settings?.replay.buffer_secs ?? 120;
  const buffered = Math.min(status?.buffered_secs ?? 0, target);
  const pct = (buffered / Math.max(1, target)) * 100;
  const s = status?.session;
  const def = settings?.replay.default_clip_secs ?? 30;
  const saveKey = settings?.hotkeys.save_default ?? null;
  const toggleKey = settings?.hotkeys.toggle_replay ?? null;
  const otherLengths = PRESETS.filter((p) => p !== def && p <= target);

  const run = async (f: () => Promise<unknown>) => {
    setBusy(true);
    try {
      await f();
    } catch (e) {
      toast({ kind: "error", title: "Recording", detail: errorText(e) });
    }
    setBusy(false);
  };
  const save = (secs: number) => api.saveClip(secs).catch((e) => toast({ kind: "error", title: "Clip not saved", detail: errorText(e) }));
  const gpu = stats?.gpus[0];

  return (
    <div className="page">
      <div className="page-head">
        <div>
          <h1>Dashboard</h1>
          <p>Clip the last moments of your game with one key.</p>
        </div>
        {active && (
          <button className="btn" onClick={() => run(api.toggleReplay)} disabled={busy}>
            <Pause size={14} /> Stop recording
            {toggleKey && <Kbd accel={toggleKey} />}
          </button>
        )}
      </div>

      {state === "error" && status?.message ? (
        <div className="card hero simple">
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
                <button className="btn sm primary" onClick={() => run(api.startReplay)} disabled={busy}>
                  {busy ? <Loader2 size={13} className="spin" /> : <RotateCw size={13} />} Retry
                </button>
              </div>
            </div>
          </div>
        </div>
      ) : (
        <div className="card hero simple">
          <div>
            <div className="hero-title">
              <span className={`dot ${running ? "live" : starting ? "warn" : ""}`} />
              {running ? "Recording" : starting ? "Starting…" : "Recording is off"}
            </div>
            <p className="hero-sub">
              {running ? (
                <>
                  VINCI keeps the last <b>{formatSpan(target)}</b> in memory — nothing is saved until you clip.
                  {saveKey && (
                    <>
                      {" "}
                      In game, press <Kbd accel={saveKey} /> to save the last <b>{formatSpan(def)}</b>.
                    </>
                  )}
                </>
              ) : starting ? (
                "Getting your screen from the desktop…"
              ) : (
                "Start recording so you can clip what just happened."
              )}
            </p>
          </div>

          {active && (
            <div className="timeline">
              <div className="track">
                <div className="fill" style={{ width: `${pct}%` }} />
                {running && <div className="head" />}
              </div>
              <div className="ticks num">
                <span>−{formatSpan(target)}</span>
                <span>{formatDuration(buffered)} ready</span>
                <span>now</span>
              </div>
            </div>
          )}

          <div className="hero-actions">
            {active ? (
              <button className="btn xl primary" disabled={!running || status?.saving} onClick={() => save(def)}>
                {status?.saving ? <Loader2 size={16} className="spin" /> : <Scissors size={16} />}
                Save the last {formatSpan(def)}
                {saveKey && <Kbd accel={saveKey} />}
              </button>
            ) : (
              <button className="btn xl primary" onClick={() => run(api.startReplay)} disabled={busy}>
                {busy ? <Loader2 size={16} className="spin" /> : <Play size={16} />} Start recording
                {toggleKey && <Kbd accel={toggleKey} />}
              </button>
            )}
            {running && otherLengths.length > 0 && (
              <div className="more-lengths">
                <span>or</span>
                {otherLengths.map((p) => (
                  <button key={p} className="btn sm" title={`Save the last ${formatSpan(p)} (or everything recorded so far)`} onClick={() => save(p)}>
                    {formatSpan(p)}
                  </button>
                ))}
              </div>
            )}
          </div>

          {status?.fallback_note && (
            <div className="note">
              <AlertTriangle size={14} />
              <span>{status.fallback_note}</span>
            </div>
          )}
        </div>
      )}

      {!recent.length && (
        <div className="steps">
          <Step n={1} icon={<Gamepad2 size={16} />} title="Play normally">
            VINCI keeps the last {formatSpan(target)} of your screen in memory with your graphics card’s video encoder. Nothing is written to disk.
          </Step>
          <Step n={2} icon={<Keyboard size={16} />} title={saveKey ? <>Press <Kbd accel={saveKey} /></> : "Press your clip key"}>
            The last {formatSpan(def)} become a clip, instantly — even in a full-screen game.
          </Step>
          <Step n={3} icon={<FolderOpen size={16} />} title="Watch & share">
            Your clips are in Library (and in Videos/VINCI), sorted by game.
          </Step>
        </div>
      )}

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
          <span>{running && saveKey ? <>Press <Kbd accel={saveKey} /> in game to make your first one.</> : "Start recording, play, and clip the moment."}</span>
        </div>
      )}

      <details className="card details">
        <summary>
          <ChevronRight size={14} className="chev" /> Details — game, capture, performance
        </summary>
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
                  {status?.game ? `${status.game.exe}${status.game.foreground ? " · in focus" : ""}` : "Clips are filed under the game you play"}
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
              <dt>Screen</dt>
              <dd>{s?.monitor ?? "—"}</dd>
              <dt>Encoder</dt>
              <dd>{s ? encoderLabel(s.encoder) : "—"}</dd>
              <dt>Output</dt>
              <dd className="num">{s ? `${resolutionLabel(s.width, s.height)} · ${s.fps} fps · ${codecLabel(s.codec)}` : "—"}</dd>
              <dt>Memory</dt>
              <dd className="num">
                {status?.buffer_storage === "disk" ? "Disk ring" : "RAM"} · {formatBytes(status?.buffer_bytes ?? 0)}
              </dd>
            </dl>
          </div>

          <div className="card pad">
            <div className="card-title">
              <span className="icon">
                <Gauge size={14} /> Performance
              </span>
              <span className="faint" style={{ fontSize: 11 }}>
                VINCI incl. encoder
              </span>
            </div>
            <Meter label="VINCI CPU" value={stats?.app_cpu_percent} right={formatPercent(stats?.app_cpu_percent)} />
            <Meter label="VINCI memory" value={stats ? (stats.app_ram_bytes / 1.5e9) * 100 : undefined} right={stats ? formatBytes(stats.app_ram_bytes) : "—"} />
            {gpu ? (
              <Meter label={gpu.encoder_percent != null ? `GPU video encoder` : "GPU"} value={gpu.encoder_percent ?? gpu.util_percent ?? 0} right={formatPercent(gpu.encoder_percent ?? gpu.util_percent)} muted />
            ) : (
              <Meter label="GPU" value={0} right="n/a" muted />
            )}
          </div>
        </div>
      </details>
    </div>
  );
}

function Step({ n, icon, title, children }: { n: number; icon: React.ReactNode; title: React.ReactNode; children: React.ReactNode }) {
  return (
    <div className="card pad step">
      <div className="step-head">
        <span className="step-n">{n}</span>
        {icon}
        <b>{title}</b>
      </div>
      <p>{children}</p>
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
