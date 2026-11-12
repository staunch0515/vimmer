import { useRef } from "react";
import type { VideoTimeline } from "../../../src/types/timeline";
import { ALLOWED_RESOLUTIONS } from "../model";

type Props = {
  timeline: VideoTimeline;
  problems: string[];
  canUndo: boolean;
  canRedo: boolean;
  onUndo: () => void;
  onRedo: () => void;
  onLoad: (t: VideoTimeline) => void;
  onResolution: (w: number, h: number) => void;
};

const isTimeline = (v: unknown): v is VideoTimeline => {
  const t = v as VideoTimeline;
  return !!t && typeof t.fps === "number" && Array.isArray(t.scenes) && !!t.resolution;
};

export const Toolbar = ({ timeline, problems, canUndo, canRedo, onUndo, onRedo, onLoad, onResolution }: Props) => {
  const fileInput = useRef<HTMLInputElement>(null);
  const current = `${timeline.resolution.width}x${timeline.resolution.height}`;

  const importJson = async (file: File) => {
    try {
      const parsed: unknown = JSON.parse(await file.text());
      if (!isTimeline(parsed)) throw new Error("File is not a timeline (missing fps, resolution or scenes).");
      onLoad({ ...parsed, subtitles: parsed.subtitles ?? [] });
    } catch (e) {
      window.alert(`Import failed: ${(e as Error).message}`);
    }
  };

  const exportJson = () => {
    const blob = new Blob([JSON.stringify(timeline, null, 2) + "\n"], { type: "application/json" });
    const url = URL.createObjectURL(blob);
    const a = document.createElement("a");
    a.href = url;
    a.download = "timeline.json";
    a.click();
    URL.revokeObjectURL(url);
  };

  return (
    <header className="toolbar glass">
      <div className="brand">
        <span className="logo" aria-hidden />
        <span>Vimmer</span>
        <span className="brand-sub">Studio</span>
      </div>
      <div className="divider" />
      <button className="btn ghost" onClick={onUndo} disabled={!canUndo} title="Undo (⌘Z)">Undo</button>
      <button className="btn ghost" onClick={onRedo} disabled={!canRedo} title="Redo (⇧⌘Z)">Redo</button>
      <div className="divider" />
      <label className="field-inline">
        <span className="dim">Format</span>
        <select value={current} onChange={(e) => {
          const [w, h] = e.target.value.split("x").map(Number);
          onResolution(w, h);
        }}>
          {ALLOWED_RESOLUTIONS.map((r) => (
            <option key={r.label} value={`${r.width}x${r.height}`}>{r.label}</option>
          ))}
        </select>
      </label>
      <span className="spacer" />
      <span className={`status ${problems.length ? "warn" : "ok"}`}>
        <span className="dot" />
        {problems.length ? `${problems.length} issue${problems.length > 1 ? "s" : ""}` : "Ready to render"}
      </span>
      <input
        ref={fileInput}
        type="file"
        accept="application/json,.json"
        hidden
        onChange={(e) => {
          const f = e.target.files?.[0];
          if (f) void importJson(f);
          e.target.value = "";
        }}
      />
      <button className="btn" onClick={() => fileInput.current?.click()}>Import JSON</button>
      <button className="btn primary" onClick={exportJson}>Export JSON</button>
    </header>
  );
};
