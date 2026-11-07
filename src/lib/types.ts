// TypeScript mirrors of the Rust (serde) types exposed by the engine.

export type ReplayState = "stopped" | "starting" | "running" | "restarting" | "error";
export type Codec = "h264" | "hevc" | "av1";
export type CodecChoice = "auto" | Codec;
export type QualityProfile = "quality" | "balanced" | "compact";
export type BackendKind = "auto" | "wgc" | "dda" | "x11" | "portal" | "test";
export type Resolution = "native" | "p720" | "p1080" | "p1440" | "p2160";
export type BufferStorage = "auto" | "memory" | "disk";
export type Corner = "top_left" | "top_right" | "bottom_left" | "bottom_right";
export type Priority = "below_normal" | "normal" | "above_normal";
export type Channel = "stable" | "beta";
export type EncoderFamily = "nvenc" | "amf" | "qsv" | "vaapi" | "software";

export interface SessionInfo {
  backend: BackendKind;
  encoder: string;
  codec: Codec;
  width: number;
  height: number;
  fps: number;
  monitor: string;
  restore_token: string | null;
}

export interface DetectedGame {
  name: string;
  exe: string;
  pid: number;
  foreground: boolean;
  source: "custom" | "database" | "store_library";
}

export interface EngineStatus {
  state: ReplayState;
  message: string | null;
  buffered_secs: number;
  buffer_target_secs: number;
  buffer_bytes: number;
  buffer_storage: "memory" | "disk";
  session: SessionInfo | null;
  since_ms: number | null;
  restarts: number;
  game: DetectedGame | null;
  saving: boolean;
  fallback_note: string | null;
  last_clip_ms: number | null;
  /** Shell command that fixes `message` (e.g. installs missing GStreamer plugins). */
  fix_command?: string | null;
}

export type CaptureTarget = { kind: "monitor"; index: number } | { kind: "window"; exe: string } | { kind: "follow_game" };

export interface AudioConfig {
  system: boolean;
  microphone: boolean;
  system_device: string | null;
  mic_device: string | null;
  system_volume: number;
  mic_volume: number;
}

export interface Settings {
  version: number;
  general: {
    launch_at_startup: boolean;
    start_minimized: boolean;
    start_replay_on_launch: boolean;
    auto_replay_on_game: boolean;
    close_to_tray: boolean;
    first_run_done: boolean;
  };
  capture: {
    backend: BackendKind;
    target: CaptureTarget;
    resolution: Resolution;
    fps: number;
    cursor: boolean;
    audio: AudioConfig;
  };
  encoding: {
    codec: CodecChoice;
    encoder: string | null;
    profile: QualityProfile;
    keyframe_interval_secs: number;
    allow_software: boolean;
  };
  replay: {
    buffer_secs: number;
    storage: BufferStorage;
    default_clip_secs: number;
    max_ram_mb: number;
  };
  library: { clips_dir: string | null; organize_by_game: boolean };
  hotkeys: {
    save_default: string | null;
    save_30: string | null;
    save_60: string | null;
    save_120: string | null;
    save_180: string | null;
    save_300: string | null;
    toggle_replay: string | null;
  };
  overlay: { enabled: boolean; position: Corner; show_indicator: boolean; show_toasts: boolean; opacity: number };
  performance: { resource_saver: boolean; stats_interval_ms: number; free_ui_when_hidden: boolean; capture_priority: Priority };
  updates: { auto_check: boolean; channel: Channel; last_check: number | null; skipped_version: string | null };
  advanced: { ffmpeg_path: string | null; audio_offset_ms: number; fragment_ms: number; log_level: string };
  custom_games: { name: string; exes: string[]; cmd_contains?: string[] }[];
  portal_restore_token: string | null;
}

export interface Clip {
  id: number;
  path: string;
  title: string;
  game: string | null;
  created_at: number;
  duration: number;
  size: number;
  width: number;
  height: number;
  fps: number;
  video_codec: string | null;
  audio_codec: string | null;
  encoder: string | null;
  favorite: boolean;
  thumbnail: string | null;
}

export type SortBy = "newest" | "oldest" | "largest" | "smallest" | "longest" | "shortest" | "name";

export interface ClipQuery {
  search?: string | null;
  game?: string | null;
  favorites_only?: boolean;
  sort?: SortBy;
  limit?: number | null;
  offset?: number | null;
}

export interface LibraryStats {
  count: number;
  total_bytes: number;
  total_duration: number;
  games: { game: string; count: number }[];
}

export interface GpuUsage {
  name: string;
  util_percent: number | null;
  encoder_percent: number | null;
  vram_used_bytes: number | null;
  vram_total_bytes: number | null;
}

export interface ResourceStats {
  app_cpu_percent: number;
  app_ram_bytes: number;
  sys_cpu_percent: number;
  sys_ram_used_bytes: number;
  sys_ram_total_bytes: number;
  gpus: GpuUsage[];
}

export interface ProbeResult {
  id: string;
  codec: Codec;
  family: EncoderFamily;
  ok: boolean;
  error: string | null;
  millis: number;
}

export interface GpuInfo {
  index: number;
  vendor: "nvidia" | "amd" | "intel" | "other";
  name: string;
  vram_bytes: number | null;
  driver: string | null;
  render_node: string | null;
}

export interface HwState {
  report: {
    system: {
      os_family: string;
      os_name: string;
      kernel: string;
      windows_build: number | null;
      arch: string;
      cpu_name: string;
      cpu_threads: number;
      ram_total_bytes: number;
      display_server: "windows" | "x11" | "wayland" | "unknown";
      desktop: string | null;
    };
    gpus: GpuInfo[];
    ffmpeg: {
      path: string;
      version: string;
      encoders: string[];
      filters: string[];
      input_devices: string[];
      source: string;
    } | null;
    ffmpeg_error: string | null;
    gstreamer: { launch: string; elements: string[] } | null;
  };
  probes: ProbeResult[];
  fingerprint: string;
}

export interface MonitorInfo {
  index: number;
  name: string;
  x: number;
  y: number;
  width: number;
  height: number;
  primary: boolean;
}

export interface AudioDevice {
  id: string;
  name: string;
  input: boolean;
}

export interface Estimate {
  width: number;
  height: number;
  fps: number;
  codec: Codec;
  mb_per_minute: number;
  buffer_mb: number;
  buffer_storage: "memory" | "disk";
  clip_mb: [number, number][];
}

export type CompressTarget = { kind: "profile"; value: QualityProfile } | { kind: "size_mb"; value: number };

export interface CompressOptions {
  codec: Codec;
  target: CompressTarget;
  max_height: number | null;
  max_fps: number | null;
}

export interface HotkeyStatus {
  backend: string;
  registrations: { id: string; accelerator: string; ok: boolean; error: string | null }[];
  note: string | null;
}

export interface AvailableUpdate {
  version: string;
  current: string;
  channel: Channel;
  notes: string;
  pub_date: string;
  asset_key: string;
  asset: { url: string; sha256: string; size: number } | null;
}

export interface AppInfo {
  version: string;
  repo: string;
  clips_dir: string;
  logs_dir: string;
  config_dir: string;
  install_kind: string;
  platform: string;
}

export type EngineEvent =
  | ({ type: "status" } & EngineStatus)
  | { type: "clip_saved"; clip: Clip; remuxed: boolean; elapsed_ms: number }
  | { type: "clip_failed"; message: string }
  | { type: "game_changed"; game: DetectedGame | null }
  | { type: "job_progress"; job_id: number; clip_id: number; kind: string; progress: number }
  | { type: "job_done"; job_id: number; clip_id: number; kind: string; clip: Clip | null; error: string | null }
  | { type: "library_changed" }
  | { type: "hotkey_pressed"; action: { type: "save_clip"; secs: number | null } | { type: "toggle_replay" } };

export type OverlayMessage = { kind: "saved"; title: string; seconds: number } | { kind: "error"; title: string } | { kind: "info"; title: string } | { kind: "indicator"; active: boolean };

export interface DriverCheck {
  gpu: string;
  vendor: "nvidia" | "amd" | "intel" | "other";
  driver: string;
  encoder_api: string;
  encoder_ok: boolean;
  problem: string | null;
  fix: string | null;
  link: string | null;
  optional: boolean;
}

export interface PlayerCheck {
  players: string[];
  default_handler: string | null;
  plays_everything: boolean;
  hevc_extension: boolean | null;
  av1_extension: boolean | null;
}

export interface Issue {
  severity: "warning" | "info";
  title: string;
  detail: string;
  fix: string | null;
  link: string | null;
}

export interface SystemCheck {
  drivers: DriverCheck[];
  players: PlayerCheck;
  codec: Codec;
  gpu_encoder: boolean;
  issues: Issue[];
}
