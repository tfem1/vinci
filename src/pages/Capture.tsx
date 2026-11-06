import { AudioLines, Check, Clapperboard, HardDrive, Monitor, Timer, X } from "lucide-react";
import { useEffect, useState } from "react";
import { Range, Seg, Select, Setting, Toggle } from "../components/ui";
import { useEngine } from "../hooks/engine";
import { api } from "../lib/api";
import { codecLabel, formatBytes, formatSpan } from "../lib/format";
import type { AudioDevice, BackendKind, BufferStorage, Codec, CodecChoice, Estimate, MonitorInfo, QualityProfile, Resolution } from "../lib/types";

const PROFILES: { id: QualityProfile; name: string; desc: string }[] = [
  { id: "quality", name: "Quality", desc: "Visually lossless, fast motion" },
  { id: "balanced", name: "Balanced", desc: "Sharp and light — recommended" },
  { id: "compact", name: "Compact", desc: "Smallest files for sharing" },
];

export function Capture() {
  const { settings, update, hw, status } = useEngine();
  const [monitors, setMonitors] = useState<MonitorInfo[]>([]);
  const [devices, setDevices] = useState<AudioDevice[]>([]);
  const [est, setEst] = useState<Estimate | null>(null);
  const [buf, setBuf] = useState<number | null>(null);

  useEffect(() => {
    api.monitors().then(setMonitors).catch(() => {});
    api.audioDevices().then(setDevices).catch(() => {});
  }, []);
  useEffect(() => {
    if (settings) api.estimate(settings).then(setEst).catch(() => {});
  }, [settings]);

  if (!settings) return null;
  const s = settings;
  const ff = hw?.report.ffmpeg;
  const os = hw?.report.system.os_family ?? "linux";
  const display = hw?.report.system.display_server;
  const working = new Set((hw?.probes ?? []).filter((p) => p.ok).map((p) => p.id));
  const hasHw = (c: Codec) => (hw?.probes ?? []).some((p) => p.ok && p.codec === c && p.family !== "software");
  const has = (c: Codec) => (hw?.probes ?? []).some((p) => p.ok && p.codec === c);

  const backends: { value: BackendKind; label: string; disabled?: boolean; title?: string }[] =
    os === "windows"
      ? [
          { value: "auto", label: "Auto" },
          { value: "wgc", label: "Graphics Capture", disabled: !!ff && !ff.filters.includes("gfxcapture"), title: "Windows.Graphics.Capture — window or monitor, borderless games" },
          { value: "dda", label: "Desktop Duplication", disabled: !!ff && !ff.filters.includes("ddagrab"), title: "DXGI — captures exclusive-fullscreen games" },
        ]
      : [
          { value: "auto", label: "Auto" },
          { value: "x11", label: "X11", disabled: display === "wayland" && !ff?.input_devices.includes("x11grab") },
          { value: "portal", label: "PipeWire", disabled: !hw?.report.gstreamer?.elements.includes("pipewiresrc"), title: "Wayland: xdg-desktop-portal + PipeWire" },
        ];

  const mbm = (p: QualityProfile) => {
    if (!est) return "";
    const ratio = { quality: 1.75, balanced: 1, compact: 0.62 }[p] / { quality: 1.75, balanced: 1, compact: 0.62 }[s.encoding.profile];
    return `≈ ${Math.round(est.mb_per_minute * ratio)} MB/min`;
  };

  const encoders = (hw?.probes ?? []).filter((p) => p.ok);
  const bufferVal = buf ?? s.replay.buffer_secs;

  return (
    <div className="page">
      <div className="page-head">
        <div>
          <h1>Capture</h1>
          <p>Source, quality and replay buffer. Changes apply instantly.</p>
        </div>
      </div>
      <div className="two">
        <div>
          <div className="section">
            <h2>
              <Monitor size={14} /> Source
            </h2>
            <div className="card pad">
              <Setting title="Capture method" desc={os === "windows" ? "Native Windows APIs — no injection, anti-cheat safe." : display === "wayland" ? "Wayland uses the screen-sharing portal (asked once)." : "Native X11 shared-memory capture."}>
                <Seg value={s.capture.backend === "test" ? "auto" : s.capture.backend} options={backends} onChange={(v) => update((x) => void (x.capture.backend = v))} />
              </Setting>
              <Setting title="Display" desc={monitors.length ? `${monitors.length} detected` : display === "wayland" ? "Chosen in the portal dialog" : "—"}>
                <Select<string>
                  label="Display"
                  width={260}
                  value={s.capture.target.kind === "monitor" ? String(s.capture.target.index) : s.capture.target.kind}
                  onChange={(v) =>
                    update((x) => {
                      x.capture.target = v === "follow_game" ? { kind: "follow_game" } : { kind: "monitor", index: Number(v) };
                    })
                  }
                  options={[
                    ...monitors.map((m) => ({ value: String(m.index), label: `${m.name} · ${m.width}×${m.height}${m.primary ? " (main)" : ""}` })),
                    ...(os === "windows" ? [{ value: "follow_game", label: "Follow the game window" }] : []),
                    ...(monitors.length === 0 ? [{ value: "0", label: "Primary display" }] : []),
                  ]}
                />
              </Setting>
              <Setting title="Show cursor">
                <Toggle on={s.capture.cursor} onChange={(v) => update((x) => void (x.capture.cursor = v))} />
              </Setting>
            </div>
          </div>

          <div className="section">
            <h2>
              <Clapperboard size={14} /> Video
            </h2>
            <div className="card pad">
              <div className="profiles" style={{ marginBottom: 14 }}>
                {PROFILES.map((p) => (
                  <button key={p.id} className={`profile ${s.encoding.profile === p.id ? "on" : ""}`} onClick={() => update((x) => void (x.encoding.profile = p.id))}>
                    <b>{p.name}</b>
                    <span>{p.desc}</span>
                    <em className="num">{mbm(p.id)}</em>
                  </button>
                ))}
              </div>
              <Setting title="Resolution" desc="Never upscales; keeps your aspect ratio.">
                <Seg<Resolution>
                  value={s.capture.resolution}
                  onChange={(v) => update((x) => void (x.capture.resolution = v))}
                  options={[
                    { value: "native", label: "Native" },
                    { value: "p720", label: "720p" },
                    { value: "p1080", label: "1080p" },
                    { value: "p1440", label: "1440p" },
                    { value: "p2160", label: "4K" },
                  ]}
                />
              </Setting>
              <Setting title="Frame rate">
                <Seg<number>
                  value={s.capture.fps}
                  onChange={(v) => update((x) => void (x.capture.fps = v))}
                  options={[30, 60, 120, 144].map((f) => ({ value: f, label: `${f}`, title: f > 60 ? "Requires a fast GPU encoder" : undefined }))}
                />
              </Setting>
              <Setting title="Codec" desc="Auto picks HEVC (AV1 for Compact) when your GPU can encode it.">
                <Seg<CodecChoice>
                  value={s.encoding.codec}
                  onChange={(v) => update((x) => void (x.encoding.codec = v))}
                  options={[
                    { value: "auto", label: "Auto" },
                    { value: "h264", label: "H.264", disabled: !!hw && !has("h264") },
                    { value: "hevc", label: hasHw("hevc") || !hw ? "HEVC" : "HEVC (CPU)", disabled: !!hw && !has("hevc") },
                    { value: "av1", label: hasHw("av1") || !hw ? "AV1" : "AV1 (CPU)", disabled: !!hw && !has("av1") },
                  ]}
                />
              </Setting>
              <Setting title="Encoder" desc="Only encoders validated on this PC are listed.">
                <Select<string>
                  label="Encoder"
                  width={230}
                  value={s.encoding.encoder ?? ""}
                  onChange={(v) => update((x) => void (x.encoding.encoder = v || null))}
                  options={[{ value: "", label: "Automatic (best GPU)" }, ...encoders.map((p) => ({ value: p.id, label: `${p.id}${p.family === "software" ? " — CPU" : ""}` }))]}
                />
              </Setting>
            </div>
          </div>

          <div className="section">
            <h2>
              <Timer size={14} /> Replay buffer
            </h2>
            <div className="card pad">
              <Setting title="Buffer length" desc="How far back you can save. Changing it keeps the footage already buffered.">
                <span className="num" style={{ fontWeight: 600, minWidth: 54, textAlign: "right" }}>
                  {formatSpan(bufferVal)}
                </span>
              </Setting>
              <div style={{ padding: "0 0 14px" }}>
                <Range label="Buffer length" min={15} max={600} step={15} value={bufferVal} onChange={setBuf} onCommit={(v) => { setBuf(null); update((x) => void (x.replay.buffer_secs = v)); }} />
                <div style={{ display: "flex", gap: 6, marginTop: 12 }}>
                  {[30, 60, 120, 180, 300].map((v) => (
                    <button key={v} className={`chip ${s.replay.buffer_secs === v ? "on" : ""}`} onClick={() => update((x) => void (x.replay.buffer_secs = v))}>
                      {formatSpan(v)}
                    </button>
                  ))}
                </div>
              </div>
              <Setting title="Default clip length" desc="Saved by the main hotkey.">
                <Seg<number>
                  value={s.replay.default_clip_secs}
                  onChange={(v) => update((x) => void (x.replay.default_clip_secs = v))}
                  options={[15, 30, 60, 120, 300].map((v) => ({ value: v, label: formatSpan(v), disabled: v > s.replay.buffer_secs }))}
                />
              </Setting>
              <Setting title="Storage" desc="RAM is fastest; the disk ring uses ~0 RAM (one preallocated file, deleted on exit).">
                <Seg<BufferStorage>
                  value={s.replay.storage}
                  onChange={(v) => update((x) => void (x.replay.storage = v))}
                  options={[
                    { value: "auto", label: "Auto" },
                    { value: "memory", label: "RAM" },
                    { value: "disk", label: "Disk" },
                  ]}
                />
              </Setting>
            </div>
          </div>

          <div className="section">
            <h2>
              <AudioLines size={14} /> Audio
            </h2>
            <div className="card pad">
              <Setting title="Game & desktop audio" desc="Everything you hear.">
                {s.capture.audio.system && (
                  <Select<string>
                    label="Output device"
                    width={200}
                    value={s.capture.audio.system_device ?? ""}
                    onChange={(v) => update((x) => void (x.capture.audio.system_device = v || null))}
                    options={[{ value: "", label: "Default output" }, ...devices.filter((d) => !d.input).map((d) => ({ value: d.id, label: d.name }))]}
                  />
                )}
                <Toggle on={s.capture.audio.system} onChange={(v) => update((x) => void (x.capture.audio.system = v))} />
              </Setting>
              <Setting title="Microphone" desc="Mixed into the clip.">
                {s.capture.audio.microphone && (
                  <Select<string>
                    label="Microphone"
                    width={200}
                    value={s.capture.audio.mic_device ?? ""}
                    onChange={(v) => update((x) => void (x.capture.audio.mic_device = v || null))}
                    options={[{ value: "", label: "Default microphone" }, ...devices.filter((d) => d.input).map((d) => ({ value: d.id, label: d.name }))]}
                  />
                )}
                <Toggle on={s.capture.audio.microphone} onChange={(v) => update((x) => void (x.capture.audio.microphone = v))} />
              </Setting>
              {s.capture.audio.microphone && (
                <Setting title="Microphone volume">
                  <div style={{ width: 200 }}>
                    <Range label="Microphone volume" min={0} max={2} step={0.05} value={s.capture.audio.mic_volume} onChange={(v) => update((x) => void (x.capture.audio.mic_volume = v))} />
                  </div>
                  <span className="num muted" style={{ width: 40, textAlign: "right" }}>
                    {Math.round(s.capture.audio.mic_volume * 100)}%
                  </span>
                </Setting>
              )}
            </div>
          </div>
        </div>

        <div className="sticky grid">
          <div className="card pad">
            <div className="card-title">
              <span className="icon">
                <HardDrive size={14} /> Footprint
              </span>
            </div>
            <div className="est-big num">{est ? `${Math.round(est.mb_per_minute)} MB` : "—"}</div>
            <div className="muted" style={{ marginBottom: 14 }}>
              per minute · {est ? `${est.width}×${est.height} @ ${est.fps} · ${codecLabel(est.codec)}` : ""}
            </div>
            <dl className="kv">
              <dt>Buffer</dt>
              <dd className="num">
                {est ? `${formatBytes(est.buffer_mb * 1e6)} in ${est.buffer_storage === "disk" ? "disk ring" : "RAM"}` : "—"}
              </dd>
              {est?.clip_mb.map(([secs, mb]) => (
                <span key={secs} style={{ display: "contents" }}>
                  <dt>{formatSpan(secs)} clip</dt>
                  <dd className="num">{formatBytes(mb * 1e6)}</dd>
                </span>
              ))}
            </dl>
            {status?.session && (
              <div className="faint" style={{ fontSize: 11.5, marginTop: 14 }}>
                Live: {status.session.encoder} · {status.session.width}×{status.session.height}@{status.session.fps}
              </div>
            )}
          </div>
          <div className="card pad">
            <div className="card-title">
              <span>Encoders on this PC</span>
            </div>
            <div className="encoders">
              {(hw?.probes ?? []).map((p) => (
                <div key={p.id} className={`enc ${p.ok ? "" : "off"}`} title={p.error ?? ""}>
                  <span className="st">{p.ok ? <Check size={13} /> : <X size={12} />}</span>
                  <span className="mono">{p.id}</span>
                  <span className="faint" style={{ marginLeft: "auto" }}>
                    {p.family === "software" ? "CPU" : "GPU"}
                  </span>
                </div>
              ))}
              {!hw && <span className="muted">Detecting…</span>}
              {working.size === 0 && hw && <span className="muted">No working encoder — check FFmpeg in Settings → Diagnostics.</span>}
            </div>
          </div>
        </div>
      </div>
    </div>
  );
}
