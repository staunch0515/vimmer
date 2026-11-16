// The voice-over track (旁白音轨) in the editor: adding, moving, trimming and deleting clips, and
// moving a scene's own audio onto the track. Pure (type imports only), so it is unit-tested with
// Node (test/editor-voiceover.test.ts). Mirrors vimmer-dsl/src/voiceover.rs.
//
// Clip lengths come from the files: `lengths` maps a timeline path to its length in ms, as far as
// the editor has measured it. A clip whose file is not measured yet is treated as
// `UNKNOWN_LENGTH_MS` long for placing (so a new clip never lands on top of another one).

import type { Scene, VideoTimeline, VoiceoverClip } from "../../src/types/timeline";

export type Lengths = Record<string, number>;

export const MAX_VOICEOVER_GAIN = 4;
export const MAX_VOICEOVER_CLIPS = 200;
export const UNKNOWN_LENGTH_MS = 2000;
/** Shortest a clip may be trimmed to. */
export const MIN_CLIP_MS = 100;

const total = (t: VideoTimeline): number => t.scenes.reduce((n, s) => n + s.duration_ms, 0);

export const clipsOf = (t: VideoTimeline): VoiceoverClip[] => t.voiceover ?? [];

export const fileMs = (c: VoiceoverClip, lengths: Lengths): number => lengths[c.source] ?? UNKNOWN_LENGTH_MS;

/** How long the clip plays (its trims taken off its file). */
export const playedMs = (c: VoiceoverClip, lengths: Lengths): number =>
  Math.max(0, fileMs(c, lengths) - (c.trim_start_ms ?? 0) - (c.trim_end_ms ?? 0));

export const endMs = (c: VoiceoverClip, lengths: Lengths): number => c.start_ms + playedMs(c, lengths);

/** Drops default values so the JSON stays small: trims of 0 and a gain of 1 are not written. */
export const compactClip = (c: VoiceoverClip): VoiceoverClip => {
  const out: VoiceoverClip = { id: c.id, source: c.source, start_ms: Math.max(0, Math.round(c.start_ms)) };
  if (c.trim_start_ms) out.trim_start_ms = Math.max(0, Math.round(c.trim_start_ms));
  if (c.trim_end_ms) out.trim_end_ms = Math.max(0, Math.round(c.trim_end_ms));
  if (c.gain !== undefined && c.gain !== 1) out.gain = Math.round(c.gain * 100) / 100;
  return out;
};

/** The timeline with `clips` as its track, in time order; an empty track removes the key. */
export const withClips = (t: VideoTimeline, clips: VoiceoverClip[]): VideoTimeline => {
  const { voiceover: _old, ...rest } = t;
  if (clips.length === 0) return rest;
  return { ...rest, voiceover: [...clips].map(compactClip).sort((a, b) => a.start_ms - b.start_ms) };
};

/** A new unique clip id: `vo-1`, `vo-2`, … */
export const newClipId = (t: VideoTimeline, prefix = "vo"): string => {
  const ids = new Set(clipsOf(t).map((c) => c.id));
  for (let n = 1; ; n++) if (!ids.has(`${prefix}-${n}`)) return `${prefix}-${n}`;
};

/** The free span around `at` that `except` (an index) may occupy: [lo, hi). */
const freeSpan = (clips: VoiceoverClip[], lengths: Lengths, except: number, at: number): { lo: number; hi: number } => {
  let lo = 0;
  let hi = Number.POSITIVE_INFINITY;
  clips.forEach((c, i) => {
    if (i === except) return;
    const end = endMs(c, lengths);
    if (end <= at) lo = Math.max(lo, end);
    else if (c.start_ms >= at) hi = Math.min(hi, c.start_ms);
    else lo = Math.max(lo, end); // `at` is inside this clip: start after it
  });
  return { lo, hi };
};

/**
 * Where a clip `played` ms long may start: at `wanted` (inside the video) when that is free, else
 * right after the clips in the way. Never on another clip. Null when that is past the video's end.
 */
export const placeStart = (t: VideoTimeline, lengths: Lengths, except: number, wanted: number, played: number): number | null => {
  const others = clipsOf(t)
    .filter((_, i) => i !== except)
    .sort((a, b) => a.start_ms - b.start_ms);
  const end = total(t);
  let s = Math.max(0, Math.min(Math.round(wanted), Math.max(0, end - 1)));
  for (const c of others) {
    const e = endMs(c, lengths);
    if (e <= s) continue;
    if (c.start_ms >= s + played) break;
    s = e;
  }
  return s < end ? s : null;
};

export type TrackEdit = { timeline: VideoTimeline; index: number } | { error: string };

/** Adds a clip of `source` near `atMs` (the playhead), after any clip already playing there. */
export const addClip = (t: VideoTimeline, source: string, atMs: number, lengths: Lengths, id = newClipId(t)): TrackEdit => {
  if (!source.trim()) return { error: "Choose an audio file first." };
  if (clipsOf(t).length >= MAX_VOICEOVER_CLIPS) return { error: `The voice-over track holds at most ${MAX_VOICEOVER_CLIPS} clips.` };
  const played = lengths[source] ?? UNKNOWN_LENGTH_MS;
  const start = placeStart(t, lengths, -1, atMs, played);
  if (start === null) return { error: "There is no room on the voice-over track after the playhead." };
  const next = withClips(t, [...clipsOf(t), { id, source, start_ms: start }]);
  return { timeline: next, index: clipsOf(next).findIndex((c) => c.id === id) };
};

/** Moves clip `index` to start near `startMs`, never onto another clip. */
export const moveClip = (t: VideoTimeline, index: number, startMs: number, lengths: Lengths): VideoTimeline => {
  const clips = clipsOf(t);
  const clip = clips[index];
  if (!clip) return t;
  const played = playedMs(clip, lengths);
  const { lo, hi } = freeSpan(clips, lengths, index, clip.start_ms);
  const end = total(t);
  const start = Math.max(lo, Math.min(Math.round(startMs), hi - played, end - 1));
  if (start === clip.start_ms || start < 0) return t;
  return withClips(t, clips.map((c, i) => (i === index ? { ...c, start_ms: start } : c)));
};

/**
 * Trims clip `index`. Its left handle (`trim_start_ms`) keeps the sound in place: the clip's start
 * moves by the same amount. Trims stay inside the file, leave at least MIN_CLIP_MS and never make
 * the clip run into a neighbour.
 */
export const trimClip = (
  t: VideoTimeline,
  index: number,
  edge: "start" | "end",
  trimMs: number,
  lengths: Lengths,
): VideoTimeline => {
  const clips = clipsOf(t);
  const clip = clips[index];
  if (!clip) return t;
  const file = fileMs(clip, lengths);
  const ts = clip.trim_start_ms ?? 0;
  const te = clip.trim_end_ms ?? 0;
  const { lo, hi } = freeSpan(clips, lengths, index, clip.start_ms);
  let next: VoiceoverClip;
  if (edge === "start") {
    // Lowest trim: the file's start, the previous clip's end, the video start.
    const min = Math.max(0, ts - (clip.start_ms - lo));
    const max = file - te - MIN_CLIP_MS;
    const v = Math.max(min, Math.min(Math.round(trimMs), max));
    next = { ...clip, trim_start_ms: v, start_ms: clip.start_ms + (v - ts) };
  } else {
    const roomAfter = hi === Number.POSITIVE_INFINITY ? Number.POSITIVE_INFINITY : hi - endMs(clip, lengths);
    const min = Math.max(0, te - roomAfter);
    const max = file - ts - MIN_CLIP_MS;
    const v = Math.max(min, Math.min(Math.round(trimMs), max));
    next = { ...clip, trim_end_ms: v };
  }
  if (next.start_ms === clip.start_ms && (next.trim_start_ms ?? 0) === ts && (next.trim_end_ms ?? 0) === te) return t;
  return withClips(t, clips.map((c, i) => (i === index ? next : c)));
};

export const setGain = (t: VideoTimeline, index: number, gain: number): VideoTimeline => {
  const clips = clipsOf(t);
  if (!clips[index] || !Number.isFinite(gain)) return t;
  const g = Math.max(0, Math.min(MAX_VOICEOVER_GAIN, gain));
  return withClips(t, clips.map((c, i) => (i === index ? { ...c, gain: g } : c)));
};

export const deleteClip = (t: VideoTimeline, index: number): VideoTimeline =>
  clipsOf(t)[index] ? withClips(t, clipsOf(t).filter((_, i) => i !== index)) : t;

const sceneStart = (t: VideoTimeline, index: number): number =>
  t.scenes.slice(0, index).reduce((n, s) => n + s.duration_ms, 0);

/**
 * 「移到旁白音轨」: the scene's own audio becomes a voice-over clip at the same absolute time (scene
 * start + offset), so it is no longer cut at the scene's end. The scene keeps no audio; its
 * narration setting is switched off too, or the render would start synthesizing it again.
 */
export const moveSceneAudioToTrack = (t: VideoTimeline, sceneIndex: number, lengths: Lengths): TrackEdit => {
  const scene = t.scenes[sceneIndex];
  if (!scene?.audio?.source.trim()) return { error: "This scene has no audio file to move." };
  const start = sceneStart(t, sceneIndex) + scene.audio.offset_ms;
  const id = newClipId(t);
  const clip: VoiceoverClip = { id, source: scene.audio.source, start_ms: start };
  const clash = clipsOf(t).find((c) => c.start_ms < endMs(clip, lengths) && endMs(c, lengths) > start);
  if (clash) return { error: `The voice-over clip “${clash.id}” already plays at that time; move or delete it first.` };
  const scenes = t.scenes.map((s, i): Scene => (i === sceneIndex ? { ...s, audio: null, narration: null } : s));
  const next = withClips({ ...t, scenes }, [...clipsOf(t), clip]);
  return { timeline: next, index: clipsOf(next).findIndex((c) => c.id === id) };
};

/** Mirrors `voiceover_problems` (validation.rs) plus overlaps of measured clips (`vimmer check`). */
export const voiceoverProblems = (t: VideoTimeline, lengths: Lengths = {}): string[] => {
  const clips = clipsOf(t);
  const out: string[] = [];
  const end = total(t);
  if (clips.length > MAX_VOICEOVER_CLIPS) out.push(`The voice-over track has ${clips.length} clips; the limit is ${MAX_VOICEOVER_CLIPS}.`);
  const seen = new Set<string>();
  clips.forEach((c) => {
    const name = `Voice-over clip “${c.id}”`;
    if (!c.id.trim()) out.push("A voice-over clip needs an id.");
    else if (seen.has(c.id)) out.push(`Two voice-over clips are called “${c.id}”.`);
    seen.add(c.id);
    if (!c.source.trim()) out.push(`${name} has no audio file.`);
    const g = c.gain ?? 1;
    if (!(g >= 0 && g <= MAX_VOICEOVER_GAIN)) out.push(`${name}: gain must be between 0 and ${MAX_VOICEOVER_GAIN}.`);
    if (c.start_ms >= end) out.push(`${name} starts after the end of the video.`);
  });
  const measured = clips.filter((c) => lengths[c.source] !== undefined).sort((a, b) => a.start_ms - b.start_ms);
  for (let i = 1; i < measured.length; i++) {
    if (measured[i].start_ms < endMs(measured[i - 1], lengths)) {
      out.push(`Voice-over clips “${measured[i - 1].id}” and “${measured[i].id}” overlap.`);
    }
  }
  return out;
};

/** The scenes a clip plays over (by index): every scene whose time overlaps the clip. */
export const scenesUnder = (t: VideoTimeline, clip: VoiceoverClip, lengths: Lengths): number[] => {
  const out: number[] = [];
  let at = 0;
  const end = endMs(clip, lengths);
  t.scenes.forEach((s, i) => {
    if (at < end && at + s.duration_ms > clip.start_ms) out.push(i);
    at += s.duration_ms;
  });
  return out;
};
