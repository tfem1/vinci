/** VINCI mark: a "V" cut by a play/aperture notch, monochrome. */
export function LogoMark({ size = 26 }: { size?: number }) {
  return (
    <svg className="mark" width={size} height={size} viewBox="0 0 64 64" aria-hidden="true">
      <rect x="1" y="1" width="62" height="62" rx="16" fill="#fff" />
      <path d="M17 18h9.5L32 36.5 37.5 18H47L36.6 46h-9.2z" fill="#090909" />
      <circle cx="46.5" cy="44.5" r="4.5" fill="#090909" />
    </svg>
  );
}

export function Wordmark() {
  return (
    <div className="brand">
      <LogoMark />
      <div>
        <div className="word">VINCI</div>
        <div className="tag">Capture Every Moment</div>
      </div>
    </div>
  );
}
