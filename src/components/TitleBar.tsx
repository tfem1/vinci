import { Minus, Square, X } from "lucide-react";
import { isTauri } from "../lib/api";

async function win() {
  const { getCurrentWindow } = await import("@tauri-apps/api/window");
  return getCurrentWindow();
}

export function TitleBar({ page }: { page: string }) {
  return (
    <div className="titlebar">
      <div className="drag" data-tauri-drag-region>
        <span className="crumb" data-tauri-drag-region>
          VINCI <span className="faint">/</span> <b>{page}</b>
        </span>
      </div>
      {isTauri && (
        <div className="winctl">
          <button aria-label="Minimize" onClick={async () => (await win()).minimize()}>
            <Minus size={14} />
          </button>
          <button aria-label="Maximize" onClick={async () => (await win()).toggleMaximize()}>
            <Square size={11} />
          </button>
          <button aria-label="Close" className="close" onClick={async () => (await win()).close()}>
            <X size={15} />
          </button>
        </div>
      )}
    </div>
  );
}
