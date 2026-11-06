import { ExternalLink, FolderInput, FolderOpen, Minimize2, Scissors, Star, Trash2, X } from "lucide-react";
import { useCallback, useEffect, useRef, useState } from "react";
import { useEngine } from "../hooks/engine";
import { api, errorText, fileSrc, isTauri } from "../lib/api";
import { codecLabel, encoderLabel, formatBytes, formatDuration, resolutionLabel } from "../lib/format";
import type { Clip, CompressOptions, EngineEvent } from "../lib/types";
import { Bar, Modal, Toggle, useClickOutside } from "./ui";

const COMPRESS_PRESETS: { label: string; hint: string; options: CompressOptions }[] = [
  { label: "Discord (10 MB)", hint: "auto-resizes", options: { codec: "h264", target: { kind: "size_mb", value: 10 }, max_height: null, max_fps: 60 } },
  { label: "Discord Nitro Basic (50 MB)", hint: "H.264", options: { codec: "h264", target: { kind: "size_mb", value: 50 }, max_height: null, max_fps: null } },
  { label: "25 MB", hint: "HEVC", options: { codec: "hevc", target: { kind: "size_mb", value: 25 }, max_height: null, max_fps: null } },
  { label: "Compact quality", hint: "AV1 / HEVC", options: { codec: "av1", target: { kind: "profile", value: "compact" }, max_height: null, max_fps: null } },
  { label: "Compact 1080p", hint: "AV1 / HEVC", options: { codec: "av1", target: { kind: "profile", value: "compact" }, max_height: 1080, max_fps: null } },
];

export function Player({ clip: initial, onClose }: { clip: Clip; onClose: () => void }) {
  const { toast, subscribe } = useEngine();
  const [clip, setClip] = useState(initial);
  const [title, setTitle] = useState(initial.title);
  const [menu, setMenu] = useState(false);
  const [confirmDelete, setConfirmDelete] = useState(false);
  const [job, setJob] = useState<{ id: number; kind: string; progress: number } | null>(null);
  const [trim, setTrim] = useState<{ start: number; end: number; replace: boolean } | null>(null);
  const [videoError, setVideoError] = useState(false);
  const [videoKey, setVideoKey] = useState(0);
  const video = useRef<HTMLVideoElement>(null);
  const menuRef = useClickOutside(menu, () => setMenu(false));

  useEffect(
    () =>
      subscribe((e: EngineEvent) => {
        if (e.type === "job_progress" && e.clip_id === clip.id) setJob({ id: e.job_id, kind: e.kind, progress: e.progress });
        if (e.type === "job_done" && e.clip_id === clip.id) {
          setJob(null);
          if (e.clip && e.clip.id === clip.id) {
            setClip(e.clip);
            setVideoKey((k) => k + 1);
            setVideoError(false);
          }
        }
      }),
    [subscribe, clip.id],
  );

  const commitTitle = useCallback(async () => {
    const t = title.trim();
    if (!t || t === clip.title) return setTitle(clip.title);
    try {
      const c = await api.renameClip(clip.id, t);
      setClip(c);
      setTitle(c.title);
    } catch (e) {
      toast({ kind: "error", title: "Rename failed", detail: errorText(e) });
      setTitle(clip.title);
    }
  }, [title, clip, toast]);

  const run = async (fn: () => Promise<unknown>, err: string) => {
    try {
      await fn();
    } catch (e) {
      toast({ kind: "error", title: err, detail: errorText(e) });
    }
  };

  const compress = (o: CompressOptions) =>
    run(async () => {
      setMenu(false);
      const id = await api.startCompress(clip.id, o, true);
      setJob({ id, kind: "compress", progress: 0 });
    }, "Compression failed");

  const move = () =>
    run(async () => {
      const dest = await api.pickFolder("Move clip to…");
      if (!dest) return;
      setClip(await api.moveClip(clip.id, dest));
      setVideoKey((k) => k + 1);
      toast({ kind: "ok", title: "Clip moved", detail: dest });
    }, "Move failed");

  const del = () =>
    run(async () => {
      await api.deleteClip(clip.id);
      toast({ kind: "info", title: "Clip deleted", detail: clip.title });
      onClose();
    }, "Delete failed");

  const now = () => video.current?.currentTime ?? 0;

  return (
    <Modal onClose={onClose}>
      <div className="modal-head">
        <input className="title-input" value={title} onChange={(e) => setTitle(e.target.value)} onBlur={commitTitle} onKeyDown={(e) => e.key === "Enter" && (e.target as HTMLInputElement).blur()} aria-label="Clip title" />
        <button className="btn icon ghost" aria-label="Favorite" title="Favorite" onClick={() => run(async () => setClip(await api.setFavorite(clip.id, !clip.favorite)), "Could not update")}>
          <Star size={16} fill={clip.favorite ? "currentColor" : "none"} />
        </button>
        <button className="btn icon ghost" aria-label="Close" onClick={onClose}>
          <X size={16} />
        </button>
      </div>
      <div className="modal-body">
        <div className="video-wrap">
          {!videoError ? (
            <video key={videoKey} ref={video} src={fileSrc(clip.path)} controls autoPlay preload="metadata" onError={() => setVideoError(true)} />
          ) : (
            <div className="fallback">
              <b style={{ color: "var(--text)" }}>This clip can't be previewed here</b>
              <span>The system web view can't decode {codecLabel(clip.video_codec)} here. The file itself is fine — open it in your video player.</span>
              <div>
                <button className="btn sm" onClick={() => run(() => api.openClip(clip.id), "Could not open")}>
                  <ExternalLink size={13} /> Open in player
                </button>
              </div>
            </div>
          )}
        </div>
        <div className="meta-row">
          {clip.game && <span className="pill solid">{clip.game}</span>}
          <span className="pill num">{formatDuration(clip.duration)}</span>
          <span className="pill">{resolutionLabel(clip.width, clip.height)} · {Math.round(clip.fps)} fps</span>
          <span className="pill">{codecLabel(clip.video_codec)}{clip.audio_codec ? ` + ${clip.audio_codec.toUpperCase()}` : ""}</span>
          {clip.encoder && <span className="pill">{encoderLabel(clip.encoder)}</span>}
          <span className="pill num">{formatBytes(clip.size)}</span>
          <span className="pill faint">{new Date(clip.created_at).toLocaleString()}</span>
        </div>

        {job && (
          <div className="note">
            <Minimize2 size={14} />
            <div style={{ flex: 1, display: "grid", gap: 8 }}>
              <span>{job.kind === "trim" ? "Trimming" : "Compressing"}… {Math.round(job.progress * 100)}%</span>
              <Bar value={job.progress * 100} />
            </div>
            <button className="btn sm ghost" onClick={() => api.cancelJob(job.id)}>
              Cancel
            </button>
          </div>
        )}

        {trim && (
          <div className="trim">
            <div className="row">
              <Scissors size={14} />
              <b style={{ fontWeight: 560 }}>Trim</b>
              <span className="muted num" style={{ marginLeft: "auto" }}>
                {formatDuration(trim.start)} → {formatDuration(trim.end)} ({formatDuration(Math.max(0, trim.end - trim.start))})
              </span>
            </div>
            <div className="row">
              <button className="btn sm" onClick={() => setTrim({ ...trim, start: Math.min(now(), trim.end - 1) })}>
                Set start at playhead
              </button>
              <button className="btn sm" onClick={() => setTrim({ ...trim, end: Math.max(now(), trim.start + 1) })}>
                Set end at playhead
              </button>
              <span style={{ marginLeft: "auto", display: "flex", alignItems: "center", gap: 8 }} className="muted">
                Replace original <Toggle on={trim.replace} onChange={(v) => setTrim({ ...trim, replace: v })} />
              </span>
              <button className="btn sm ghost" onClick={() => setTrim(null)}>
                Cancel
              </button>
              <button
                className="btn sm primary"
                disabled={trim.end - trim.start < 1}
                onClick={() =>
                  run(async () => {
                    const id = await api.startTrim(clip.id, trim.start, trim.end, trim.replace);
                    setJob({ id, kind: "trim", progress: 0 });
                    setTrim(null);
                  }, "Trim failed")
                }
              >
                Save trim
              </button>
            </div>
            <span className="faint" style={{ fontSize: 11.5 }}>
              Lossless (no re-encode). The start snaps to the previous keyframe.
            </span>
          </div>
        )}

        <div className="actions">
          <div className="rel" ref={menuRef}>
            <button className="btn" onClick={() => setMenu((m) => !m)} disabled={!!job}>
              <Minimize2 size={14} /> Compress
            </button>
            {menu && (
              <div className="menu" style={{ bottom: 40, left: 0 }}>
                {COMPRESS_PRESETS.map((p) => (
                  <button key={p.label} onClick={() => compress(p.options)}>
                    {p.label} <span>{p.hint}</span>
                  </button>
                ))}
                <hr />
                <div className="faint" style={{ fontSize: 11, padding: "2px 10px 4px" }}>
                  Replaces the file in place — no duplicate.
                </div>
              </div>
            )}
          </div>
          <button className="btn" disabled={!!job} onClick={() => setTrim({ start: 0, end: clip.duration, replace: false })}>
            <Scissors size={14} /> Trim
          </button>
          <button className="btn" onClick={() => run(() => api.revealClip(clip.id), "Could not open folder")}>
            <FolderOpen size={14} /> Show in folder
          </button>
          {isTauri && (
            <button className="btn" onClick={move}>
              <FolderInput size={14} /> Move…
            </button>
          )}
          <div className="grow" />
          {confirmDelete ? (
            <>
              <span className="muted" style={{ fontSize: 12 }}>
                Delete the file permanently?
              </span>
              <button className="btn ghost" onClick={() => setConfirmDelete(false)}>
                Keep
              </button>
              <button className="btn danger" onClick={del}>
                <Trash2 size={14} /> Delete
              </button>
            </>
          ) : (
            <button className="btn ghost danger" onClick={() => setConfirmDelete(true)}>
              <Trash2 size={14} /> Delete
            </button>
          )}
        </div>
        <div className="faint mono" style={{ fontSize: 11, wordBreak: "break-all", userSelect: "text" }}>
          {clip.path}
        </div>
      </div>
    </Modal>
  );
}
