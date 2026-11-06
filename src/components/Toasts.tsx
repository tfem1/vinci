import { AlertTriangle, Check, Info } from "lucide-react";

export type Toast = { id: number; kind: "ok" | "error" | "info"; title: string; detail?: string };

export function Toasts({ items }: { items: Toast[] }) {
  return (
    <div className="toasts" aria-live="polite">
      {items.map((t) => (
        <div key={t.id} className={`toast ${t.kind === "error" ? "error" : ""}`}>
          <div className="ic">{t.kind === "ok" ? <Check size={15} strokeWidth={2.5} /> : t.kind === "error" ? <AlertTriangle size={14} /> : <Info size={14} />}</div>
          <div style={{ minWidth: 0 }}>
            <b>{t.title}</b>
            {t.detail && <span>{t.detail}</span>}
          </div>
        </div>
      ))}
    </div>
  );
}
