import type { Music, Scene, SubtitleCue, VideoTimeline } from "../../src/types/timeline";
import { emptyScene, type Selection } from "./model";

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
  | { type: "patchSubtitle"; index: number; patch: Partial<SubtitleCue> }
  | { type: "addSubtitle"; startMs: number }
  | { type: "deleteSubtitle"; index: number }
  | { type: "setResolution"; width: number; height: number }
  | { type: "setMusic"; music: Music | null }
  | { type: "patchMusic"; patch: Partial<Music> }
  | { type: "undo" }
  | { type: "redo" };

// Edits that mutate the timeline push the previous version onto history, so undo works.
const commit = (state: EditorState, timeline: VideoTimeline, selection = state.selection): EditorState => ({
  timeline,
  selection,
  history: [...state.history.slice(-99), state.timeline],
  future: [],
});

export const reducer = (state: EditorState, action: Action): EditorState => {
  const t = state.timeline;
  switch (action.type) {
    case "load":
      return { timeline: action.timeline, selection: { kind: "none" }, history: [], future: [] };
    case "select":
      return { ...state, selection: action.selection };
    case "patchScene": {
      const scenes = t.scenes.map((s, i) => (i === action.index ? { ...s, ...action.patch } : s));
      return commit(state, { ...t, scenes });
    }
    case "addScene": {
      const scene = emptyScene(t.scenes.length);
      return commit(state, { ...t, scenes: [...t.scenes, scene] }, { kind: "scene", index: t.scenes.length });
    }
    case "deleteScene": {
      const scenes = t.scenes.filter((_, i) => i !== action.index);
      return commit(state, { ...t, scenes }, { kind: "none" });
    }
    case "duplicateScene": {
      const src = t.scenes[action.index];
      const copy = { ...src, id: `${src.id}-copy`, highlights: [...src.highlights] };
      const scenes = [...t.scenes.slice(0, action.index + 1), copy, ...t.scenes.slice(action.index + 1)];
      return commit(state, { ...t, scenes }, { kind: "scene", index: action.index + 1 });
    }
    case "moveScene": {
      const target = action.index + action.delta;
      if (target < 0 || target >= t.scenes.length) return state;
      const scenes = [...t.scenes];
      [scenes[action.index], scenes[target]] = [scenes[target], scenes[action.index]];
      return commit(state, { ...t, scenes }, { kind: "scene", index: target });
    }
    case "patchSubtitle": {
      const subtitles = t.subtitles.map((c, i) => (i === action.index ? { ...c, ...action.patch } : c));
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
