import { Check, Cpu, Download, FolderOpen, Gauge, Keyboard, Layers, RefreshCw, Settings2, Wrench, X } from "lucide-react";
import { useEffect, useState } from "react";
import { HotkeyInput, Range, Seg, Select, Setting, Toggle } from "../components/ui";
import { useEngine } from "../hooks/engine";
import { api, errorText, isTauri } from "../lib/api";
import { formatBytes } from "../lib/format";
import type { AvailableUpdate, Channel, Corner, HotkeyStatus, Priority, Settings as S } from "../lib/types";

const HOTKEYS: { key: keyof S["hotkeys"]; label: string; desc?: string }[] = [
  { key: "save_default", label: "Save clip", desc: "Default length" },
  { key: "save_30", label: "Save last 30 seconds" },
  { key: "save_60", label: "Save last minute" },
  { key: "save_120", label: "Save last 2 minutes" },
  { key: "save_180", label: "Save last 3 minutes" },
  { key: "save_300", label: "Save last 5 minutes" },
  { key: "toggle_replay", label: "Start / stop replay" },
];

export function Settings() {
  const { settings, update, info, hw, refreshHardware, toast } = useEngine();
  const [hk, setHk] = useState<HotkeyStatus | null>(null);
  const [upd, setUpd] = useState<AvailableUpdate | null | undefined>(undefined);
  const [checking, setChecking] = useState(false);
  const [installing, setInstalling] = useState(false);
  const [probing, setProbing] = useState(false);

  useEffect(() => {
    api.hotkeyStatus().then(setHk).catch(() => {});
  }, [settings?.hotkeys]);

  if (!settings) return null;
  const s = settings;

  const check = async () => {
    setChecking(true);
    try {
      setUpd(await api.checkUpdate());
    } catch (e) {
      toast({ kind: "error", title: "Update check failed", detail: errorText(e) });
    }
    setChecking(false);
  };
  const install = async () => {
    if (!upd) return;
    setInstalling(true);
    try {
      const r = await api.installUpdate(upd);
      toast({ kind: "info", title: "Update", detail: r });
    } catch (e) {
      toast({ kind: "error", title: "Update failed", detail: errorText(e) });
    }
    setInstalling(false);
  };
  const reg = (id: string) => hk?.registrations.find((r) => r.id === id);

  return (
    <div className="page">
      <div className="page-head">
        <div>
          <h1>Settings</h1>
          <p>Saved automatically. Your clips and settings are never touched by updates.</p>
        </div>
      </div>

      <div className="section">
        <h2>
          <Settings2 size={14} /> General
        </h2>
        <div className="card pad">
          <Setting title="Launch VINCI at startup" desc="Starts minimized to the tray.">
            <Toggle
              on={s.general.launch_at_startup}
              onChange={async (v) => {
                try {
                  await api.setAutostart(v);
                  await update((x) => void (x.general.launch_at_startup = v));
                } catch (e) {
                  toast({ kind: "error", title: "Autostart", detail: errorText(e) });
                }
              }}
            />
          </Setting>
          <Setting title="Start the replay buffer when VINCI opens">
            <Toggle on={s.general.start_replay_on_launch} onChange={(v) => update((x) => void (x.general.start_replay_on_launch = v))} />
          </Setting>
          <Setting title="Only buffer while a game is running" desc="Starts automatically when a game is detected and stops when it closes.">
            <Toggle on={s.general.auto_replay_on_game} onChange={(v) => update((x) => void (x.general.auto_replay_on_game = v))} />
          </Setting>
          <Setting title="Close button keeps VINCI in the tray" desc="Hotkeys and the replay buffer keep working.">
            <Toggle on={s.general.close_to_tray} onChange={(v) => update((x) => void (x.general.close_to_tray = v))} />
          </Setting>
        </div>
      </div>

      <div className="section">
        <h2>
          <FolderOpen size={14} /> Storage
        </h2>
        <div className="card pad">
          <Setting title="Clips folder" desc={<span className="mono">{info?.clips_dir ?? "—"}</span>}>
            {isTauri && (
              <button
                className="btn sm"
                onClick={async () => {
                  const d = await api.pickFolder("Choose the clips folder");
                  if (d) await update((x) => void (x.library.clips_dir = d));
                }}
              >
                Change…
              </button>
            )}
            <button className="btn sm ghost" onClick={() => api.openClipsFolder()}>
              Open
            </button>
          </Setting>
          <Setting title="One folder per game" desc="e.g. Clips/Valorant/Valorant 2026-11-04 21-03-15.mp4">
            <Toggle on={s.library.organize_by_game} onChange={(v) => update((x) => void (x.library.organize_by_game = v))} />
          </Setting>
        </div>
      </div>

      <div className="section">
        <h2>
          <Keyboard size={14} /> Hotkeys
        </h2>
        <div className="card pad">
          {HOTKEYS.map((h) => {
            const r = reg(h.key);
            return (
              <Setting
                key={h.key}
                title={h.label}
                desc={
                  r && !r.ok ? (
                    <span style={{ color: "#e8b4b4" }}>{r.error}</span>
                  ) : (
                    h.desc ?? (h.key === "toggle_replay" ? undefined : "Works while the game has focus")
                  )
                }
              >
                {r?.ok && <Check size={13} className="muted" />}
                <HotkeyInput value={s.hotkeys[h.key]} validate={api.validateHotkey} onChange={(v) => update((x) => void (x.hotkeys[h.key] = v))} />
              </Setting>
            );
          })}
          {hk?.note && (
            <div className="note" style={{ marginTop: 8 }}>
              <Keyboard size={14} />
              <span>
                {hk.note} Command for custom shortcuts: <span className="mono">vinci --save-clip 30</span>
              </span>
            </div>
          )}
        </div>
      </div>

      <div className="section">
        <h2>
          <Layers size={14} /> In-game overlay
        </h2>
        <div className="card pad">
          <Setting title="Overlay" desc="Discreet notification when a clip is saved. Works over borderless/windowed games.">
            <Toggle on={s.overlay.enabled} onChange={(v) => update((x) => void (x.overlay.enabled = v))} />
          </Setting>
          <Setting title="Position">
            <div className="corner-pick" role="radiogroup" aria-label="Overlay position">
              {(["top_left", "top_right", "bottom_left", "bottom_right"] as Corner[]).map((c) => (
                <button key={c} aria-label={c} className={s.overlay.position === c ? "on" : ""} onClick={() => update((x) => void (x.overlay.position = c))} />
              ))}
            </div>
          </Setting>
          <Setting title="Recording indicator" desc="Small persistent “REPLAY” badge while buffering.">
            <Toggle on={s.overlay.show_indicator} onChange={(v) => update((x) => void (x.overlay.show_indicator = v))} />
          </Setting>
          <Setting title="Opacity">
            <div style={{ width: 180 }}>
              <Range label="Opacity" min={0.3} max={1} step={0.02} value={s.overlay.opacity} onChange={(v) => update((x) => void (x.overlay.opacity = v))} />
            </div>
            <button className="btn sm" onClick={() => api.previewOverlay()}>
              Preview
            </button>
          </Setting>
        </div>
      </div>

      <div className="section">
        <h2>
          <Gauge size={14} /> Performance
        </h2>
        <div className="card pad">
          <Setting title="Resource saver" desc="30 fps cap, Compact profile, slower background polling.">
            <Toggle on={s.performance.resource_saver} onChange={(v) => update((x) => void (x.performance.resource_saver = v))} />
          </Setting>
          <Setting title="Free UI memory when hidden" desc="Closes the interface process when VINCI is in the tray (≈ 60–150 MB saved).">
            <Toggle on={s.performance.free_ui_when_hidden} onChange={(v) => update((x) => void (x.performance.free_ui_when_hidden = v))} />
          </Setting>
          <Setting title="Capture priority" desc="Above normal avoids dropped frames when the game saturates the CPU.">
            <Seg<Priority>
              value={s.performance.capture_priority}
              onChange={(v) => update((x) => void (x.performance.capture_priority = v))}
              options={[
                { value: "below_normal", label: "Low" },
                { value: "normal", label: "Normal" },
                { value: "above_normal", label: "High" },
              ]}
            />
          </Setting>
          <Setting title="RAM limit for the buffer" desc="Above this, Auto storage switches to the disk ring.">
            <Seg<number> value={s.replay.max_ram_mb} onChange={(v) => update((x) => void (x.replay.max_ram_mb = v))} options={[256, 512, 1024, 2048].map((v) => ({ value: v, label: v >= 1024 ? `${v / 1024} GB` : `${v} MB` }))} />
          </Setting>
        </div>
      </div>

      <div className="section">
        <h2>
          <Download size={14} /> Updates
        </h2>
        <div className="card pad">
          <Setting title={`VINCI ${info?.version ?? ""}`} desc={`Installed as ${info?.install_kind ?? "—"} · signed updates from github.com/${info?.repo ?? "tfem1/vinci"}`}>
            <button className="btn sm" onClick={check} disabled={checking}>
              <RefreshCw size={13} className={checking ? "spin" : ""} /> Check now
            </button>
          </Setting>
          {upd !== undefined && (
            <Setting title={upd ? `VINCI ${upd.version} is available` : "You're up to date"} desc={upd?.notes || undefined}>
              {upd && (
                <button className="btn sm primary" onClick={install} disabled={installing}>
                  <Download size={13} /> {installing ? "Installing…" : "Install"}
                </button>
              )}
            </Setting>
          )}
          <Setting title="Check automatically" desc="Daily. Updates are verified (Ed25519 + SHA-256) before installing.">
            <Toggle on={s.updates.auto_check} onChange={(v) => update((x) => void (x.updates.auto_check = v))} />
          </Setting>
          <Setting title="Channel">
            <Seg<Channel> value={s.updates.channel} onChange={(v) => update((x) => void (x.updates.channel = v))} options={[{ value: "stable", label: "Stable" }, { value: "beta", label: "Beta" }]} />
          </Setting>
        </div>
      </div>

      <div className="section">
        <h2>
          <Wrench size={14} /> Advanced
        </h2>
        <div className="card pad">
          <Setting title="Audio offset" desc="Shift audio if it is out of sync (positive = later).">
            <Select<string> label="Audio offset" width={120} value={String(s.advanced.audio_offset_ms)} onChange={(v) => update((x) => void (x.advanced.audio_offset_ms = Number(v)))} options={[-300, -200, -100, -50, 0, 50, 100, 200, 300].map((v) => ({ value: String(v), label: `${v > 0 ? "+" : ""}${v} ms` }))} />
          </Setting>
          <Setting title="Keyframe interval" desc="Cut precision of clips. Shorter = more precise, slightly bigger.">
            <Seg<number> value={s.encoding.keyframe_interval_secs} onChange={(v) => update((x) => void (x.encoding.keyframe_interval_secs = v))} options={[1, 2, 3].map((v) => ({ value: v, label: `${v}s` }))} />
          </Setting>
          <Setting title="Allow CPU encoding fallback" desc="Used only if no GPU encoder works.">
            <Toggle on={s.encoding.allow_software} onChange={(v) => update((x) => void (x.encoding.allow_software = v))} />
          </Setting>
          <Setting title="Logs" desc="Useful when reporting a problem.">
            <button className="btn sm" onClick={() => api.openLogsFolder()}>
              Open logs folder
            </button>
          </Setting>
        </div>
      </div>

      <div className="section">
        <h2>
          <Cpu size={14} /> Diagnostics
        </h2>
        <div className="card pad">
          <dl className="kv" style={{ marginBottom: 14 }}>
            <dt>System</dt>
            <dd>{hw?.report.system.os_name ?? "—"}</dd>
            <dt>CPU</dt>
            <dd>{hw ? `${hw.report.system.cpu_name} · ${hw.report.system.cpu_threads} threads` : "—"}</dd>
            <dt>Memory</dt>
            <dd>{hw ? formatBytes(hw.report.system.ram_total_bytes) : "—"}</dd>
            <dt>Display</dt>
            <dd>{hw ? `${hw.report.system.display_server}${hw.report.system.desktop ? ` · ${hw.report.system.desktop}` : ""}` : "—"}</dd>
            {hw?.report.gpus.map((g) => (
              <span key={g.index} style={{ display: "contents" }}>
                <dt>GPU {g.index}</dt>
                <dd>
                  {g.name}
                  {g.vram_bytes ? ` · ${formatBytes(g.vram_bytes)}` : ""}
                </dd>
              </span>
            ))}
            <dt>FFmpeg</dt>
            <dd className="mono" title={hw?.report.ffmpeg?.path}>{hw?.report.ffmpeg ? `${hw.report.ffmpeg.version} (${hw.report.ffmpeg.source})` : hw?.report.ffmpeg_error ?? "—"}</dd>
          </dl>
          <div className="encoders">
            {(hw?.probes ?? []).map((p) => (
              <div key={p.id} className={`enc ${p.ok ? "" : "off"}`}>
                <span className="st">{p.ok ? <Check size={13} /> : <X size={12} />}</span>
                <span className="mono">{p.id}</span>
                <span className="faint" style={{ marginLeft: "auto", maxWidth: 380, overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }} title={p.error ?? ""}>
                  {p.ok ? `${p.millis} ms` : p.error}
                </span>
              </div>
            ))}
          </div>
          <div style={{ marginTop: 14, display: "flex", gap: 8 }}>
            <button
              className="btn sm"
              disabled={probing}
              onClick={async () => {
                setProbing(true);
                await refreshHardware(true).catch(() => {});
                setProbing(false);
              }}
            >
              <RefreshCw size={13} className={probing ? "spin" : ""} /> Re-detect hardware
            </button>
            <button className="btn sm ghost" onClick={() => navigator.clipboard?.writeText(JSON.stringify(hw, null, 2)).then(() => toast({ kind: "info", title: "Report copied" }))}>
              Copy report
            </button>
          </div>
        </div>
      </div>
    </div>
  );
}
