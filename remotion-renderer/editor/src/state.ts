import type { Keyframe, Layer, LayerKind, Music, Scene, Transition, SubtitleCue, VideoTimeline } from "../../src/types/timeline";
import { emptyScene, type Selection } from "./model";
import { clampLayers, moveInStack, newLayer } from "./layers";
import { patchCue } from "../../src/subtitles/words";

export type EditorState = {
  timeline: VideoTimeline;
  selection: Selection;
  history: VideoTimeline[];
  future: VideoTimeline[];
};

export type Action =
  | { type: "load"; timeline: VideoTimeline }
  | { type: "select"; selection: Selection }
  | { type: "patchScene"; index: number; patch: Partial<Scene> }
  | { type: "addScene" }
  | { type: "deleteScene"; index: number }
  | { type: "duplicateScene"; index: number }
  | { type: "moveScene"; index: number; delta: -1 | 1 }
  | { type: "setTransition"; index: number; transition: Transition | null }
  | { type: "addKeyframe"; sceneIndex: number; keyframe: Keyframe }
  | { type: "patchKeyframe"; sceneIndex: number; index: number; patch: Partial<Keyframe> }
  | { type: "moveKeyframe"; sceneIndex: number; index: number; atMs: number }
  | { type: "deleteKeyframe"; sceneIndex: number; index: number }
  | { type: "addLayer"; sceneIndex: number; kind: LayerKind }
  // `patch` replaces whole keys; a key set to undefined is removed (back to its default).
  | { type: "patchLayer"; sceneIndex: number; index: number; patch: Partial<Record<string, unknown>> }
  | { type: "deleteLayer"; sceneIndex: number; index: number }
  | { type: "moveLayer"; sceneIndex: number; index: number; delta: -1 | 1 }
  | { type: "patchSubtitle"; index: number; patch: Partial<SubtitleCue> }
  | { type: "addSubtitle"; startMs: number }
  | { type: "deleteSubtitle"; index: number }
  | { type: "setResolution"; width: number; height: number }
  | { type: "setMusic"; music: Music | null }
  | { type: "patchMusic"; patch: Partial<Music> }
  // A whole timeline proposed by the editing assistant and accepted by the user: one undo step.
  | { type: "applyAssistant"; timeline: VideoTimeline }
  | { type: "undo" }
  | { type: "redo" };

// Edits that mutate the timeline push the previous version onto history, so undo works.
const commit = (state: EditorState, timeline: VideoTimeline, selection = state.selection): EditorState => ({
  timeline,
  selection,
  history: [...state.history.slice(-99), state.timeline],
  future: [],
});

// Keeps scene-local motion valid after structural edits (resize, reorder, delete):
// no transition on the first scene, transitions and keyframes stay inside the scene.
const normalizeScenes = (scenes: Scene[]): Scene[] =>
  scenes.map((s, i) => {
    let next = s;
    if (next.transition_in) {
      if (i === 0) {
        const { transition_in: _drop, ...rest } = next;
        next = rest;
      } else if (next.transition_in.duration_ms > next.duration_ms) {
        next = { ...next, transition_in: { ...next.transition_in, duration_ms: next.duration_ms } };
      }
    }
    if (next.keyframes?.some((k) => k.at_ms > next.duration_ms)) {
      const kept: Keyframe[] = [];
      for (const k of next.keyframes) {
        const at = Math.min(k.at_ms, next.duration_ms);
        if (kept.length && kept[kept.length - 1].at_ms >= at) continue; // would duplicate: drop
        kept.push({ ...k, at_ms: at });
      }
      next = { ...next, keyframes: kept };
    }
    const layers = clampLayers(next.layers, next.duration_ms);
    if (layers !== next.layers) next = { ...next, layers };
    return next;
  });

const withScene = (t: VideoTimeline, i: number, fn: (s: Scene) => Scene): VideoTimeline => ({
  ...t,
  scenes: t.scenes.map((s, j) => (j === i ? fn(s) : s)),
});

const withLayers = (t: VideoTimeline, sceneIndex: number, layers: Layer[]): VideoTimeline =>
  withScene(t, sceneIndex, (s) => {
    if (layers.length > 0) return { ...s, layers };
    const { layers: _l, ...rest } = s;
    return rest;
  });

// Removes optional keys whose value is undefined so they are not written to the JSON.
const compactKeyframe = (k: Keyframe): Keyframe =>
  Object.fromEntries(Object.entries(k).filter(([, v]) => v !== undefined)) as Keyframe;

export const reducer = (state: EditorState, action: Action): EditorState => {
  const t = state.timeline;
  switch (action.type) {
    case "load":
      return { timeline: action.timeline, selection: { kind: "none" }, history: [], future: [] };
    case "select":
      return { ...state, selection: action.selection };
    case "patchScene": {
      const scenes = t.scenes.map((s, i) => (i === action.index ? { ...s, ...action.patch } : s));
      return commit(state, { ...t, scenes: normalizeScenes(scenes) });
    }
    case "setTransition": {
      if (action.index === 0 && action.transition) return state;
      return commit(
        state,
        withScene(t, action.index, (s) => {
          const { transition_in: _old, ...rest } = s;
          if (!action.transition) return rest;
          const ms = Math.max(1, Math.min(Math.round(action.transition.duration_ms), s.duration_ms));
          return { ...rest, transition_in: { kind: action.transition.kind, duration_ms: ms } };
        }),
      );
    }
    case "addKeyframe": {
      const scene = t.scenes[action.sceneIndex];
      if (!scene) return state;
      const existing = scene.keyframes ?? [];
      const at = Math.max(0, Math.min(Math.round(action.keyframe.at_ms), scene.duration_ms));
      if (existing.some((k) => k.at_ms === at)) return state;
      const list = [...existing, compactKeyframe({ ...action.keyframe, at_ms: at })].sort((a, b) => a.at_ms - b.at_ms);
      const index = list.findIndex((k) => k.at_ms === at);
      return commit(state, withScene(t, action.sceneIndex, (s) => ({ ...s, keyframes: list })), {
        kind: "keyframe", sceneIndex: action.sceneIndex, index,
      });
    }
    case "patchKeyframe": {
      const scene = t.scenes[action.sceneIndex];
      const kf = scene?.keyframes?.[action.index];
      if (!scene || !kf) return state;
      const list = scene.keyframes!.map((k, i) => (i === action.index ? compactKeyframe({ ...k, ...action.patch, at_ms: k.at_ms }) : k));
      return commit(state, withScene(t, action.sceneIndex, (s) => ({ ...s, keyframes: list })));
    }
    case "moveKeyframe": {
      const scene = t.scenes[action.sceneIndex];
      const list = scene?.keyframes;
      if (!scene || !list?.[action.index]) return state;
      // Stay strictly between the neighbours (order kept, no duplicates) and inside the scene.
      const lo = action.index > 0 ? list[action.index - 1].at_ms + 1 : 0;
      const hi = action.index < list.length - 1 ? list[action.index + 1].at_ms - 1 : scene.duration_ms;
      if (lo > hi) return state;
      const at = Math.max(lo, Math.min(Math.round(action.atMs), hi));
      if (at === list[action.index].at_ms) return state;
      const next = list.map((k, i) => (i === action.index ? { ...k, at_ms: at } : k));
      return commit(state, withScene(t, action.sceneIndex, (s) => ({ ...s, keyframes: next })));
    }
    case "deleteKeyframe": {
      const scene = t.scenes[action.sceneIndex];
      if (!scene?.keyframes) return state;
      const next = scene.keyframes.filter((_, i) => i !== action.index);
      return commit(
        state,
        withScene(t, action.sceneIndex, (s) => {
          if (next.length > 0) return { ...s, keyframes: next };
          const { keyframes: _k, ...rest } = s;
          return rest;
        }),
        { kind: "scene", index: action.sceneIndex },
      );
    }
    case "addLayer": {
      const scene = t.scenes[action.sceneIndex];
      if (!scene) return state;
      const layers = [...(scene.layers ?? []), newLayer(action.kind, scene, t.resolution)];
      return commit(state, withLayers(t, action.sceneIndex, layers), {
        kind: "layer", sceneIndex: action.sceneIndex, index: layers.length - 1,
      });
    }
    case "patchLayer": {
      const layer = t.scenes[action.sceneIndex]?.layers?.[action.index];
      if (!layer) return state;
      const merged = Object.fromEntries(
        Object.entries({ ...layer, ...action.patch }).filter(([, v]) => v !== undefined),
      ) as Layer;
      const layers = t.scenes[action.sceneIndex].layers!.map((l, i) => (i === action.index ? merged : l));
      return commit(state, withLayers(t, action.sceneIndex, layers));
    }
    case "deleteLayer": {
      const list = t.scenes[action.sceneIndex]?.layers;
      if (!list?.[action.index]) return state;
      return commit(state, withLayers(t, action.sceneIndex, list.filter((_, i) => i !== action.index)), {
        kind: "scene", index: action.sceneIndex,
      });
    }
    case "moveLayer": {
      const list = t.scenes[action.sceneIndex]?.layers;
      if (!list?.[action.index]) return state;
      const moved = list[action.index];
      const next = moveInStack(list, action.index, action.delta);
      if (next === list) return state;
      const index = next.findIndex((l) => l.id === moved.id);
      return commit(state, withLayers(t, action.sceneIndex, next), { kind: "layer", sceneIndex: action.sceneIndex, index });
    }
    case "addScene": {
      const scene = emptyScene(t.scenes.length);
      return commit(state, { ...t, scenes: [...t.scenes, scene] }, { kind: "scene", index: t.scenes.length });
    }
    case "deleteScene": {
      const scenes = t.scenes.filter((_, i) => i !== action.index);
      return commit(state, { ...t, scenes: normalizeScenes(scenes) }, { kind: "none" });
    }
    case "duplicateScene": {
      const src = t.scenes[action.index];
      const copy = { ...src, id: `${src.id}-copy`, highlights: [...src.highlights], ...(src.layers ? { layers: [...src.layers] } : {}) };
      const scenes = [...t.scenes.slice(0, action.index + 1), copy, ...t.scenes.slice(action.index + 1)];
      return commit(state, { ...t, scenes }, { kind: "scene", index: action.index + 1 });
    }
    case "moveScene": {
      const target = action.index + action.delta;
      if (target < 0 || target >= t.scenes.length) return state;
      const scenes = [...t.scenes];
      [scenes[action.index], scenes[target]] = [scenes[target], scenes[action.index]];
      return commit(state, { ...t, scenes: normalizeScenes(scenes) }, { kind: "scene", index: target });
    }
    case "patchSubtitle": {
      const subtitles = t.subtitles.map((c, i) => (i === action.index ? patchCue(c, action.patch) : c));
      return commit(state, { ...t, subtitles });
    }
    case "addSubtitle": {
      const cue: SubtitleCue = { start_ms: action.startMs, end_ms: action.startMs + 2000, text: "Subtitle" };
      const subtitles = [...t.subtitles, cue];
      return commit(state, { ...t, subtitles }, { kind: "subtitle", index: subtitles.length - 1 });
    }
    case "deleteSubtitle": {
      const subtitles = t.subtitles.filter((_, i) => i !== action.index);
      return commit(state, { ...t, subtitles }, { kind: "none" });
    }
    case "setResolution":
      return commit(state, { ...t, resolution: { width: action.width, height: action.height } });
    case "setMusic":
      return commit(state, { ...t, music: action.music }, action.music ? { kind: "music" } : { kind: "none" });
    case "patchMusic": {
      if (!t.music) return state;
      return commit(state, { ...t, music: { ...t.music, ...action.patch } });
    }
    case "applyAssistant": {
      const next = { ...action.timeline, subtitles: action.timeline.subtitles ?? [] };
      const keep =
        (state.selection.kind === "scene" && next.scenes[state.selection.index]) ||
        (state.selection.kind === "layer" && next.scenes[state.selection.sceneIndex]?.layers?.[state.selection.index]) ||
        (state.selection.kind === "subtitle" && next.subtitles[state.selection.index]) ||
        (state.selection.kind === "music" && next.music);
      return commit(state, next, keep ? state.selection : { kind: "none" });
    }
    case "undo": {
      const prev = state.history[state.history.length - 1];
      if (!prev) return state;
      return { timeline: prev, selection: { kind: "none" }, history: state.history.slice(0, -1), future: [t, ...state.future] };
    }
    case "redo": {
      const next = state.future[0];
      if (!next) return state;
      return { timeline: next, selection: { kind: "none" }, history: [...state.history, t], future: state.future.slice(1) };
    }
  }
};
