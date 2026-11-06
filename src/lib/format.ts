// Formatting helpers (pure — unit tested in format.test.ts).

export function formatDuration(totalSecs: number): string {
  const s = Math.max(0, Math.round(totalSecs));
  const h = Math.floor(s / 3600);
  const m = Math.floor((s % 3600) / 60);
  const sec = s % 60;
  if (h > 0) return `${h}:${String(m).padStart(2, "0")}:${String(sec).padStart(2, "0")}`;
  return `${m}:${String(sec).padStart(2, "0")}`;
}

/** 30 → "30s", 60 → "1m", 90 → "1m 30s", 300 → "5m". */
export function formatSpan(secs: number): string {
  const s = Math.round(secs);
  if (s < 60) return `${s}s`;
  const m = Math.floor(s / 60);
  const r = s % 60;
  return r ? `${m}m ${r}s` : `${m}m`;
}

export function formatBytes(bytes: number, digits = 1): string {
  if (!Number.isFinite(bytes) || bytes <= 0) return "0 B";
  const units = ["B", "KB", "MB", "GB", "TB"];
  let i = 0;
  let v = bytes;
  while (v >= 1000 && i < units.length - 1) {
    v /= 1000;
    i++;
  }
  return `${v.toFixed(i === 0 ? 0 : v >= 100 ? 0 : digits)} ${units[i]}`;
}

export function formatPercent(v: number | null | undefined): string {
  if (v == null || !Number.isFinite(v)) return "—";
  return `${v < 10 ? v.toFixed(1) : Math.round(v)}%`;
}

export function formatRelative(ms: number, now = Date.now()): string {
  const d = Math.max(0, now - ms) / 1000;
  if (d < 45) return "just now";
  if (d < 3600) return `${Math.round(d / 60)} min ago`;
  if (d < 86400) return `${Math.round(d / 3600)} h ago`;
  if (d < 86400 * 7) return `${Math.round(d / 86400)} d ago`;
  return new Date(ms).toLocaleDateString(undefined, { day: "numeric", month: "short", year: "numeric" });
}

export function resolutionLabel(w: number, h: number): string {
  if (!w || !h) return "—";
  const named: Record<number, string> = { 720: "720p", 1080: "1080p", 1440: "1440p", 2160: "4K" };
  return named[h] ?? `${w}×${h}`;
}

export function codecLabel(c: string | null | undefined): string {
  switch ((c ?? "").toLowerCase()) {
    case "h264":
      return "H.264";
    case "hevc":
    case "h265":
      return "HEVC";
    case "av1":
      return "AV1";
    case "":
      return "—";
    default:
      return c!.toUpperCase();
  }
}

const ENCODER_LABELS: Record<string, string> = {
  nvenc: "NVIDIA NVENC",
  amf: "AMD AMF",
  qsv: "Intel Quick Sync",
  vaapi: "VA-API",
  lib: "CPU",
};

export function encoderLabel(id: string | null | undefined): string {
  if (!id) return "—";
  const fam = id.includes("nvenc") ? "nvenc" : id.includes("amf") ? "amf" : id.includes("qsv") ? "qsv" : id.includes("vaapi") ? "vaapi" : "lib";
  const codec = id.startsWith("h264") || id === "libx264" ? "H.264" : id.startsWith("hevc") || id === "libx265" ? "HEVC" : "AV1";
  return `${ENCODER_LABELS[fam]} · ${codec}`;
}

/** Display a normalised accelerator nicely: "Alt+F10" → ["Alt", "F10"]. */
export function keyParts(acc: string | null | undefined): string[] {
  return acc ? acc.split("+").filter(Boolean) : [];
}

/** Turn a keyboard event into an accelerator string, or null if only modifiers are held. */
export function acceleratorFromEvent(e: { key: string; code: string; ctrlKey: boolean; altKey: boolean; shiftKey: boolean; metaKey: boolean }): string | null {
  const mods: string[] = [];
  if (e.ctrlKey) mods.push("Ctrl");
  if (e.altKey) mods.push("Alt");
  if (e.shiftKey) mods.push("Shift");
  if (e.metaKey) mods.push("Super");
  const code = e.code;
  let key: string | null = null;
  if (/^Key[A-Z]$/.test(code)) key = code.slice(3);
  else if (/^Digit[0-9]$/.test(code)) key = code.slice(5);
  else if (/^F([1-9]|1[0-9]|2[0-4])$/.test(code)) key = code;
  else {
    const named: Record<string, string> = {
      Space: "Space", Enter: "Enter", Tab: "Tab", Backspace: "Backspace", Delete: "Delete", Insert: "Insert", Home: "Home", End: "End",
      PageUp: "PageUp", PageDown: "PageDown", ArrowUp: "Up", ArrowDown: "Down", ArrowLeft: "Left", ArrowRight: "Right",
      PrintScreen: "PrintScreen", ScrollLock: "ScrollLock", Pause: "Pause", Backquote: "Backquote", Minus: "Minus", Equal: "Equal",
    };
    key = named[code] ?? null;
  }
  if (!key) return null;
  return [...mods, key].join("+");
}

export function clamp(v: number, lo: number, hi: number): number {
  return Math.min(hi, Math.max(lo, v));
}
