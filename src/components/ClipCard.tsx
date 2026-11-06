import { Film, Play, Star } from "lucide-react";
import { useEffect, useState } from "react";
import { api, fileSrc } from "../lib/api";
import { formatBytes, formatDuration, formatRelative } from "../lib/format";
import type { Clip } from "../lib/types";

export function ClipCard({ clip, onOpen }: { clip: Clip; onOpen: (c: Clip) => void }) {
  const [thumb, setThumb] = useState<string | null>(clip.thumbnail);
  useEffect(() => {
    setThumb(clip.thumbnail);
    if (!clip.thumbnail) {
      let alive = true;
      api.thumbnail(clip.id).then((t) => alive && setThumb(t)).catch(() => {});
      return () => {
        alive = false;
      };
    }
  }, [clip.id, clip.thumbnail]);
  return (
    <div className="clip" onClick={() => onOpen(clip)} role="button" tabIndex={0} onKeyDown={(e) => e.key === "Enter" && onOpen(clip)}>
      <div className="thumb">
        {thumb ? <img src={fileSrc(thumb)} alt="" loading="lazy" draggable={false} /> : (
          <div className="placeholder">
            <Film size={22} />
          </div>
        )}
        <div className="shade" />
        <div className="play">
          <span>
            <Play size={17} fill="currentColor" style={{ marginLeft: 2 }} />
          </span>
        </div>
        {clip.favorite && <Star className="fav" size={14} fill="currentColor" />}
        <span className="dur num">{formatDuration(clip.duration)}</span>
      </div>
      <div className="clip-meta">
        <b title={clip.title}>{clip.title}</b>
        <span>
          {[clip.game, formatRelative(clip.created_at), formatBytes(clip.size)].filter(Boolean).join(" · ")}
        </span>
      </div>
    </div>
  );
}
