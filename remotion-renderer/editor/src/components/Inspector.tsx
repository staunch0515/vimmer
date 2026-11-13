import type { Dispatch } from "react";
import type { Keyframe, Scene, SubtitleCue, TransitionKind, VideoTimeline } from "../../../src/types/timeline";
import {
  DEFAULT_TRANSITION_MS, EASINGS, TRANSITION_KINDS, keyframeFieldError, timecode, type Selection, sortCues,
} from "../model";
import { sampleKeyframes } from "../../../src/motion/keyframes";
import type { Action } from "../state";
import { canChooseMedia, chooseAndImport, type MediaKind } from "../media";
import { useEffect, useState } from "react";
import { AssistantPanel } from "./AssistantPanel";

export type InspectorTab = "scene" | "subtitles" | "music" | "assistant";

type Props = {
  timeline: VideoTimeline;
  selection: Selection;
  tab: InspectorTab;
  setTab: (tab: InspectorTab) => void;
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
        <button role="tab" aria-selected={tab === "music"} className={tab === "music" ? "tab active" : "tab"} onClick={() => setTab("music")}>
          Music
        </button>
        <button role="tab" aria-selected={tab === "assistant"} className={tab === "assistant" ? "tab active" : "tab"} onClick={() => setTab("assistant")}>
          Assistant
        </button>
      </div>
      <div className="inspector-body">
        {tab === "scene" && <SceneEditor {...props} />}
        {tab === "subtitles" && <SubtitleEditor {...props} />}
        {tab === "music" && <MusicEditor {...props} />}
        {/* Stays mounted so a pending proposal survives switching tabs. */}
        <div hidden={tab !== "assistant"}>
          <AssistantPanel timeline={props.timeline} selection={props.selection} dispatch={props.dispatch} />
        </div>
      </div>
    </aside>
  );
};

const SceneEditor = ({ timeline, selection, playheadMs, dispatch }: Props) => {
  if (selection.kind !== "scene" && selection.kind !== "keyframe") {
    return <Empty title="No scene selected" hint="Click a scene block on the timeline to edit it." />;
  }
  const index = selection.kind === "scene" ? selection.index : selection.sceneIndex;
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
      <div className="section-title">Transition in</div>
      {index === 0 ? (
        <div className="dim">The first scene has nothing to transition from.</div>
      ) : (
        <>
          <Field label="Type">
            <select
              value={scene.transition_in?.kind ?? "none"}
              onChange={(e) => {
                const v = e.target.value;
                dispatch({
                  type: "setTransition",
                  index,
                  transition: v === "none" ? null : {
                    kind: v as TransitionKind,
                    duration_ms: Math.min(scene.transition_in?.duration_ms ?? DEFAULT_TRANSITION_MS, scene.duration_ms),
                  },
                });
              }}
            >
              <option value="none">none</option>
              {TRANSITION_KINDS.map((k) => (
                <option key={k} value={k}>{k.replace("_", " ")}</option>
              ))}
            </select>
          </Field>
          {scene.transition_in && (
            <NumField
              label="Duration"
              suffix="ms"
              hint={`max ${scene.duration_ms}`}
              value={scene.transition_in.duration_ms}
              step={100}
              validate={(v) =>
                !Number.isInteger(v) || v <= 0 ? "Must be a whole number above 0."
                : v > scene.duration_ms ? `Cannot be longer than the scene (${scene.duration_ms} ms).` : null}
              onCommit={(v) => dispatch({ type: "setTransition", index, transition: { ...scene.transition_in!, duration_ms: v } })}
            />
          )}
        </>
      )}
      <div className="section-title">Keyframes</div>
      <KeyframeEditor
        scene={scene}
        sceneIndex={index}
        selectedIndex={selection.kind === "keyframe" ? selection.index : -1}
        sceneStartMs={timeline.scenes.slice(0, index).reduce((a, s) => a + s.duration_ms, 0)}
        playheadMs={playheadMs}
        dispatch={dispatch}
      />
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

const MusicEditor = ({ timeline, dispatch }: Props) => {
  const music = timeline.music;
  if (!music) {
    return (
      <div className="form">
        <Empty title="No background music" hint="Music loops under the whole video and fades at the ends." />
        <Field label="Music file" hint="Path relative to public/">
          <MediaInput
            kind="audio"
            value=""
            placeholder="assets/music.mp3"
            onChange={(source) => source && dispatch({ type: "setMusic", music: { source, volume: 0.3, fade_in_ms: 0, fade_out_ms: 0 } })}
          />
        </Field>
      </div>
    );
  }
  const patch = (p: Partial<typeof music>) => dispatch({ type: "patchMusic", patch: p });
  return (
    <div className="form">
      <div className="section-title">Background music</div>
      <Field label="File" hint="Path relative to public/">
        <MediaInput kind="audio" value={music.source} placeholder="assets/music.mp3" onChange={(v) => patch({ source: v })} />
      </Field>
      <Field label="Volume" suffix={`${Math.round(music.volume * 100)} %`}>
        <input
          type="range"
          min={0}
          max={1}
          step={0.05}
          value={music.volume}
          onChange={(e) => patch({ volume: Number(e.target.value) })}
        />
      </Field>
      <div className="two-col">
        <Field label="Fade in" suffix="ms">
          <input type="number" min={0} step={100} value={music.fade_in_ms} onChange={(e) => patch({ fade_in_ms: Math.max(0, Number(e.target.value) || 0) })} />
        </Field>
        <Field label="Fade out" suffix="ms">
          <input type="number" min={0} step={100} value={music.fade_out_ms} onChange={(e) => patch({ fade_out_ms: Math.max(0, Number(e.target.value) || 0) })} />
        </Field>
      </div>
      <button className="btn danger small" onClick={() => dispatch({ type: "setMusic", music: null })}>Remove music</button>
    </div>
  );
};

/** Number input that only commits values passing `validate`; otherwise it shows the reason. */
const NumField = ({
  label, suffix, hint, value, step, validate, onCommit,
}: {
  label: string; suffix?: string; hint?: string; value: number; step: number;
  validate: (v: number) => string | null; onCommit: (v: number) => void;
}) => {
  const [text, setText] = useState(String(value));
  const [error, setError] = useState<string | null>(null);
  useEffect(() => {
    setText(String(value));
    setError(null);
  }, [value]);
  return (
    <Field label={label} suffix={suffix} hint={hint}>
      <input
        type="number"
        step={step}
        value={text}
        aria-invalid={error ? true : undefined}
        onChange={(e) => {
          setText(e.target.value);
          const v = e.target.value.trim() === "" ? NaN : Number(e.target.value);
          const err = Number.isFinite(v) ? validate(v) : "Enter a number.";
          setError(err);
          if (!err) onCommit(v);
        }}
        onBlur={() => {
          setText(String(value));
          setError(null);
        }}
      />
      {error && <span className="field-error">{error}</span>}
    </Field>
  );
};

const KeyframeEditor = ({
  scene, sceneIndex, selectedIndex, sceneStartMs, playheadMs, dispatch,
}: {
  scene: Scene; sceneIndex: number; selectedIndex: number; sceneStartMs: number; playheadMs: number;
  dispatch: Dispatch<Action>;
}) => {
  const list = scene.keyframes ?? [];
  const rel = Math.round(playheadMs - sceneStartMs);
  const inScene = rel >= 0 && rel <= scene.duration_ms;
  const taken = list.some((k) => k.at_ms === rel);
  const kf: Keyframe | undefined = list[selectedIndex];
  const add = () => {
    const r = (v: number) => Math.round(v * 1000) / 1000;
    const p = sampleKeyframes(list, rel);
    const keyframe: Keyframe = list.length === 0
      ? { at_ms: rel }
      : { at_ms: rel, scale: r(p.scale), x: r(p.x), y: r(p.y), opacity: r(p.opacity), rotation: r(p.rotation) };
    dispatch({ type: "addKeyframe", sceneIndex, keyframe });
  };
  const patch = (p: Partial<Keyframe>) => dispatch({ type: "patchKeyframe", sceneIndex, index: selectedIndex, patch: p });
  const num = (field: "scale" | "x" | "y" | "opacity" | "rotation", fallback: number, label: string, step: number, suffix?: string) => (
    <NumField
      label={label}
      suffix={suffix}
      value={kf![field] ?? fallback}
      step={step}
      validate={(v) => keyframeFieldError(field, v)}
      onCommit={(v) => patch({ [field]: v })}
    />
  );
  return (
    <>
      <button className="btn primary small full" disabled={!inScene || taken} onClick={add}
        title={!inScene ? "Move the playhead into this scene" : taken ? "A keyframe already exists at the playhead" : undefined}>
        + Keyframe at playhead ({inScene ? `${rel} ms` : "outside scene"})
      </button>
      {list.length > 0 && <div className="dim">A non-empty keyframe list replaces the scene's Motion.</div>}
      <ul className="cue-list">
        {list.map((k, i) => (
          <li key={i}>
            <button
              className={i === selectedIndex ? "cue active" : "cue"}
              onClick={() => dispatch({ type: "select", selection: { kind: "keyframe", sceneIndex, index: i } })}
            >
              <span className="mono dim">{k.at_ms} ms</span>
              <span className="cue-text">scale {k.scale ?? 1} · {k.ease ?? "ease_in_out"}</span>
            </button>
          </li>
        ))}
      </ul>
      {kf && (
        <>
          <div className="section-title">Keyframe at {kf.at_ms} ms</div>
          <div className="two-col">
            {num("scale", 1, "Scale", 0.05)}
            {num("opacity", 1, "Opacity", 0.1)}
          </div>
          <div className="two-col">
            {num("x", 0, "X", 1, "%")}
            {num("y", 0, "Y", 1, "%")}
          </div>
          {num("rotation", 0, "Rotation", 1, "°")}
          <Field label="Ease" hint="into the next keyframe">
            <select value={kf.ease ?? "ease_in_out"} onChange={(e) => patch({ ease: e.target.value as Keyframe["ease"] })}>
              {EASINGS.map((e) => (
                <option key={e} value={e}>{e.replace(/_/g, " ")}</option>
              ))}
            </select>
          </Field>
          <button className="btn danger small" onClick={() => dispatch({ type: "deleteKeyframe", sceneIndex, index: selectedIndex })}>
            Delete keyframe
          </button>
        </>
      )}
    </>
  );
};
