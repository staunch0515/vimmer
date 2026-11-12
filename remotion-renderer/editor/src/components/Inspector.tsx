import type { Dispatch } from "react";
import type { Scene, SubtitleCue, VideoTimeline } from "../../../src/types/timeline";
import { timecode, type Selection, sortCues } from "../model";
import type { Action } from "../state";
import { canChooseMedia, chooseAndImport, type MediaKind } from "../media";
import { useState } from "react";

type Props = {
  timeline: VideoTimeline;
  selection: Selection;
  tab: "scene" | "subtitles";
  setTab: (tab: "scene" | "subtitles") => void;
  playheadMs: number;
  dispatch: Dispatch<Action>;
  onSelect: (s: Selection) => void;
  onAddSubtitle: () => void;
};

const MOTIONS: Scene["motion"][] = ["static", "zoom_in", "zoom_out", "pan_left", "pan_right"];

export const Inspector = (props: Props) => {
  const { tab, setTab } = props;
  return (
    <aside className="inspector panel">
      <div className="tabs" role="tablist">
        <button role="tab" aria-selected={tab === "scene"} className={tab === "scene" ? "tab active" : "tab"} onClick={() => setTab("scene")}>
          Scene
        </button>
        <button role="tab" aria-selected={tab === "subtitles"} className={tab === "subtitles" ? "tab active" : "tab"} onClick={() => setTab("subtitles")}>
          Subtitles <span className="count">{props.timeline.subtitles.length}</span>
        </button>
      </div>
      <div className="inspector-body">
        {tab === "scene" ? <SceneEditor {...props} /> : <SubtitleEditor {...props} />}
      </div>
    </aside>
  );
};

const SceneEditor = ({ timeline, selection, dispatch }: Props) => {
  if (selection.kind !== "scene") {
    return <Empty title="No scene selected" hint="Click a scene block on the timeline to edit it." />;
  }
  const index = selection.index;
  const scene = timeline.scenes[index];
  if (!scene) return <Empty title="Scene not found" hint="It may have been deleted." />;
  const patch = (p: Partial<Scene>) => dispatch({ type: "patchScene", index, patch: p });
  const isLast = index === timeline.scenes.length - 1;

  return (
    <div className="form">
      <div className="section-title">Scene {index + 1} of {timeline.scenes.length}</div>
      <Field label="ID">
        <input value={scene.id} onChange={(e) => patch({ id: e.target.value })} />
      </Field>
      <Field label="Duration" suffix="ms" hint={timecode(scene.duration_ms)}>
        <input
          type="number"
          min={100}
          step={100}
          value={scene.duration_ms}
          onChange={(e) => patch({ duration_ms: Math.max(100, Number(e.target.value) || 100) })}
        />
      </Field>
      <Field label="Text">
        <textarea rows={3} value={scene.text ?? ""} onChange={(e) => patch({ text: e.target.value || null })} />
      </Field>
      <Field label="Highlights" hint="Comma-separated words">
        <input
          value={scene.highlights.join(", ")}
          onChange={(e) => patch({ highlights: e.target.value.split(",").map((w) => w.trim()).filter(Boolean) })}
        />
      </Field>
      <Field label="Image" hint="Path relative to public/">
        <MediaInput
          kind="image"
          value={scene.image ?? ""}
          placeholder="assets/sample.png"
          onChange={(v) => patch({ image: v || null })}
        />
      </Field>
      <Field label="Motion">
        <select value={scene.motion} onChange={(e) => patch({ motion: e.target.value as Scene["motion"] })}>
          {MOTIONS.map((m) => (
            <option key={m} value={m}>{m.replace("_", " ")}</option>
          ))}
        </select>
      </Field>
      <div className="section-title">Audio</div>
      <label className="check">
        <input
          type="checkbox"
          checked={!!scene.narration}
          onChange={(e) => patch({ narration: e.target.checked ? { voice: null } : null })}
        />
        <span>Narrate the text (speech generated at render time)</span>
      </label>
      {scene.narration && (
        <Field label="Voice" hint={scene.audio ? "Manual audio is used instead" : ""}>
          <select
            value={scene.narration.voice ?? ""}
            onChange={(e) => patch({ narration: { voice: e.target.value || null } })}
          >
            <option value="">Auto (by language)</option>
            <option value="Samantha">Samantha (English)</option>
            <option value="Tingting">Tingting (Chinese)</option>
          </select>
        </Field>
      )}
      {scene.audio ? (
        <>
          <Field label="Source" hint="Path relative to public/">
            <MediaInput
              kind="audio"
              value={scene.audio.source}
              placeholder="assets/voice.wav"
              onChange={(v) => patch({ audio: { ...scene.audio!, source: v } })}
            />
          </Field>
          <Field label="Offset" suffix="ms">
            <input
              type="number"
              min={0}
              step={100}
              value={scene.audio.offset_ms}
              onChange={(e) => patch({ audio: { ...scene.audio!, offset_ms: Math.max(0, Number(e.target.value) || 0) } })}
            />
          </Field>
          <button className="btn ghost small" onClick={() => patch({ audio: null })}>Remove audio</button>
        </>
      ) : (
        <button className="btn ghost small" onClick={() => patch({ audio: { source: "", offset_ms: 0 } })}>Add audio</button>
      )}
      <div className="section-title">Arrange</div>
      <div className="button-row">
        <button className="btn ghost small" disabled={index === 0} onClick={() => dispatch({ type: "moveScene", index, delta: -1 })}>Move up</button>
        <button className="btn ghost small" disabled={isLast} onClick={() => dispatch({ type: "moveScene", index, delta: 1 })}>Move down</button>
        <button className="btn ghost small" onClick={() => dispatch({ type: "duplicateScene", index })}>Duplicate</button>
        <button className="btn danger small" onClick={() => dispatch({ type: "deleteScene", index })}>Delete</button>
      </div>
      <button className="btn small full" onClick={() => dispatch({ type: "addScene" })}>+ Add scene at end</button>
    </div>
  );
};

const SubtitleEditor = ({ timeline, selection, playheadMs, dispatch, onSelect, onAddSubtitle }: Props) => {
  const sorted = sortCues(timeline.subtitles);
  const selectedIndex = selection.kind === "subtitle" ? selection.index : -1;
  const cue: SubtitleCue | undefined = timeline.subtitles[selectedIndex];

  return (
    <div className="form">
      <button className="btn primary small full" onClick={onAddSubtitle}>+ Cue at playhead ({timecode(playheadMs)})</button>
      <div className="section-title">Cues</div>
      {sorted.length === 0 && <Empty title="No subtitles yet" hint="Add a cue at the playhead, or import an SRT from the CLI." />}
      <ul className="cue-list">
        {sorted.map((c) => {
          const i = timeline.subtitles.indexOf(c);
          return (
            <li key={i}>
              <button className={i === selectedIndex ? "cue active" : "cue"} onClick={() => onSelect({ kind: "subtitle", index: i })}>
                <span className="mono dim">{timecode(c.start_ms)}</span>
                <span className="cue-text">{c.text}</span>
              </button>
            </li>
          );
        })}
      </ul>
      {cue && (
        <>
          <div className="section-title">Selected cue</div>
          <Field label="Text">
            <textarea rows={2} value={cue.text} onChange={(e) => dispatch({ type: "patchSubtitle", index: selectedIndex, patch: { text: e.target.value } })} />
          </Field>
          <div className="two-col">
            <Field label="Start" suffix="ms">
              <input type="number" min={0} step={10} value={cue.start_ms} onChange={(e) => dispatch({ type: "patchSubtitle", index: selectedIndex, patch: { start_ms: Math.max(0, Number(e.target.value) || 0) } })} />
            </Field>
            <Field label="End" suffix="ms">
              <input type="number" min={0} step={10} value={cue.end_ms} onChange={(e) => dispatch({ type: "patchSubtitle", index: selectedIndex, patch: { end_ms: Math.max(0, Number(e.target.value) || 0) } })} />
            </Field>
          </div>
          <button className="btn danger small" onClick={() => dispatch({ type: "deleteSubtitle", index: selectedIndex })}>Delete cue</button>
        </>
      )}
    </div>
  );
};

const Field = ({ label, hint, suffix, children }: { label: string; hint?: string; suffix?: string; children: React.ReactNode }) => (
  <label className="field">
    <span className="field-label">
      {label}
      {hint && <span className="dim mono"> {hint}</span>}
      {suffix && <span className="suffix">{suffix}</span>}
    </span>
    {children}
  </label>
);

const Empty = ({ title, hint }: { title: string; hint: string }) => (
  <div className="empty">
    <div className="empty-title">{title}</div>
    <div className="dim">{hint}</div>
  </div>
);

/** A path field with a Choose button that copies the picked file into the public folder. */
const MediaInput = ({
  kind,
  value,
  placeholder,
  onChange,
}: {
  kind: MediaKind;
  value: string;
  placeholder: string;
  onChange: (value: string) => void;
}) => {
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const choose = async () => {
    setBusy(true);
    setError(null);
    try {
      const path = await chooseAndImport(kind);
      if (path) onChange(path);
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  };
  return (
    <div className="media-field">
      <div className="media-input">
        <input value={value} placeholder={placeholder} onChange={(e) => onChange(e.target.value)} />
        <button
          type="button"
          className="btn small"
          disabled={!canChooseMedia() || busy}
          title={canChooseMedia() ? "Choose a file; it is copied into the project's public folder" : "Choosing files needs the desktop app"}
          onClick={() => void choose()}
        >
          {busy ? "…" : "Choose"}
        </button>
      </div>
      {error && <span className="field-error">{error}</span>}
    </div>
  );
};
