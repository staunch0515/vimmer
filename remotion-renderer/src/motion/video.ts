// Pure frame math of video clips (scene.video and video layers). No Remotion imports and no runtime
// imports, so it can be unit-tested with Node (test/motion.test.ts).
//
// Contract, mirrored by rust-core/crates/vimmer-dsl/src/layer.rs (`clip_frames`, `clip_frame_at`):
// - The clip is the part [trim_start_ms, trim_end_ms) of the file, played at `speed`; its frame n
//   (after trim and speed) shows on frame n of its scene (background) or layer.
// - Shorter than its time: `freeze` holds its last frame, `loop` starts it again, `stretch` changes
//   the speed so it ends exactly with its time. Longer: it is cut.
// - Everything is computed from the scene- or layer-relative frame, so a chunk shows the same frame
//   as a full render (chunks never split a scene: vimmer-scheduler/src/plan.rs).
//
// For a render, the scheduler has already normalized each clip (trimmed, sped, fitted, 30 fps;
// vimmer-scheduler/src/video.rs) and rewritten it to {trim 0..its length, speed 1}, so the frame
// shown is exactly frame `clipFrameAt` of that file. The editor plays the original file, so the same
// math also handles trims and speeds, with the file length from the browser when trim_end_ms is
// absent.

import type { FitTime, VideoClip } from "../types/timeline";

export const MIN_CLIP_SPEED = 0.25;
export const MAX_CLIP_SPEED = 4;

// Length of the trimmed part of the file in ms, or null when it is not known (no trim_end_ms and
// no file length).
export const clipSourceMs = (clip: VideoClip, fileMs?: number | null): number | null => {
  const start = clip.trim_start_ms ?? 0;
  const end = clip.trim_end_ms ?? fileMs ?? null;
  if (end == null) return null;
  const clamped = fileMs != null ? Math.min(end, fileMs) : end;
  return Math.max(0, clamped - start);
};

// Frames of `sourceMs` of file at `speed`, rounded, at least 1. Mirrors Rust `clip_frames`.
export const clipFrames = (sourceMs: number, speed: number, fps: number): number =>
  Math.max(1, Math.round(((sourceMs / speed) * fps) / 1000));

// The clip frame shown on frame `frame` of the clip's time. Mirrors Rust `clip_frame_at`.
export const clipFrameAt = (frame: number, frames: number, fitTime: FitTime): number => {
  const n = Math.max(1, frames);
  const f = Math.max(0, frame);
  return fitTime === "loop" ? f % n : Math.min(f, n - 1);
};

export type ClipTime = {
  // Time in the file to show, in ms.
  fileMs: number;
  // The clip's own frame (after trim and speed) shown, or the raw frame when its length is unknown.
  clipFrame: number;
  // The speed actually used (fitted for stretch).
  speed: number;
};

// Where in the file to look on frame `frame` of a clip shown `spanFrames` frames. `fileMs` is the
// file length when known (only needed without trim_end_ms).
export const clipTimeAt = (
  clip: VideoClip,
  frame: number,
  spanFrames: number,
  fps: number,
  fileMs?: number | null,
): ClipTime => {
  const start = clip.trim_start_ms ?? 0;
  const sourceMs = clipSourceMs(clip, fileMs);
  const fit = clip.fit_time ?? "freeze";
  const span = Math.max(1, spanFrames);
  if (sourceMs == null) {
    // Length unknown: play straight on (the browser holds the last frame at the end).
    const speed = clip.speed ?? 1;
    const f = Math.max(0, frame);
    return { fileMs: start + (f * 1000 * speed) / fps, clipFrame: f, speed };
  }
  const speed = fit === "stretch" ? sourceMs / ((span * 1000) / fps) : clip.speed ?? 1;
  const frames = fit === "stretch" ? span : Math.min(clipFrames(sourceMs, speed, fps), span);
  const g = clipFrameAt(frame, frames, fit === "stretch" ? "freeze" : fit);
  return { fileMs: start + (g * 1000 * speed) / fps, clipFrame: g, speed };
};
