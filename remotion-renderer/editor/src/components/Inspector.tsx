import type { Dispatch } from "react";
import type { Keyframe, Scene, SubtitleCue, TransitionKind, VideoClip, VideoTimeline } from "../../../src/types/timeline";
import {
  DEFAULT_TRANSITION_MS, EASINGS, TRANSITION_KINDS, keyframeFieldError, timecode, type Selection, sortCues,
} from "../model";
import { sampleKeyframes } from "../../../src/motion/keyframes";
import { boxOf } from "../subtitleBox";
import type { Action } from "../state";
import { canChooseMedia, chooseAndImport, type MediaKind } from "../media";
import { ClipFields } from "./ClipFields";
import { DEFAULT_CLIP } from "../layers";
import { useEffect, useState } from "react";
import { AssistantPanel } from "./AssistantPanel";
import { LayerPanel } from "./LayerPanel";
import { TextStyleFields, withPart } from "./TextStyleFields";
import { BrandEditor, TemplateParams } from "./BrandPanel";
import { SceneAvatarFields, type AvatarController } from "./AvatarFields";
import type { CheckFinding } from "../renderOptions";
import { BeatSyncFields, SfxList, type BeatStatus } from "./SoundPanel";
import type { DetectedBeats } from "../beats";
import { MoodMusicFields, MusicModeSwitch } from "./MusicPanel";
import type { MusicLibrary, PickedTrack } from "../music";
import { clipFor, spokenText, withSpokenLine } from "../narration";
import { PageList, TextLayoutFields } from "./TextLayoutFields";
import { KEYWORD_YELLOW, addHighlight, insertPageBreak, removeHighlight, withLayoutPart } from "../textLayout";
import { pagePreview, resolveTextLayout } from "../../../src/textPages";
import { LookFields } from "./LookFields";
import { ListenButton, type ListenProps } from "./ListenButton";
import { autoPreset } from "../../../src/motion/look";
import { VoiceoverPanel, type VoicePanelProps } from "./VoiceoverPanel";
import { VoicePicker } from "./VoicePicker";

export type InspectorTab = "scene" | "subtitles" | "music" | "voice" | "brand" | "assistant";

type Props = {
  timeline: VideoTimeline;
  selection: Selection;
  tab: InspectorTab;
  setTab: (tab: InspectorTab) => void;
  playheadMs: number;
  dispatch: Dispatch<Action>;
  onSelect: (s: Selection) => void;
  onAddSubtitle: () => void;
  beats: DetectedBeats | null;
  beatStatus: BeatStatus;
  onDetectBeats: () => void;
  musicLibrary: MusicLibrary | { error: string } | null;
  onLoadMusicLibrary: () => void;
  pickedTrack: PickedTrack | null;
  pickError: string | null;
  /** URL for a public-relative file (auditioning a music track). */
  audioUrl: (path: string) => string;
  /** Moves the playhead to a timeline time in ms. */
  onSeekMs?: (ms: number) => void;
  /** Changes when another document is opened (the assistant's chat starts over). */
  docKey: number;
  /** 试听: hearing a scene's narration before a render. Missing: no Listen button. */
  listen?: ListenProps;
  /** The voice-over track (旁白音轨). Missing: no 旁白 tab. */
  voice?: VoicePanelProps;
  /** 数字人讲解人: engine, status and job; the latest check's findings for one-click fixes. */
  avatar?: { av: AvatarController; findings: CheckFinding[]; onCheck?: () => void };
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
        {props.voice && (
          <button role="tab" aria-selected={tab === "voice"} className={tab === "voice" ? "tab active" : "tab"} onClick={() => setTab("voice")} data-tab="voice">
            旁白 <span className="count">{props.timeline.voiceover?.length ?? 0}</span>
          </button>
        )}
        <button role="tab" aria-selected={tab === "brand"} className={tab === "brand" ? "tab active" : "tab"} onClick={() => setTab("brand")} data-tab="brand">
          Brand
        </button>
        <button role="tab" aria-selected={tab === "assistant"} className={tab === "assistant" ? "tab active" : "tab"} onClick={() => setTab("assistant")}>
          Assistant
        </button>
      </div>
      <div className="inspector-body">
        {tab === "scene" && <SceneEditor {...props} />}
        {tab === "subtitles" && <SubtitleEditor {...props} />}
        {tab === "music" && <MusicEditor {...props} />}
        {tab === "voice" && props.voice && <VoiceoverPanel {...props} voice={props.voice} />}
        {tab === "brand" && <BrandEditor timeline={props.timeline} selection={props.selection} dispatch={props.dispatch} av={props.avatar?.av} />}
        {/* Stays mounted so the chat and a pending proposal survive switching tabs. */}
        <div hidden={tab !== "assistant"}>
          <AssistantPanel timeline={props.timeline} selection={props.selection} dispatch={props.dispatch} docKey={props.docKey} />
        </div>
      </div>
    </aside>
  );
};

const SceneEditor = ({ timeline, selection, playheadMs, dispatch, onSeekMs, listen, voice, avatar }: Props) => {
  if (selection.kind !== "scene" && selection.kind !== "keyframe" && selection.kind !== "layer") {
    return <Empty title="No scene selected" hint="Click a scene block on the timeline to edit it." />;
  }
  const index = selection.kind === "scene" ? selection.index : selection.sceneIndex;
  const scene = timeline.scenes[index];
  if (!scene) return <Empty title="Scene not found" hint="It may have been deleted." />;
  const patch = (p: Partial<Scene>) => dispatch({ type: "patchScene", index, patch: p });
  const isLast = index === timeline.scenes.length - 1;
  const sceneStartMs = timeline.scenes.slice(0, index).reduce((a, s) => a + s.duration_ms, 0);

  return (
    <div className="form">
      <div className="section-title">Scene {index + 1} of {timeline.scenes.length}</div>
      <Field label="ID">
        <input value={scene.id} onChange={(e) => patch({ id: e.target.value })} />
      </Field>
      {/* Committed on Enter or blur, not per keystroke: typing "5000" must not pass through a
          5 ms scene that clamps the keyframes and layers on the way. */}
      <CommitNumField
        label="Duration"
        suffix="ms"
        hint={timecode(scene.duration_ms)}
        value={scene.duration_ms}
        step={100}
        min={100}
        validate={(v) => (Number.isInteger(v) && v >= 100 ? null : "A whole number, at least 100.")}
        onCommit={(v) => patch({ duration_ms: v })}
        data-scene-duration
      />
      {scene.template && <TemplateParams index={index} template={scene.template} dispatch={dispatch} />}
      <Field label="Text" hint={scene.template ? "drawn over the template" : undefined}>
        <textarea rows={3} data-scene-text value={scene.text ?? ""} onChange={(e) => patch({ text: e.target.value || null })} />
      </Field>
      {/* What the narration says, when it differs from the text on screen. Empty: it reads the
          text above; clearing it removes `narration.text`. */}
      {scene.narration && (
        <Field label="台词 Spoken line" hint={scene.audio ? "Manual audio is used instead" : "可选，与屏幕文字不同时填写"}>
          <textarea
            rows={2}
            data-spoken-line
            value={scene.narration.text ?? ""}
            placeholder="留空 = 念屏幕上的文字"
            onChange={(e) => patch({ narration: withSpokenLine(scene.narration!, e.target.value) })}
          />
        </Field>
      )}
      <Field label="关键词高亮 Highlights" hint="逗号分隔；中文里也可以只高亮几个字">
        <input
          data-highlights
          value={(scene.highlights ?? []).join(", ")}
          onChange={(e) => patch({ highlights: e.target.value.split(/[,，]/).map((w) => w.trim()).filter(Boolean) })}
        />
        <div className="row" style={{ flexWrap: "wrap", gap: 6, marginTop: 6 }}>
          <button
            type="button"
            className="btn small"
            data-highlight-selection
            title="先在上面的文字里选中几个字，再点这里"
            // Keep the text box's selection: the click must not take the focus.
            onMouseDown={(e) => e.preventDefault()}
            onClick={() => {
              const el = document.querySelector<HTMLTextAreaElement>("textarea[data-scene-text]");
              const picked = el ? el.value.slice(el.selectionStart, el.selectionEnd) : "";
              const next = addHighlight(scene.highlights, picked);
              if (next.length !== (scene.highlights ?? []).length) patch({ highlights: next });
            }}
          >
            ★ 高亮选中的字
          </button>
          {(scene.highlights ?? []).map((w) => (
            <span key={w} className="chip" data-highlight-chip={w} style={{ color: timeline.brand?.colors?.accent ?? undefined }}>
              {w}{" "}
              <button type="button" className="btn ghost small" aria-label={`取消高亮 ${w}`} onClick={() => patch({ highlights: removeHighlight(scene.highlights, w) })}>
                ×
              </button>
            </span>
          ))}
        </div>
        {(timeline.brand?.colors?.accent ?? "").toLowerCase() !== KEYWORD_YELLOW && (
          <div className="row dim" style={{ marginTop: 4 }}>
            高亮颜色 = 品牌的 Accent 色
            <button
              type="button"
              className="btn ghost small"
              data-keyword-yellow
              onClick={() =>
                dispatch({
                  type: "setBrand",
                  brand: { ...(timeline.brand ?? {}), colors: { ...(timeline.brand?.colors ?? {}), accent: KEYWORD_YELLOW } },
                })
              }
            >
              <span style={{ display: "inline-block", width: 10, height: 10, background: KEYWORD_YELLOW, borderRadius: 2 }} /> 关键词用黄色
            </button>
          </div>
        )}
      </Field>
      <div className="section-title">文字 Text</div>
      <TextLayoutFields
        name="scene"
        layout={scene.text_layout}
        inherited={timeline.brand?.text_layout}
        onChange={(part, value) => patch({ text_layout: withLayoutPart(scene.text_layout, part, value) ?? undefined })}
      />
      {(() => {
        const paging = resolveTextLayout(scene.text_layout, timeline.brand).paging;
        if (paging === "off") return null;
        const ownText = Boolean(scene.narration) && !scene.audio && spokenText(scene) === scene.text;
        const clip = ownText && listen ? clipFor(listen.heard, scene, listen.engine) : null;
        return (
          <PageList
            pages={pagePreview(scene, timeline.brand, timeline.resolution, clip?.speech_ms)}
            paging={paging}
            narrated={ownText}
            heard={clip != null}
            onSeek={(ms) => onSeekMs?.(sceneStartMs + Math.min(ms, scene.duration_ms - 1))}
            onInsertBreak={() => {
              const el = document.querySelector<HTMLTextAreaElement>("textarea[data-scene-text]");
              const text = scene.text ?? "";
              const at = el && document.activeElement === el ? el.selectionStart : (el?.selectionStart ?? text.length);
              patch({ text: insertPageBreak(text, at).text });
            }}
          />
        );
      })()}
      <div className="section-title">Text style</div>
      <TextStyleFields
        name="scene"
        style={scene.text_style}
        inherited={timeline.brand?.text_style}
        builtinShadow="soft"
        onChange={(part, value) => patch({ text_style: withPart(scene.text_style, part, value) ?? undefined })}
      />
      <Field label="Background">
        <select
          value={scene.video ? "video" : "image"}
          data-scene-background
          onChange={(e) => {
            // One background at a time; an undefined key is left out of the saved JSON.
            if (e.target.value === "video") patch({ image: null, video: { src: DEFAULT_CLIP } });
            else patch({ video: undefined });
          }}
        >
          <option value="image">Image</option>
          <option value="video">Video clip</option>
        </select>
      </Field>
      {scene.video ? (
        <ClipFields
          clip={scene.video}
          patch={(p) => {
            const merged = Object.fromEntries(
              Object.entries({ ...scene.video, ...p }).filter(([, v]) => v !== undefined),
            ) as unknown as VideoClip;
            patch({ video: merged });
          }}
        />
      ) : (
        <Field label="Image" hint="Path relative to public/">
          <MediaInput
            kind="image"
            value={scene.image ?? ""}
            placeholder="assets/sample.png"
            onChange={(v) => patch({ image: v || null })}
          />
        </Field>
      )}
      <Field label="Motion">
        <select value={scene.motion} onChange={(e) => patch({ motion: e.target.value as Scene["motion"] })}>
          {MOTIONS.map((m) => (
            <option key={m} value={m}>{m.replace("_", " ")}</option>
          ))}
        </select>
      </Field>
      <div className="section-title">画面氛围 Look</div>
      <LookFields
        scope="scene"
        look={scene.look}
        inherited={timeline.look}
        autoAs={autoPreset(timeline)}
        onChange={(look) => patch({ look })}
      />
      <SceneAvatarFields timeline={timeline} index={index} dispatch={dispatch} findings={avatar?.findings ?? []} onCheck={avatar?.onCheck} />
      <LayerPanel
        scene={scene}
        sceneIndex={index}
        selectedIndex={selection.kind === "layer" ? selection.index : -1}
        selectedKeyframe={selection.kind === "layer" ? selection.keyframe : undefined}
        playheadMs={playheadMs}
        sceneStartMs={sceneStartMs}
        onSeekMs={onSeekMs}
        dispatch={dispatch}
        brand={timeline.brand}
      />
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
        sceneStartMs={sceneStartMs}
        playheadMs={playheadMs}
        dispatch={dispatch}
      />
      <div className="section-title">朗读 Narration</div>
      <label className="check">
        <input
          type="checkbox"
          checked={!!scene.narration}
          onChange={(e) => patch({ narration: e.target.checked ? { voice: null } : null })}
        />
        <span>Narrate the text (speech generated at render time)</span>
      </label>
      {scene.narration && (
        <Field label="声音 Voice" hint={scene.audio ? "Manual audio is used instead" : ""}>
          <VoicePicker
            narration={scene.narration}
            spoken={spokenText(scene)}
            onChange={(n) => patch({ narration: n })}
            audioUrl={listen?.audioUrl}
          />
        </Field>
      )}
      {/* Keyed by scene: switching scenes unmounts it, which stops the sound. */}
      {scene.narration && listen && <ListenButton key={`${index}:${scene.id}`} scene={scene} listen={listen} />}
      <div className="section-title">本幕音频（跟着场景走）</div>
      <p className="dim small-text">
        只在这一幕里播放，到这一幕结束就停。整段的旁白请放到「旁白」音轨，它不会被场景切换截断。
      </p>
      {voice?.sceneRecorder?.(index)}
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
          <div className="button-row">
            <button className="btn ghost small" onClick={() => patch({ audio: null })}>Remove audio</button>
            {voice && (
              <MoveToTrackButton onMove={() => voice.onMoveSceneAudio(index)} disabled={!scene.audio.source.trim()} />
            )}
          </div>
        </>
      ) : (
        <button className="btn ghost small" onClick={() => patch({ audio: { source: "", offset_ms: 0 } })}>Add audio</button>
      )}
      <div className="section-title">Sound effects</div>
      <SfxList scene={scene} index={index} dispatch={dispatch} />
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
      <SubtitlePosition timeline={timeline} dispatch={dispatch} />
    </div>
  );
};

/** Where every cue sits (`subtitle_box`): fields, and the reset to the built-in place. */
const SubtitlePosition = ({ timeline, dispatch }: { timeline: VideoTimeline; dispatch: Dispatch<Action> }) => {
  const box = boxOf(timeline.subtitle_box, timeline.resolution);
  const set = (p: Partial<typeof box>) => dispatch({ type: "setSubtitleBox", box: { ...box, ...p } });
  return (
    <div data-subtitle-position>
      <div className="section-title">Position <span className="dim">all cues</span></div>
      <div className="dim">
        {timeline.subtitle_box ? "Moved." : "Built-in place."} Select a cue, then drag its box in the preview or use the arrow keys (Shift: 10 px).
      </div>
      <div className="two-col">
        <NumField
          label="X"
          hint="center"
          value={Math.round(box.x * 1000) / 1000}
          step={0.01}
          validate={(v) => (v >= 0 && v <= 1 ? null : "0 to 1")}
          onCommit={(v) => set({ x: v })}
        />
        <NumField
          label="Y"
          hint="bottom edge"
          value={Math.round(box.y * 1000) / 1000}
          step={0.01}
          validate={(v) => (v > 0 && v <= 1 ? null : "above 0, at most 1")}
          onCommit={(v) => set({ y: v })}
        />
      </div>
      <button className="btn ghost small" disabled={!timeline.subtitle_box} onClick={() => dispatch({ type: "setSubtitleBox", box: null })}>
        Reset to the built-in place
      </button>
    </div>
  );
};

// 「移到旁白音轨」: one undo step; an error (a clip already playing there) is shown under it.
const MoveToTrackButton = ({ onMove, disabled }: { onMove: () => string | null; disabled: boolean }) => {
  const [error, setError] = useState<string | null>(null);
  return (
    <>
      <button
        className="btn ghost small"
        disabled={disabled}
        title="The file plays at the same time on the voice-over track, and is no longer cut at the scene end"
        onClick={() => setError(onMove())}
        data-move-to-track
      >
        移到旁白音轨
      </button>
      {error && <span className="field-error" role="alert">{error}</span>}
    </>
  );
};

export const Field = ({ label, hint, suffix, children }: { label: string; hint?: string; suffix?: string; children: React.ReactNode }) => (
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
export const MediaInput = ({
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

const MusicEditor = (props: Props) => {
  const { timeline, dispatch, beats, beatStatus, onDetectBeats, musicLibrary, onLoadMusicLibrary, pickedTrack, pickError, audioUrl } = props;
  const music = timeline.music;
  useEffect(() => {
    if (!musicLibrary) onLoadMusicLibrary();
  }, [musicLibrary, onLoadMusicLibrary]);
  const library = musicLibrary && !("error" in musicLibrary) ? musicLibrary : null;
  const libraryError = musicLibrary && "error" in musicLibrary ? musicLibrary.error : null;
  const firstMood = library?.main_moods.includes("calm") ? "calm" : library?.main_moods[0] ?? "calm";
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
        <button
          className="btn small"
          data-music-add-mood
          onClick={() => dispatch({ type: "setMusic", music: { pick: { mood: firstMood }, volume: 0.3, fade_in_ms: 0, fade_out_ms: 0 } })}
        >
          Or pick music by mood…
        </button>
        {libraryError && <span className="field-error">{libraryError}</span>}
      </div>
    );
  }
  const patch = (p: Partial<typeof music>) => dispatch({ type: "patchMusic", patch: p });
  // Beat sync works on the file the render will play (the chosen track for a pick).
  const playing = music.pick ? (pickedTrack ? { ...music, pick: undefined, source: pickedTrack.track.source } : null) : music;
  return (
    <div className="form">
      <div className="section-title">Background music</div>
      <MusicModeSwitch music={music} patch={patch} firstMood={firstMood} pickedTrack={pickedTrack} />
      {music.pick ? (
        <MoodMusicFields
          pick={music.pick}
          patch={patch}
          library={library}
          libraryError={libraryError}
          pickedTrack={pickedTrack}
          pickError={pickError}
          audioUrl={audioUrl}
        />
      ) : (
        <Field label="File" hint="Path relative to public/">
          <MediaInput kind="audio" value={music.source ?? ""} placeholder="assets/music.mp3" onChange={(v) => patch({ source: v })} />
        </Field>
      )}
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
      {playing && <BeatSyncFields music={playing} patch={patch} detected={beats} status={beatStatus} onDetect={onDetectBeats} />}
      <button className="btn danger small" onClick={() => dispatch({ type: "setMusic", music: null })}>Remove music</button>
    </div>
  );
};

/**
 * Number input that commits once, on Enter or blur (Escape reverts), for values whose
 * in-between states would change other data (a scene's length clamps its keyframes and layers).
 * The spinner arrows commit right away.
 */
export const CommitNumField = ({
  label, suffix, hint, value, step, min, validate, onCommit, ...rest
}: {
  label: string; suffix?: string; hint?: string; value: number; step: number; min?: number;
  validate: (v: number) => string | null; onCommit: (v: number) => void; "data-scene-duration"?: boolean;
}) => {
  const [text, setText] = useState(String(value));
  const [error, setError] = useState<string | null>(null);
  useEffect(() => {
    setText(String(value));
    setError(null);
  }, [value]);
  const parse = (t: string) => (t.trim() === "" ? NaN : Number(t));
  const commit = () => {
    const v = parse(text);
    const err = Number.isFinite(v) ? validate(v) : "Enter a number.";
    if (err) {
      setText(String(value));
      setError(null);
      return;
    }
    setError(null);
    if (v !== value) onCommit(v);
  };
  return (
    <Field label={label} suffix={suffix} hint={hint}>
      <input
        type="number"
        step={step}
        min={min}
        value={text}
        aria-invalid={error ? true : undefined}
        data-scene-duration={rest["data-scene-duration"] || undefined}
        onChange={(e) => {
          setText(e.target.value);
          const v = parse(e.target.value);
          setError(Number.isFinite(v) ? validate(v) : "Enter a number.");
          // A spinner click (or arrow key) is a whole value, not a keystroke on the way.
          const native = e.nativeEvent as InputEvent;
          if (native.inputType === undefined || native.inputType === "insertReplacementText") {
            if (Number.isFinite(v) && !validate(v) && v !== value) onCommit(v);
          }
        }}
        onKeyDown={(e) => {
          if (e.key === "Enter") commit();
          if (e.key === "Escape") {
            setText(String(value));
            setError(null);
          }
        }}
        onBlur={commit}
      />
      {error && <span className="field-error">{error}</span>}
    </Field>
  );
};

/** Number input that only commits values passing `validate`; otherwise it shows the reason. */
export const NumField = ({
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
