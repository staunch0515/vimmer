import type { Easing, Keyframe, Scene, SubtitleCue, TransitionKind, VideoTimeline } from "../../src/types/timeline";
import { totalFrames } from "../../src/layout";
import { checkClip, checkLayers } from "./layers";
import { templateProblems } from "./templates";

export const MAX_DURATION_MS = 200_000;
export const ALLOWED_RESOLUTIONS = [
  { width: 1920, height: 1080, label: "Landscape 1080p" },
  { width: 1080, height: 1920, label: "Vertical 1080p" },
] as const;

export const emptyScene = (index: number): Scene => ({
  id: `scene-${String(index + 1).padStart(2, "0")}`,
  duration_ms: 5000,
  text: "New scene",
  highlights: [],
  image: null,
  motion: "static",
  audio: null,
  narration: null,
});

export const totalMs = (t: VideoTimeline): number =>
  t.scenes.reduce((sum, s) => sum + s.duration_ms, 0);

// Mirrors the Rust `validate` rules so problems show up before export.
export const checkTimeline = (t: VideoTimeline): string[] => {
  const problems: string[] = [];
  if (t.scenes.length === 0) problems.push("Timeline has no scenes.");
  if (t.fps !== 30) problems.push(`FPS is ${t.fps}; the renderer targets 30.`);
  if (!ALLOWED_RESOLUTIONS.some((r) => r.width === t.resolution.width && r.height === t.resolution.height)) {
    problems.push(`Resolution ${t.resolution.width}x${t.resolution.height} is not supported.`);
  }
  const ids = new Set<string>();
  for (const s of t.scenes) {
    if (ids.has(s.id)) problems.push(`Duplicate scene id “${s.id}”.`);
    ids.add(s.id);
    if (s.duration_ms <= 0) problems.push(`Scene “${s.id}” has zero duration.`);
    if (s.template) problems.push(...templateProblems(s.template).map((p) => `Scene “${s.id}” template: ${p}`));
    if (!s.template && !s.text?.trim() && !s.image && !s.video && !s.layers?.length) problems.push(`Scene “${s.id}” needs text, an image, a video or a layer.`);
    if (s.image && s.video) problems.push(`Scene “${s.id}” has both an image and a video; choose one background.`);
    if (s.video) problems.push(...checkClip(`Scene “${s.id}” background`, s.video));
    problems.push(...checkSceneMotion(s, t.scenes.indexOf(s)));
    problems.push(...checkLayers(s));
    problems.push(...checkSfx(s));
  }
  const total = totalMs(t);
  if (total > MAX_DURATION_MS) problems.push(`Timeline is ${(total / 1000).toFixed(1)} s; the limit is 200 s.`);
  if (t.music) {
    const hasFile = !!t.music.source?.trim();
    const pick = t.music.pick;
    if (!hasFile && !pick) problems.push("Background music has no file or mood.");
    if (t.music.source !== undefined && pick) problems.push("Background music has both a file and a mood; keep one.");
    if (pick && !pick.mood.trim()) problems.push("Choose a mood for the music.");
    if (pick?.energy !== undefined && !(pick.energy >= 0 && pick.energy <= 1)) problems.push("Music energy must be between 0 and 1.");
    if (t.music.volume < 0 || t.music.volume > 1) problems.push("Music volume must be between 0 and 1.");
    if (t.music.fade_in_ms + t.music.fade_out_ms > total) problems.push("Music fades are longer than the video.");
    const b = t.music.beats;
    if (b && !(b.bpm >= 30 && b.bpm <= 300)) problems.push("Music tempo must be between 30 and 300 BPM.");
    const sync = t.music.beat_sync;
    if (sync && sync.max_shift_ms != null && !(sync.max_shift_ms >= 1 && sync.max_shift_ms <= 1000)) {
      problems.push("Beat sync max shift must be between 1 and 1000 ms.");
    }
  }
  t.subtitles?.forEach((c, i) => {
    if (c.start_ms >= c.end_ms) problems.push(`Subtitle ${i + 1} must start before it ends.`);
    else if (c.end_ms > total) problems.push(`Subtitle ${i + 1} runs past the end of the timeline.`);
    if (!c.text.trim()) problems.push(`Subtitle ${i + 1} is empty.`);
  });
  return problems;
};

export const frameCount = (t: VideoTimeline): number => Math.max(1, totalFrames(t));

export const timecode = (ms: number): string => {
  const safe = Math.max(0, ms);
  const m = Math.floor(safe / 60_000);
  const s = Math.floor((safe % 60_000) / 1000);
  const frac = Math.floor((safe % 1000) / 33.33);
  return `${String(m).padStart(2, "0")}:${String(s).padStart(2, "0")}:${String(frac).padStart(2, "0")}`;
};

export const frameToMs = (frame: number, fps: number): number => Math.round((frame * 1000) / fps);

export type Selection =
  | { kind: "scene"; index: number }
  | { kind: "subtitle"; index: number }
  | { kind: "keyframe"; sceneIndex: number; index: number }
  | { kind: "layer"; sceneIndex: number; index: number }
  | { kind: "music" }
  | { kind: "none" };

export const sortCues = (cues: SubtitleCue[]): SubtitleCue[] =>
  [...cues].sort((a, b) => a.start_ms - b.start_ms);

export const TRANSITION_KINDS: TransitionKind[] = [
  "fade", "slide_left", "slide_right", "slide_up", "slide_down", "zoom_in", "wipe_left",
];
export const EASINGS: Easing[] = ["linear", "ease_in", "ease_out", "ease_in_out", "back_out", "elastic_out", "bounce_out"];
export const DEFAULT_TRANSITION_MS = 500;

/** Mirrors vimmer-dsl validate_transition / validate_keyframes. */
export const checkSceneMotion = (s: Scene, index: number): string[] => {
  const out: string[] = [];
  const tr = s.transition_in;
  if (tr) {
    if (index === 0) out.push(`Scene “${s.id}” is the first scene, so it cannot have a transition.`);
    if (!(tr.duration_ms > 0)) out.push(`Scene “${s.id}” has a zero-length transition.`);
    else if (tr.duration_ms > s.duration_ms) {
      out.push(`Scene “${s.id}” transition (${tr.duration_ms} ms) is longer than the scene (${s.duration_ms} ms).`);
    }
  }
  (s.keyframes ?? []).forEach((k, i, all) => {
    if (k.at_ms < 0 || k.at_ms > s.duration_ms) out.push(`Scene “${s.id}” keyframe ${i + 1} is outside the scene.`);
    if (k.scale !== undefined && !(Number.isFinite(k.scale) && k.scale > 0)) out.push(`Scene “${s.id}” keyframe ${i + 1}: scale must be above 0.`);
    if (k.opacity !== undefined && !(k.opacity >= 0 && k.opacity <= 1)) out.push(`Scene “${s.id}” keyframe ${i + 1}: opacity must be between 0 and 1.`);
    if (i > 0 && k.at_ms <= all[i - 1].at_ms) out.push(`Scene “${s.id}” keyframes must be in increasing time order without duplicates.`);
  });
  return out;
};

/** Returns an error message for a keyframe field value, or null when it is acceptable. */
export const keyframeFieldError = (field: "scale" | "x" | "y" | "opacity" | "rotation", v: number): string | null => {
  if (!Number.isFinite(v)) return "Enter a number.";
  if (field === "scale" && v <= 0) return "Scale must be above 0.";
  if (field === "opacity" && (v < 0 || v > 1)) return "Opacity must be between 0 and 1.";
  return null;
};

export const sortedKeyframes = (s: Scene): Keyframe[] => s.keyframes ?? [];

// Mirrors `sfx_problems` in rust-core/crates/vimmer-dsl/src/validation.rs.
const checkSfx = (s: Scene): string[] => {
  const out: string[] = [];
  const list = s.sfx ?? [];
  if (list.length > 16) out.push(`Scene “${s.id}” has ${list.length} sound effects; the limit is 16.`);
  list.forEach((fx, k) => {
    const name = `Scene “${s.id}” sound ${k + 1}`;
    if (!fx.src.trim()) out.push(`${name} has no file.`);
    else if (!/\.(wav|mp3|m4a|aac|ogg|flac|opus)$/i.test(fx.src)) out.push(`${name} must be an audio file.`);
    if ((fx.at_ms != null) === !!fx.on) out.push(`${name} needs either a time or a layer event.`);
    if (fx.at_ms != null && fx.at_ms >= s.duration_ms) out.push(`${name} is after the scene ends.`);
    if (fx.on && !(s.layers ?? []).some((l) => l.id === fx.on!.layer)) out.push(`${name} plays on layer “${fx.on.layer}”, which does not exist.`);
    if (fx.volume != null && !(fx.volume >= 0 && fx.volume <= 1)) out.push(`${name} volume must be between 0 and 1.`);
  });
  return out;
};
