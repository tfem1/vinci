// Typed bridge to the Rust engine. In a plain browser (no Tauri) a mock backend is
// used so the UI can be developed and previewed with `npm run dev`.

import type {
  AppInfo, AudioDevice, AvailableUpdate, Clip, ClipQuery, CompressOptions, EngineEvent, EngineStatus, Estimate,
  HotkeyStatus, HwState, LibraryStats, MonitorInfo, ResourceStats, Settings, SystemCheck,
} from "./types";

export const isTauri = typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;

type Invoke = <T>(cmd: string, args?: Record<string, unknown>) => Promise<T>;

let invokeImpl: Invoke | null = null;
async function invoke<T>(cmd: string, args?: Record<string, unknown>): Promise<T> {
  if (!invokeImpl) {
    if (isTauri) {
      const core = await import("@tauri-apps/api/core");
      invokeImpl = core.invoke as Invoke;
    } else {
      const mock = await import("./mock");
      invokeImpl = mock.mockInvoke as Invoke;
    }
  }
  return invokeImpl<T>(cmd, args);
}

export const api = {
  status: () => invoke<EngineStatus>("get_status"),
  settings: () => invoke<Settings>("get_settings"),
  updateSettings: (settings: Settings) => invoke<Settings>("update_settings", { settings }),
  startReplay: () => invoke<void>("start_replay"),
  stopReplay: () => invoke<void>("stop_replay"),
  toggleReplay: () => invoke<boolean>("toggle_replay"),
  clearBuffer: () => invoke<void>("clear_buffer"),
  saveClip: (secs: number | null) => invoke<void>("save_clip", { secs }),
  hardware: (force = false) => invoke<HwState>("get_hardware", { force }),
  monitors: () => invoke<MonitorInfo[]>("get_monitors"),
  audioDevices: () => invoke<AudioDevice[]>("get_audio_devices"),
  estimate: (settings: Settings) => invoke<Estimate>("get_estimate", { settings }),
  stats: () => invoke<ResourceStats>("get_stats"),
  listClips: (query: ClipQuery) => invoke<Clip[]>("list_clips", { query }),
  libraryStats: () => invoke<LibraryStats>("library_stats"),
  renameClip: (id: number, title: string) => invoke<Clip>("rename_clip", { id, title }),
  setFavorite: (id: number, favorite: boolean) => invoke<Clip>("set_favorite", { id, favorite }),
  deleteClip: (id: number) => invoke<void>("delete_clip", { id }),
  moveClip: (id: number, dest: string) => invoke<Clip>("move_clip", { id, dest }),
  thumbnail: (id: number) => invoke<string | null>("clip_thumbnail", { id }),
  scanLibrary: () => invoke<{ added: number; removed: number; updated: number }>("scan_library"),
  revealClip: (id: number) => invoke<void>("reveal_clip", { id }),
  openClip: (id: number) => invoke<void>("open_clip", { id }),
  openClipsFolder: () => invoke<void>("open_clips_folder"),
  openLogsFolder: () => invoke<void>("open_logs_folder"),
  startCompress: (id: number, options: CompressOptions, replace: boolean) => invoke<number>("start_compress", { id, options, replace }),
  startTrim: (id: number, start: number, end: number, replace: boolean) => invoke<number>("start_trim", { id, start, end, replace }),
  cancelJob: (jobId: number) => invoke<void>("cancel_job", { jobId }),
  checkUpdate: () => invoke<AvailableUpdate | null>("check_update"),
  installUpdate: (update: AvailableUpdate) => invoke<string>("install_update", { update }),
  appInfo: () => invoke<AppInfo>("get_app_info"),
  hotkeyStatus: () => invoke<HotkeyStatus | null>("hotkey_status"),
  systemCheck: () => invoke<SystemCheck>("system_check"),
  openLink: (url: string) => invoke<void>("open_link", { url }),
  validateHotkey: (accelerator: string) => invoke<string>("validate_hotkey", { accelerator }),
  setAutostart: (enabled: boolean) => invoke<void>("set_autostart", { enabled }),
  previewOverlay: () => invoke<void>("preview_overlay"),
  pickFolder: (title: string) => invoke<string | null>("pick_folder", { title }),
  mediaUrl: (id: number) => invoke<string | null>("media_url", { id }),
};

/** Subscribe to engine events. Returns an unsubscribe function. */
export async function onEngineEvent(cb: (e: EngineEvent) => void): Promise<() => void> {
  if (isTauri) {
    const { listen } = await import("@tauri-apps/api/event");
    return listen<EngineEvent>("vinci://event", (e) => cb(e.payload));
  }
  const mock = await import("./mock");
  return mock.mockListen(cb);
}

/** URL usable in <img>/<video> for a local file. */
let convert: ((p: string) => string) | null = null;
export async function initFileSrc(): Promise<void> {
  if (isTauri) {
    const core = await import("@tauri-apps/api/core");
    convert = (p) => core.convertFileSrc(p);
  } else {
    const mock = await import("./mock");
    convert = mock.mockFileSrc;
  }
}
export function fileSrc(path: string | null | undefined): string | undefined {
  if (!path || !convert) return undefined;
  return convert(path);
}

export function errorText(e: unknown): string {
  if (typeof e === "string") return e;
  if (e && typeof e === "object" && "message" in e) return String((e as { message: unknown }).message);
  return JSON.stringify(e);
}
