import { AlertTriangle, Check, Info } from "lucide-react";
import { useEffect, useState } from "react";
import { formatSpan } from "./lib/format";
import type { Corner, OverlayMessage } from "./lib/types";

/** The transparent, click-through, always-on-top overlay window. */
export function Overlay() {
  const [msg, setMsg] = useState<(OverlayMessage & { key: number; out?: boolean }) | null>(null);
  const [indicator, setIndicator] = useState(false);
  const [pos, setPos] = useState<Corner>("top_right");
  const [opacity, setOpacity] = useState(0.94);

  useEffect(() => {
    document.body.classList.add("overlay-window");
    let un: (() => void) | undefined;
    let timers: number[] = [];
    import("@tauri-apps/api/event").then(({ listen, emit }) =>
      listen<OverlayMessage & { position?: Corner; opacity?: number }>("vinci://overlay", (e) => {
        const m = e.payload;
        if (m.position) setPos(m.position);
        if (m.opacity) setOpacity(m.opacity);
        if (m.kind === "indicator") return setIndicator(m.active);
        timers.forEach(clearTimeout);
        const key = Date.now();
        setMsg({ ...m, key });
        timers = [window.setTimeout(() => setMsg((x) => (x && x.key === key ? { ...x, out: true } : x)), 2600), window.setTimeout(() => setMsg((x) => (x && x.key === key ? null : x)), 2950)];
      }).then((u) => {
        un = u;
        // Tell the backend we can receive messages (it queues them until then).
        void emit("vinci://overlay-ready");
      }),
    );
    return () => un?.();
  }, []);

  return (
    <div className={`overlay-root ${pos}`} style={{ opacity }}>
      {msg ? (
        <div className={`ov-toast ${msg.out ? "out" : ""}`} key={msg.key}>
          <div className="ic">{msg.kind === "saved" ? <Check size={17} strokeWidth={2.6} /> : msg.kind === "error" ? <AlertTriangle size={16} /> : <Info size={16} />}</div>
          <div>
            <b>{msg.kind === "saved" ? `Clip saved · ${formatSpan(msg.seconds)}` : "title" in msg ? msg.title : ""}</b>
            <span>{msg.kind === "saved" ? msg.title : "VINCI"}</span>
          </div>
        </div>
      ) : indicator ? (
        <div className="ov-dot">
          <span className="dot live" /> REPLAY
        </div>
      ) : null}
    </div>
  );
}
