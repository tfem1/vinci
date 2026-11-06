import { describe, expect, it } from "vitest";
import { acceleratorFromEvent, codecLabel, encoderLabel, formatBytes, formatDuration, formatRelative, formatSpan, resolutionLabel } from "./format";

const ev = (code: string, mods: Partial<{ ctrlKey: boolean; altKey: boolean; shiftKey: boolean; metaKey: boolean }> = {}) => ({
  key: "",
  code,
  ctrlKey: false,
  altKey: false,
  shiftKey: false,
  metaKey: false,
  ...mods,
});

describe("format", () => {
  it("durations", () => {
    expect(formatDuration(0)).toBe("0:00");
    expect(formatDuration(30.4)).toBe("0:30");
    expect(formatDuration(125)).toBe("2:05");
    expect(formatDuration(3725)).toBe("1:02:05");
    expect(formatSpan(30)).toBe("30s");
    expect(formatSpan(60)).toBe("1m");
    expect(formatSpan(90)).toBe("1m 30s");
    expect(formatSpan(300)).toBe("5m");
  });

  it("bytes", () => {
    expect(formatBytes(0)).toBe("0 B");
    expect(formatBytes(999)).toBe("999 B");
    expect(formatBytes(9_500_000)).toBe("9.5 MB");
    expect(formatBytes(137_000_000)).toBe("137 MB");
    expect(formatBytes(2_400_000_000)).toBe("2.4 GB");
  });

  it("labels", () => {
    expect(resolutionLabel(1920, 1080)).toBe("1080p");
    expect(resolutionLabel(3840, 2160)).toBe("4K");
    expect(resolutionLabel(1720, 720)).toBe("720p");
    expect(resolutionLabel(1366, 768)).toBe("1366×768");
    expect(codecLabel("hevc")).toBe("HEVC");
    expect(encoderLabel("hevc_nvenc")).toBe("NVIDIA NVENC · HEVC");
    expect(encoderLabel("libx264")).toBe("CPU · H.264");
    expect(encoderLabel("av1_qsv")).toBe("Intel Quick Sync · AV1");
  });

  it("relative time", () => {
    const now = 1_800_000_000_000;
    expect(formatRelative(now - 10_000, now)).toBe("just now");
    expect(formatRelative(now - 5 * 60_000, now)).toBe("5 min ago");
    expect(formatRelative(now - 3 * 3600_000, now)).toBe("3 h ago");
  });

  it("accelerators from keyboard events", () => {
    expect(acceleratorFromEvent(ev("F10", { altKey: true }))).toBe("Alt+F10");
    expect(acceleratorFromEvent(ev("KeyS", { ctrlKey: true, shiftKey: true }))).toBe("Ctrl+Shift+S");
    expect(acceleratorFromEvent(ev("Digit1", { altKey: true }))).toBe("Alt+1");
    expect(acceleratorFromEvent(ev("ShiftLeft", { shiftKey: true }))).toBeNull();
    expect(acceleratorFromEvent(ev("PrintScreen"))).toBe("PrintScreen");
  });
});
