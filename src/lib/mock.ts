// In-browser mock of the engine, used only when the UI runs outside Tauri
// (`npm run dev` in a normal browser). Never shipped behaviour — the desktop app
// always talks to the real Rust engine.

import type { Clip, EngineEvent, EngineStatus, HwState, Settings } from "./types";

const listeners = new Set<(e: EngineEvent) => void>();
const emit = (e: EngineEvent) => listeners.forEach((l) => l(e));

const settings: Settings = {
  version: 1,
  general: { launch_at_startup: false, start_minimized: false, start_replay_on_launch: true, auto_replay_on_game: true, close_to_tray: true, first_run_done: true },
  capture: { backend: "auto", target: { kind: "monitor", index: 0 }, resolution: "native", fps: 60, cursor: true, audio: { system: true, microphone: false, system_device: null, mic_device: null, system_volume: 1, mic_volume: 1 } },
  encoding: { codec: "auto", encoder: null, profile: "balanced", keyframe_interval_secs: 2, allow_software: true },
  replay: { buffer_secs: 120, storage: "auto", default_clip_secs: 30, max_ram_mb: 512 },
  library: { clips_dir: null, organize_by_game: true },
  hotkeys: { save_default: "Alt+F10", save_30: null, save_60: "Alt+2", save_120: null, save_180: null, save_300: "Alt+5", toggle_replay: "Alt+F9" },
  overlay: { enabled: true, position: "top_right", show_indicator: false, show_toasts: true, opacity: 0.94 },
  performance: { resource_saver: false, stats_interval_ms: 2000, free_ui_when_hidden: true, capture_priority: "above_normal" },
  updates: { auto_check: true, channel: "stable", last_check: null, skipped_version: null },
  advanced: { ffmpeg_path: null, audio_offset_ms: 0, fragment_ms: 500, log_level: "info" },
  custom_games: [],
  portal_restore_token: null,
};

const started = Date.now() - 41_000;
let running = true;
const status = (): EngineStatus => ({
  state: running ? "running" : "stopped",
  message: null,
  buffered_secs: running ? Math.min(settings.replay.buffer_secs, (Date.now() - started) / 1000) : 0,
  buffer_target_secs: settings.replay.buffer_secs,
  buffer_bytes: running ? Math.min(settings.replay.buffer_secs, (Date.now() - started) / 1000) * 1_150_000 : 0,
  buffer_storage: "memory",
  session: running ? { backend: "wgc", encoder: "hevc_nvenc", codec: "hevc", width: 2560, height: 1440, fps: 60, monitor: "Display 1", restore_token: null } : null,
  since_ms: running ? started : null,
  restarts: 0,
  game: { name: "Valorant", exe: "VALORANT-Win64-Shipping.exe", pid: 4242, foreground: true, source: "database" },
  saving: false,
  fallback_note: null,
  last_clip_ms: Date.now() - 260_000,
});

const games = ["Valorant", "Counter-Strike 2", "Apex Legends", "Rocket League", "Elden Ring"];
const titles = ["1v4 clutch on Ascent", "Ace — deagle only", "Last circle squad wipe", "Ceiling shot double tap", "No-hit Malenia", "Insane flick", "Overtime winner", "Triple kill retake", "Pistol round ace", "Clean defuse"];
let clips: Clip[] = titles.map((t, i) => ({
  id: i + 1,
  path: `/home/you/Videos/VINCI/${games[i % games.length]}/clip-${i + 1}.mp4`,
  title: t,
  game: games[i % games.length],
  created_at: Date.now() - i * 5_400_000 - 260_000,
  duration: [30.4, 60.2, 120.5, 30.1, 180.3, 30.0, 60.4, 30.2, 300.6, 30.3][i],
  size: [21e6, 43e6, 88e6, 20e6, 131e6, 19e6, 44e6, 22e6, 214e6, 21e6][i],
  width: 2560,
  height: 1440,
  fps: 60,
  video_codec: i % 3 === 0 ? "av1" : "hevc",
  audio_codec: "aac",
  encoder: i % 3 === 0 ? "av1_nvenc" : "hevc_nvenc",
  favorite: i % 4 === 0,
  thumbnail: `mock://${i}`,
}));

const hw: HwState = {
  report: {
    system: { os_family: "windows", os_name: "Windows 11 Pro (build 26100)", kernel: "26100", windows_build: 26100, arch: "x86_64", cpu_name: "AMD Ryzen 7 7800X3D 8-Core Processor", cpu_threads: 16, ram_total_bytes: 32e9, display_server: "windows", desktop: null },
    gpus: [{ index: 0, vendor: "nvidia", name: "NVIDIA GeForce RTX 4070", vram_bytes: 12e9, driver: null, render_node: null }],
    ffmpeg: { path: "C:\\Users\\you\\AppData\\Local\\VINCI\\ffmpeg.exe", version: "n8.1", encoders: [], filters: ["gfxcapture", "ddagrab", "scale_d3d11"], input_devices: ["lavfi", "dshow"], source: "bundled" },
    ffmpeg_error: null,
    gstreamer: null,
  },
  probes: [
    { id: "h264_nvenc", codec: "h264", family: "nvenc", ok: true, error: null, millis: 180 },
    { id: "hevc_nvenc", codec: "hevc", family: "nvenc", ok: true, error: null, millis: 190 },
    { id: "av1_nvenc", codec: "av1", family: "nvenc", ok: true, error: null, millis: 210 },
    { id: "h264_qsv", codec: "h264", family: "qsv", ok: false, error: "No Intel GPU", millis: 40 },
    { id: "libx264", codec: "h264", family: "software", ok: true, error: null, millis: 320 },
    { id: "libx265", codec: "hevc", family: "software", ok: true, error: null, millis: 900 },
    { id: "libsvtav1", codec: "av1", family: "software", ok: true, error: null, millis: 700 },
  ],
  fingerprint: "mock",
};

function thumbSvg(i: number): string {
  const shades = ["#2a2a2a", "#1f1f1f", "#262626", "#1c1c1c", "#303030"];
  const a = shades[i % shades.length];
  const b = shades[(i + 2) % shades.length];
  const svg = `<svg xmlns='http://www.w3.org/2000/svg' width='480' height='270'><defs><linearGradient id='g' x1='0' y1='0' x2='1' y2='1'><stop offset='0' stop-color='${a}'/><stop offset='1' stop-color='${b}'/></linearGradient></defs><rect width='480' height='270' fill='url(#g)'/><circle cx='${120 + (i * 53) % 240}' cy='${80 + (i * 37) % 120}' r='${40 + (i % 3) * 18}' fill='#ffffff' fill-opacity='0.05'/><path d='M0 ${200 - (i % 4) * 14} L480 ${150 + (i % 5) * 12} L480 270 L0 270Z' fill='#000' fill-opacity='0.35'/></svg>`;
  return `data:image/svg+xml;utf8,${encodeURIComponent(svg)}`;
}

export function mockFileSrc(p: string): string {
  const m = /^mock:\/\/(\d+)/.exec(p);
  return m ? thumbSvg(Number(m[1])) : p;
}

export function mockListen(cb: (e: EngineEvent) => void): () => void {
  listeners.add(cb);
  return () => listeners.delete(cb);
}

setInterval(() => emit({ type: "status", ...status() }), 1000);

export async function mockInvoke<T>(cmd: string, args: Record<string, unknown> = {}): Promise<T> {
  await new Promise((r) => setTimeout(r, 30));
  const r = (v: unknown) => v as T;
  switch (cmd) {
    case "get_status":
      return r(status());
    case "get_settings":
      return r(structuredClone(settings));
    case "update_settings":
      Object.assign(settings, args.settings);
      return r(structuredClone(settings));
    case "start_replay":
      running = true;
      return r(undefined);
    case "stop_replay":
      running = false;
      return r(undefined);
    case "toggle_replay":
      running = !running;
      return r(running);
    case "save_clip": {
      const secs = (args.secs as number | null) ?? settings.replay.default_clip_secs;
      const clip: Clip = { ...clips[0], id: Date.now(), title: `Valorant — ${new Date().toLocaleTimeString()}`, created_at: Date.now(), duration: secs + 0.4, size: secs * 700_000, favorite: false };
      setTimeout(() => {
        clips = [clip, ...clips];
        emit({ type: "clip_saved", clip, remuxed: true, elapsed_ms: 640 });
        emit({ type: "library_changed" });
      }, 700);
      return r(undefined);
    }
    case "get_hardware":
      return r(hw);
    case "get_monitors":
      return r([{ index: 0, name: "Display 1 (DISPLAY1)", x: 0, y: 0, width: 2560, height: 1440, primary: true }, { index: 1, name: "Display 2 (DISPLAY2)", x: 2560, y: 0, width: 1920, height: 1080, primary: false }]);
    case "get_audio_devices":
      return r([{ id: "out1", name: "Speakers (Realtek)", input: false }, { id: "in1", name: "Microphone (Shure MV7)", input: true }]);
    case "get_estimate": {
      const s = args.settings as Settings;
      const per = { quality: 120, balanced: 69, compact: 42 }[s.encoding.profile] * (s.capture.fps / 60) ** 0.6;
      return r({ width: 2560, height: 1440, fps: s.capture.fps, codec: "hevc", mb_per_minute: per, buffer_mb: (per * s.replay.buffer_secs) / 60, buffer_storage: (per * s.replay.buffer_secs) / 60 > s.replay.max_ram_mb ? "disk" : "memory", clip_mb: [30, 60, 120, 180, 300].map((x) => [x, (per * x) / 60]) });
    }
    case "get_stats":
      return r({ app_cpu_percent: 0.6 + Math.random() * 0.4, app_ram_bytes: 58e6 + Math.random() * 3e6, sys_cpu_percent: 31 + Math.random() * 8, sys_ram_used_bytes: 14.2e9, sys_ram_total_bytes: 32e9, gpus: [{ name: "NVIDIA GeForce RTX 4070", util_percent: 71 + Math.random() * 6, encoder_percent: 9 + Math.random() * 3, vram_used_bytes: 7.1e9, vram_total_bytes: 12e9 }] });
    case "list_clips": {
      const q = (args.query ?? {}) as { search?: string; game?: string; favorites_only?: boolean; sort?: string; limit?: number };
      let out = clips.filter((c) => (!q.search || (c.title + (c.game ?? "")).toLowerCase().includes(q.search.toLowerCase())) && (!q.game || c.game === q.game) && (!q.favorites_only || c.favorite));
      const sorters: Record<string, (a: Clip, b: Clip) => number> = { newest: (a, b) => b.created_at - a.created_at, oldest: (a, b) => a.created_at - b.created_at, largest: (a, b) => b.size - a.size, smallest: (a, b) => a.size - b.size, longest: (a, b) => b.duration - a.duration, shortest: (a, b) => a.duration - b.duration, name: (a, b) => a.title.localeCompare(b.title) };
      out = [...out].sort(sorters[q.sort ?? "newest"]);
      return r(q.limit ? out.slice(0, q.limit) : out);
    }
    case "library_stats": {
      const g = new Map<string, number>();
      clips.forEach((c) => c.game && g.set(c.game, (g.get(c.game) ?? 0) + 1));
      return r({ count: clips.length, total_bytes: clips.reduce((a, c) => a + c.size, 0), total_duration: clips.reduce((a, c) => a + c.duration, 0), games: [...g].map(([game, count]) => ({ game, count })) });
    }
    case "rename_clip":
    case "set_favorite": {
      const c = clips.find((x) => x.id === args.id)!;
      if (cmd === "rename_clip") c.title = String(args.title);
      else c.favorite = Boolean(args.favorite);
      return r({ ...c });
    }
    case "delete_clip":
      clips = clips.filter((c) => c.id !== args.id);
      return r(undefined);
    case "clip_thumbnail":
      return r(clips.find((c) => c.id === args.id)?.thumbnail ?? null);
    case "get_app_info":
      return r({ version: "0.1.0", repo: "USERNAME/vinci", clips_dir: "C:\\Users\\you\\Videos\\VINCI", logs_dir: "", config_dir: "", install_kind: "windows_nsis", platform: "windows" });
    case "hotkey_status":
      return r({ backend: "global-hotkey", registrations: [{ id: "save_default", accelerator: "Alt+F10", ok: true, error: null }, { id: "toggle_replay", accelerator: "Alt+F9", ok: true, error: null }], note: null });
    case "validate_hotkey":
      return r(String(args.accelerator));
    case "check_update":
      return r(null);
    case "start_compress":
    case "start_trim": {
      const id = Date.now();
      let p = 0;
      const t = setInterval(() => {
        p += 0.1;
        emit({ type: "job_progress", job_id: id, clip_id: args.id as number, kind: cmd === "start_trim" ? "trim" : "compress", progress: Math.min(1, p) });
        if (p >= 1) {
          clearInterval(t);
          const c = clips.find((x) => x.id === args.id)!;
          c.size = Math.round(c.size * 0.4);
          emit({ type: "job_done", job_id: id, clip_id: c.id, kind: "compress", clip: { ...c }, error: null });
        }
      }, 200);
      return r(id);
    }
    default:
      return r(null);
  }
}
