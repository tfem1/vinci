import { useEffect, useRef, useState, type ReactNode } from "react";
import { acceleratorFromEvent, keyParts } from "../lib/format";

export function Toggle({ on, onChange, disabled, label }: { on: boolean; onChange: (v: boolean) => void; disabled?: boolean; label?: string }) {
  return <button type="button" role="switch" aria-checked={on} aria-label={label} disabled={disabled} className={`toggle ${on ? "on" : ""}`} onClick={() => onChange(!on)} />;
}

export function Seg<T extends string | number>({ value, options, onChange }: { value: T; options: { value: T; label: ReactNode; disabled?: boolean; title?: string }[]; onChange: (v: T) => void }) {
  return (
    <div className="seg" role="radiogroup">
      {options.map((o) => (
        <button type="button" key={String(o.value)} role="radio" aria-checked={o.value === value} title={o.title} disabled={o.disabled} className={o.value === value ? "on" : ""} onClick={() => onChange(o.value)}>
          {o.label}
        </button>
      ))}
    </div>
  );
}

export function Range({ value, min, max, step = 1, onChange, onCommit, label }: { value: number; min: number; max: number; step?: number; onChange: (v: number) => void; onCommit?: (v: number) => void; label?: string }) {
  const p = ((value - min) / (max - min)) * 100;
  return (
    <input
      aria-label={label}
      className="range"
      type="range"
      min={min}
      max={max}
      step={step}
      value={value}
      style={{ ["--p" as string]: `${p}%` }}
      onChange={(e) => onChange(Number(e.target.value))}
      onMouseUp={(e) => onCommit?.(Number((e.target as HTMLInputElement).value))}
      onKeyUp={(e) => onCommit?.(Number((e.target as HTMLInputElement).value))}
    />
  );
}

export function Kbd({ accel }: { accel: string | null | undefined }) {
  const parts = keyParts(accel);
  if (!parts.length) return null;
  return (
    <span className="kbd">
      {parts.map((p) => (
        <span key={p}>{p}</span>
      ))}
    </span>
  );
}

export function Setting({ title, desc, children }: { title: ReactNode; desc?: ReactNode; children: ReactNode }) {
  return (
    <div className="setting">
      <div className="txt">
        <b>{title}</b>
        {desc && <span>{desc}</span>}
      </div>
      <div className="ctl">{children}</div>
    </div>
  );
}

export function Bar({ value, muted, thin }: { value: number; muted?: boolean; thin?: boolean }) {
  return (
    <div className={`bar ${muted ? "muted" : ""} ${thin ? "thin" : ""}`}>
      <i style={{ width: `${Math.max(0, Math.min(100, value))}%` }} />
    </div>
  );
}

export function Select<T extends string>({ value, onChange, options, width, label }: { value: T; onChange: (v: T) => void; options: { value: T; label: string }[]; width?: number; label?: string }) {
  return (
    <select aria-label={label} className="select" value={value} style={{ width }} onChange={(e) => onChange(e.target.value as T)}>
      {options.map((o) => (
        <option key={o.value} value={o.value}>
          {o.label}
        </option>
      ))}
    </select>
  );
}

/** Click, then press a key combination. Escape cancels, Backspace clears. */
export function HotkeyInput({ value, onChange, validate }: { value: string | null; onChange: (v: string | null) => void; validate: (v: string) => Promise<string> }) {
  const [rec, setRec] = useState(false);
  const [err, setErr] = useState<string | null>(null);
  const ref = useRef<HTMLButtonElement>(null);
  useEffect(() => {
    if (!rec) return;
    const onKey = async (e: KeyboardEvent) => {
      e.preventDefault();
      e.stopPropagation();
      if (e.code === "Escape") {
        setRec(false);
        return;
      }
      if (e.code === "Backspace" && !e.ctrlKey && !e.altKey && !e.shiftKey && !e.metaKey) {
        onChange(null);
        setRec(false);
        return;
      }
      const acc = acceleratorFromEvent(e);
      if (!acc) return;
      try {
        const norm = await validate(acc);
        setErr(null);
        onChange(norm);
      } catch (x) {
        setErr(String(x));
      }
      setRec(false);
    };
    window.addEventListener("keydown", onKey, true);
    return () => window.removeEventListener("keydown", onKey, true);
  }, [rec, onChange, validate]);
  return (
    <span style={{ display: "inline-grid", gap: 4, justifyItems: "end" }}>
      <button ref={ref} type="button" className={`hotkey-input ${rec ? "rec" : value ? "set" : ""}`} onClick={() => setRec(true)} onBlur={() => setRec(false)}>
        {rec ? "Press keys… (Esc cancels)" : value ? <Kbd accel={value} /> : "Not set"}
      </button>
      {err && <span style={{ color: "#e8b4b4", fontSize: 11 }}>{err}</span>}
    </span>
  );
}

export function Modal({ children, onClose, small }: { children: ReactNode; onClose: () => void; small?: boolean }) {
  useEffect(() => {
    const k = (e: KeyboardEvent) => e.key === "Escape" && onClose();
    window.addEventListener("keydown", k);
    return () => window.removeEventListener("keydown", k);
  }, [onClose]);
  return (
    <div className="scrim" onMouseDown={(e) => e.target === e.currentTarget && onClose()}>
      <div className={`modal ${small ? "small" : ""}`} role="dialog" aria-modal="true">
        {children}
      </div>
    </div>
  );
}

export function useClickOutside(open: boolean, close: () => void) {
  const ref = useRef<HTMLDivElement>(null);
  useEffect(() => {
    if (!open) return;
    const h = (e: MouseEvent) => {
      if (ref.current && !ref.current.contains(e.target as Node)) close();
    };
    window.addEventListener("mousedown", h);
    return () => window.removeEventListener("mousedown", h);
  }, [open, close]);
  return ref;
}
