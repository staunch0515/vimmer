import { useMemo, useRef, useState, type Dispatch, type PointerEvent as ReactPointerEvent } from "react";
import type { VideoTimeline } from "../../../src/types/timeline";
import { timecode, type Selection } from "../model";
import type { Action } from "../state";
import type { BeatMark } from "../beats";
import { keyframeRange, layerSpan } from "../layerKeyframes";
import { ResizeHandle, usePersistedSize } from "./layout/Split";
import { clamp, restore } from "./layout/layout";

// The track-label column: resizable, remembered like the studio's dividers.
const LABEL_W = 116;
const LABEL_MIN = 64;
const LABEL_MAX = 280;
const SNAP_PX = 8;
const TRACK_H = 46;
const MIN_DURATION_MS = 100;

type Props = {
  timeline: VideoTimeline;
  selection: Selection;
  frame: number;
  onSeek: (frame: number) => void;
  onSelect: (s: Selection) => void;
  dispatch: Dispatch<Action>;
  // Beats of the music in video time (after "Detect beats", or from a hand-set tempo).
  beatMarks?: BeatMark[];
};

type Drag =
  | { kind: "scrub" }
  | { kind: "keyframe"; sceneIndex: number; index: number; atMs: number; moved: boolean }
  // A keyframe of layer `index`: `keyframe` is its index in the layer's list.
  | { kind: "layerKeyframe"; sceneIndex: number; index: number; keyframe: number; atMs: number; moved: boolean }
  | { kind: "resize"; index: number; sceneStartMs: number; startX: number; startDuration: number; duration: number };

// Magnetic snapping: a point within SNAP_PX of a boundary pulls onto it and shows a guide.
const snapTo = (ms: number, points: number[], pps: number): { ms: number; guide: number | null } => {
  let best: number | null = null;
  for (const p of points) {
    if (Math.abs(p - ms) * (pps / 1000) <= SNAP_PX && (best === null || Math.abs(p - ms) < Math.abs(best - ms))) {
      best = p;
    }
  }
  return best === null ? { ms, guide: null } : { ms: best, guide: best };
};

export const EditorTimeline = ({ timeline, selection, frame, onSeek, onSelect, dispatch, beatMarks = [] }: Props) => {
  const [pps, setPps] = useState(60);
  const [drag, setDrag] = useState<Drag | null>(null);
  const [guide, setGuide] = useState<number | null>(null);
  const rulerRef = useRef<HTMLDivElement>(null);
  const fps = timeline.fps;
  const [labelStored, saveLabel] = usePersistedSize("timeline-labels", LABEL_W);
  const [labelLive, setLabelLive] = useState<number | null>(null);
  const labelLiveRef = useRef<number | null>(null);
  const setLabel = (w: number | null) => {
    labelLiveRef.current = w;
    setLabelLive(w);
  };
  const labelStart = useRef(LABEL_W);
  const labelW = labelLive ?? clamp(Math.round(labelStored.size), LABEL_MIN, LABEL_MAX);
  const keepLabel = (w: number) => saveLabel({ size: clamp(Math.round(w), LABEL_MIN, LABEL_MAX), basis: null, collapsed: false });

  const starts = useMemo(() => {
    let acc = 0;
    return timeline.scenes.map((s) => {
      const start = acc;
      acc += s.duration_ms;
      return start;
    });
  }, [timeline.scenes]);

  const totalMs = timeline.scenes.reduce((sum, s) => sum + s.duration_ms, 0);
  const contentW = Math.max(totalMs, 30_000) * (pps / 1000) + 240;
  const snapPoints = useMemo(() => {
    const pts = new Set<number>([0, totalMs]);
    starts.forEach((s, i) => pts.add(s + timeline.scenes[i].duration_ms));
    timeline.subtitles.forEach((c) => {
      pts.add(c.start_ms);
      pts.add(c.end_ms);
    });
    // Beats pull scene edges too, so a cut can be dragged onto one.
    beatMarks.forEach((b) => pts.add(Math.round(b.ms)));
    return [...pts];
  }, [starts, timeline.scenes, timeline.subtitles, totalMs, beatMarks]);

  const msFromClientX = (clientX: number): number => {
    const rect = rulerRef.current!.getBoundingClientRect();
    return Math.max(0, (clientX - rect.left) / (pps / 1000));
  };

  // Ruler scrubbing: pointer capture keeps the drag alive outside the element.
  const onRulerDown = (e: ReactPointerEvent<HTMLDivElement>) => {
    e.currentTarget.setPointerCapture(e.pointerId);
    setDrag({ kind: "scrub" });
    scrubTo(e.clientX);
  };
  const scrubTo = (clientX: number) => {
    const snapped = snapTo(msFromClientX(clientX), snapPoints, pps);
    setGuide(snapped.guide);
    onSeek(Math.round((snapped.ms * fps) / 1000));
  };

  const onPointerMove = (e: ReactPointerEvent<HTMLDivElement>) => {
    if (!drag) return;
    if (drag.kind === "scrub") {
      scrubTo(e.clientX);
      return;
    }
    if (drag.kind === "keyframe") {
      const scene = timeline.scenes[drag.sceneIndex];
      const list = scene.keyframes ?? [];
      const lo = drag.index > 0 ? list[drag.index - 1].at_ms + 1 : 0;
      const hi = drag.index < list.length - 1 ? list[drag.index + 1].at_ms - 1 : scene.duration_ms;
      const raw = Math.round(msFromClientX(e.clientX) - starts[drag.sceneIndex]);
      setDrag({ ...drag, atMs: Math.max(lo, Math.min(raw, hi)), moved: true });
      return;
    }
    if (drag.kind === "layerKeyframe") {
      const scene = timeline.scenes[drag.sceneIndex];
      const layer = scene?.layers?.[drag.index];
      if (!layer) return;
      // Clamped like the reducer: between its neighbours, inside the layer's time.
      const { lo, hi } = keyframeRange(layer, scene.duration_ms, drag.keyframe);
      const raw = Math.round(msFromClientX(e.clientX) - starts[drag.sceneIndex]);
      setDrag({ ...drag, atMs: Math.max(lo, Math.min(raw, hi)), moved: true });
      return;
    }
    const dxMs = ((e.clientX - drag.startX) / pps) * 1000;
    const wanted = drag.sceneStartMs + drag.startDuration + dxMs;
    const snapped = snapTo(wanted, snapPoints, pps);
    const duration = Math.max(MIN_DURATION_MS, Math.round((snapped.ms - drag.sceneStartMs) / 10) * 10);
    setGuide(snapped.guide);
    setDrag({ ...drag, duration });
  };

  const onPointerUp = () => {
    if (drag?.kind === "resize" && drag.duration !== drag.startDuration) {
      dispatch({ type: "patchScene", index: drag.index, patch: { duration_ms: drag.duration } });
    }
    if (drag?.kind === "layerKeyframe" && drag.moved) {
      // One dispatch when the drag ends: a drag is one undo step.
      dispatch({ type: "moveLayerKeyframe", sceneIndex: drag.sceneIndex, index: drag.index, keyframe: drag.keyframe, atMs: drag.atMs });
    }
    if (drag?.kind === "keyframe" && drag.moved) {
      dispatch({ type: "moveKeyframe", sceneIndex: drag.sceneIndex, index: drag.index, atMs: drag.atMs });
    }
    setDrag(null);
    setGuide(null);
  };

  const ticks = Array.from({ length: Math.ceil(contentW / pps) + 1 }, (_, s) => s);
  const playheadPx = (frame / fps) * pps;
  const sceneDuration = (i: number) =>
    drag?.kind === "resize" && drag.index === i ? drag.duration : timeline.scenes[i].duration_ms;

  return (
    <section className="timeline panel" aria-label="Timeline">
      <div className="timeline-head">
        <span className="section-label">Timeline</span>
        <span className="dim mono">{timecode(totalMs)} total</span>
        <span className="spacer" />
        <label className="zoom">
          <span className="dim">Zoom</span>
          <input type="range" min={20} max={160} step={4} value={pps} onChange={(e) => setPps(Number(e.target.value))} />
        </label>
      </div>
      <div className="timeline-scroll" onPointerMove={onPointerMove} onPointerUp={onPointerUp} onPointerCancel={onPointerUp}>
        <div className="timeline-canvas" style={{ width: contentW + labelW, ["--track-label-w" as string]: `${labelW}px` }}>
          <div className="ruler-row">
            <div className="track-label ruler-label" />
            <div className="ruler" ref={rulerRef} style={{ width: contentW }} onPointerDown={onRulerDown}>
              {ticks.map((s) => (
                <div key={s} className={s % 5 === 0 ? "tick major" : "tick"} style={{ left: s * pps }}>
                  {s % 5 === 0 && <span className="tick-label mono">{timecode(s * 1000).slice(0, 5)}</span>}
                </div>
              ))}
              {beatMarks.map((b) => (
                <div
                  key={b.ms}
                  className={b.bar ? "beat-mark bar" : "beat-mark"}
                  style={{ left: (b.ms / 1000) * pps }}
                  title={`beat ${timecode(b.ms)}`}
                  data-beat-mark
                />
              ))}
            </div>
          </div>

          <Track label="Scenes" height={TRACK_H + 6}>
            {timeline.scenes.map((scene, i) => {
              const left = (starts[i] / 1000) * pps;
              const width = Math.max(4, (sceneDuration(i) / 1000) * pps - 2);
              const active = selection.kind === "scene" && selection.index === i;
              return (
                <div
                  key={`${scene.id}-${i}`}
                  className={active ? "block scene active" : "block scene"}
                  style={{ left, width }}
                  onPointerDown={() => onSelect({ kind: "scene", index: i })}
                  title={`${scene.id} · ${scene.duration_ms} ms`}
                >
                  <span className="block-title">{scene.id}</span>
                  <span className="block-sub mono">{(sceneDuration(i) / 1000).toFixed(2)} s</span>
                  <div
                    className="resize-handle"
                    onPointerDown={(e) => {
                      e.stopPropagation();
                      e.currentTarget.setPointerCapture(e.pointerId);
                      setDrag({
                        kind: "resize",
                        index: i,
                        sceneStartMs: starts[i],
                        startX: e.clientX,
                        startDuration: scene.duration_ms,
                        duration: scene.duration_ms,
                      });
                    }}
                    aria-label={`Resize ${scene.id}`}
                  />
                </div>
              );
            })}
          </Track>

          <Track label="Keyframes" height={30}>
            {timeline.scenes.map((scene, si) =>
              (scene.keyframes ?? []).map((k, ki) => {
                const dragging = drag?.kind === "keyframe" && drag.sceneIndex === si && drag.index === ki;
                const at = dragging ? drag.atMs : k.at_ms;
                const active = selection.kind === "keyframe" && selection.sceneIndex === si && selection.index === ki;
                return (
                  <div
                    key={`kf-${si}-${ki}`}
                    role="button"
                    aria-label={`Keyframe ${ki + 1} of ${scene.id} at ${at} ms`}
                    className={active ? "kf-marker active" : "kf-marker"}
                    style={{ left: ((starts[si] + at) / 1000) * pps }}
                    title={`${scene.id} · ${at} ms`}
                    onPointerDown={(e) => {
                      e.stopPropagation();
                      e.currentTarget.setPointerCapture(e.pointerId);
                      onSelect({ kind: "keyframe", sceneIndex: si, index: ki });
                      setDrag({ kind: "keyframe", sceneIndex: si, index: ki, atMs: k.at_ms, moved: false });
                    }}
                    onKeyDown={(e) => {
                      if (e.key === "Delete" || e.key === "Backspace") dispatch({ type: "deleteKeyframe", sceneIndex: si, index: ki });
                    }}
                    tabIndex={0}
                  />
                );
              }),
            )}
          </Track>

          <LayerTrack
            timeline={timeline}
            selection={selection}
            starts={starts}
            pps={pps}
            drag={drag}
            onSelect={onSelect}
            dispatch={dispatch}
            onStartDrag={setDrag}
          />

          <Track label="Subtitles" height={TRACK_H}>
            {timeline.subtitles.map((c, i) => {
              const active = selection.kind === "subtitle" && selection.index === i;
              return (
                <div
                  key={i}
                  className={active ? "block cue active" : "block cue"}
                  style={{ left: (c.start_ms / 1000) * pps, width: Math.max(4, ((c.end_ms - c.start_ms) / 1000) * pps) }}
                  onPointerDown={() => onSelect({ kind: "subtitle", index: i })}
                  title={c.text}
                >
                  <span className="block-title">{c.text}</span>
                </div>
              );
            })}
          </Track>

          <Track label="Music" height={TRACK_H}>
            {timeline.music && (
              <div
                className={selection.kind === "music" ? "block music active" : "block music"}
                style={{ left: 0, width: Math.max(4, (totalMs / 1000) * pps - 2) }}
                onPointerDown={() => onSelect({ kind: "music" })}
                title={timeline.music.source ?? `music by mood: ${timeline.music.pick?.mood ?? ""}`}
              >
                <span className="block-title">♪ {timeline.music.source || (timeline.music.pick ? `mood: ${timeline.music.pick.mood}` : "music")}</span>
              </div>
            )}
          </Track>

          <Track label="Audio" height={TRACK_H}>
            {timeline.scenes.map((scene, i) =>
              scene.audio ? (
                <div
                  key={`audio-${i}`}
                  className="block audio"
                  style={{
                    left: ((starts[i] + scene.audio.offset_ms) / 1000) * pps,
                    width: Math.max(4, ((scene.duration_ms - scene.audio.offset_ms) / 1000) * pps - 2),
                  }}
                  onPointerDown={() => onSelect({ kind: "scene", index: i })}
                  title={scene.audio.source}
                >
                  <span className="block-title">{scene.audio.source || "audio"}</span>
                </div>
              ) : null,
            )}
          </Track>

          <div className="playhead" style={{ left: labelW + playheadPx }} aria-hidden>
            <span className="playhead-cap" />
          </div>
          {guide !== null && <div className="snap-guide" style={{ left: labelW + (guide / 1000) * pps }} aria-hidden />}
        </div>
      </div>
      {/* Outside the scroll area: the label column is sticky, so the line stays put. */}
        <ResizeHandle
        className="label-handle"
        style={{ left: labelW - 1 }}
        axis="x"
        sign={1}
        value={labelW}
        min={LABEL_MIN}
        max={LABEL_MAX}
        label="Resize track labels"
        onDragStart={() => (labelStart.current = labelW)}
        onDrag={(d) => setLabel(clamp(Math.round(labelStart.current + d), LABEL_MIN, LABEL_MAX))}
        onDragEnd={() => {
          if (labelLiveRef.current !== null) keepLabel(labelLiveRef.current);
          setLabel(null);
        }}
        onKey={(r) => {
          if (r.kind === "delta") keepLabel(labelW + r.delta);
          else if (r.kind === "min") keepLabel(LABEL_MIN);
          else if (r.kind === "max") keepLabel(LABEL_MAX);
        }}
        onReset={() => {
          setLabel(null);
          saveLabel(restore(LABEL_W));
        }}
      />
    </section>
  );
};

// The selected layer's time in its scene, with its keyframes as draggable markers.
const LayerTrack = ({
  timeline, selection, starts, pps, drag, onSelect, dispatch, onStartDrag,
}: {
  timeline: VideoTimeline; selection: Selection; starts: number[]; pps: number; drag: Drag | null;
  onSelect: (s: Selection) => void; dispatch: Dispatch<Action>; onStartDrag: (d: Drag) => void;
}) => {
  const sel = selection.kind === "layer" ? selection : null;
  const scene = sel ? timeline.scenes[sel.sceneIndex] : undefined;
  const layer = sel ? scene?.layers?.[sel.index] : undefined;
  if (!sel || !scene || !layer) {
    return (
      <Track label="Layer" height={30}>
        <span className="track-hint dim">Select a layer to see its keyframes</span>
      </Track>
    );
  }
  const { sceneIndex, index } = sel;
  const span = layerSpan(layer, scene.duration_ms);
  const base = starts[sceneIndex];
  return (
    <Track label={layer.id.length > 12 ? `${layer.id.slice(0, 11)}…` : layer.id} height={30} title={`Layer “${layer.id}” in ${scene.id}`}>
      <div
        className="layer-span"
        style={{ left: ((base + span.start) / 1000) * pps, width: Math.max(2, ((span.end - span.start) / 1000) * pps) }}
        data-layer-span={layer.id}
        onPointerDown={() => onSelect({ kind: "layer", sceneIndex, index })}
      />
      {(layer.keyframes ?? []).map((k, ki) => {
        const dragging = drag?.kind === "layerKeyframe" && drag.sceneIndex === sceneIndex && drag.index === index && drag.keyframe === ki;
        const at = dragging ? drag.atMs : k.at_ms;
        const active = sel.keyframe === ki;
        return (
          <div
            key={`lkf-${ki}`}
            role="button"
            aria-label={`Keyframe ${ki + 1} of layer ${layer.id} at ${at} ms`}
            className={active ? "kf-marker layer active" : "kf-marker layer"}
            data-layer-kf-marker={ki}
            style={{ left: ((base + at) / 1000) * pps }}
            title={`${layer.id} · ${at} ms`}
            onPointerDown={(e) => {
              e.stopPropagation();
              e.currentTarget.setPointerCapture(e.pointerId);
              onSelect({ kind: "layer", sceneIndex, index, keyframe: ki });
              onStartDrag({ kind: "layerKeyframe", sceneIndex, index, keyframe: ki, atMs: k.at_ms, moved: false });
            }}
            onKeyDown={(e) => {
              if (e.key === "Delete" || e.key === "Backspace") dispatch({ type: "deleteLayerKeyframe", sceneIndex, index, keyframe: ki });
            }}
            tabIndex={0}
          />
        );
      })}
    </Track>
  );
};

const Track = ({ label, height, title, children }: { label: string; height: number; title?: string; children: React.ReactNode }) => (
  <div className="track-row" style={{ height }}>
    <div className="track-label" title={title ?? label}>{label}</div>
    <div className="track-content">{children}</div>
  </div>
);
