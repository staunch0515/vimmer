// Layer keyframe operations and checks. Pure (type imports only), so it is unit-tested with Node
// (test/editor-keyframes.test.ts).
//
// Layer keyframes reuse the scene `Keyframe`: at_ms from the scene start, x / y offsets in percent
// of the frame, scale about the box center, opacity multiplying the layer's, rotation adding to
// it, `ease` shaping the segment that leaves the keyframe (rust-core/crates/vimmer-dsl/src/layer.rs,
// remotion-renderer/src/motion/layers.ts). The checks mirror `layer_keyframe_problems` in
// vimmer-dsl/src/validation.rs: sorted, no duplicate times, inside the scene, value ranges.
// The editor also keeps new and moved keyframes inside the layer's own time.

import type { Keyframe, Layer } from "../../src/types/timeline";

export const MAX_LAYER_KEYFRAMES = 24;

export type Pose = { scale: number; x: number; y: number; opacity: number; rotation: number };
export type PoseField = keyof Pose;

/** The layer's shown time, from the scene start: [start, end]. */
export const layerSpan = (layer: Layer, sceneMs: number): { start: number; end: number } => {
  const start = Math.max(0, Math.min(layer.start_ms ?? 0, sceneMs));
  const end = Math.max(start, Math.min(layer.end_ms ?? sceneMs, sceneMs));
  return { start, end };
};

const r3 = (v: number): number => Math.round(v * 1000) / 1000;

// Leaves out undefined keys so they are not written to the JSON.
const compact = (k: Keyframe): Keyframe => Object.fromEntries(Object.entries(k).filter(([, v]) => v !== undefined)) as Keyframe;

const withKeyframes = (layer: Layer, list: Keyframe[]): Layer => {
  if (list.length > 0) return { ...layer, keyframes: list };
  const { keyframes: _k, ...rest } = layer;
  return rest as Layer;
};

/**
 * Adds a keyframe at `atMs` (clamped to the layer's time, whole ms) holding `pose`, the layer's
 * keyframe pose at that moment, so adding one changes nothing on screen until it is edited. It
 * keeps the ease of the segment it splits. Null when one already exists there or the list is full.
 */
export const addKeyframe = (layer: Layer, sceneMs: number, atMs: number, pose: Pose): { layer: Layer; index: number } | null => {
  const list = layer.keyframes ?? [];
  if (list.length >= MAX_LAYER_KEYFRAMES) return null;
  const { start, end } = layerSpan(layer, sceneMs);
  const at = Math.max(start, Math.min(Math.round(atMs), end));
  if (list.some((k) => k.at_ms === at)) return null;
  const before = [...list].reverse().find((k) => k.at_ms < at);
  const kf = compact({
    at_ms: at,
    scale: r3(pose.scale),
    x: r3(pose.x),
    y: r3(pose.y),
    opacity: r3(pose.opacity),
    rotation: r3(pose.rotation),
    ease: before?.ease,
  });
  const next = [...list, kf].sort((a, b) => a.at_ms - b.at_ms);
  return { layer: withKeyframes(layer, next), index: next.indexOf(kf) };
};

/** The times keyframe `index` may move to: after the one before, before the one after, inside the layer. */
export const keyframeRange = (layer: Layer, sceneMs: number, index: number): { lo: number; hi: number } => {
  const list = layer.keyframes ?? [];
  const { start, end } = layerSpan(layer, sceneMs);
  const lo = index > 0 ? Math.max(start, list[index - 1].at_ms + 1) : start;
  const hi = index < list.length - 1 ? Math.min(end, list[index + 1].at_ms - 1) : end;
  return { lo, hi };
};

/** Moves keyframe `index` to `atMs`, clamped to `keyframeRange` (order kept, no duplicates). */
export const moveKeyframe = (layer: Layer, sceneMs: number, index: number, atMs: number): Layer => {
  const list = layer.keyframes ?? [];
  if (!list[index]) return layer;
  const { lo, hi } = keyframeRange(layer, sceneMs, index);
  if (lo > hi) return layer;
  const at = Math.max(lo, Math.min(Math.round(atMs), hi));
  if (at === list[index].at_ms) return layer;
  return withKeyframes(layer, list.map((k, i) => (i === index ? { ...k, at_ms: at } : k)));
};

/** Sets values or the ease of keyframe `index` (a key set to undefined goes back to its default). Time moves with `moveKeyframe`. */
export const patchKeyframe = (layer: Layer, index: number, patch: Partial<Omit<Keyframe, "at_ms">>): Layer => {
  const list = layer.keyframes ?? [];
  const k = list[index];
  if (!k) return layer;
  return withKeyframes(layer, list.map((x, i) => (i === index ? compact({ ...x, ...patch, at_ms: k.at_ms }) : x)));
};

export const deleteKeyframe = (layer: Layer, index: number): Layer => {
  const list = layer.keyframes ?? [];
  if (!list[index]) return layer;
  return withKeyframes(layer, list.filter((_, i) => i !== index));
};

/** Error for a keyframe value, or null when the DSL accepts it. */
export const fieldError = (field: PoseField, v: number): string | null => {
  if (!Number.isFinite(v)) return "Enter a number.";
  switch (field) {
    case "scale":
      return v > 0 && v <= 10 ? null : "Above 0, at most 10.";
    case "opacity":
      return v >= 0 && v <= 1 ? null : "0 to 1.";
    case "x":
    case "y":
      return Math.abs(v) <= 200 ? null : "±200 (percent of the frame).";
    case "rotation":
      return Math.abs(v) <= 360 ? null : "±360.";
  }
};

/** Error for moving keyframe `index` to `v` ms, or null. */
export const timeError = (layer: Layer, sceneMs: number, index: number, v: number): string | null => {
  if (!Number.isInteger(v)) return "Whole milliseconds.";
  const { lo, hi } = keyframeRange(layer, sceneMs, index);
  return v >= lo && v <= hi ? null : `${lo} to ${hi} (between its neighbours, inside the layer).`;
};

/** Problems with a layer's keyframes, worded for the problem bar (mirrors layer_keyframe_problems). */
export const keyframeProblems = (name: string, keyframes: Keyframe[] | undefined, sceneMs: number): string[] => {
  const out: string[] = [];
  const list = keyframes ?? [];
  if (list.length > MAX_LAYER_KEYFRAMES) out.push(`${name}: ${list.length} keyframes; the limit is ${MAX_LAYER_KEYFRAMES}.`);
  list.forEach((k, i) => {
    const n = `${name} keyframe ${i + 1}`;
    if (!(Number.isInteger(k.at_ms) && k.at_ms >= 0)) out.push(`${n}: time must be whole milliseconds, 0 or more.`);
    else if (k.at_ms > sceneMs) out.push(`${n} is after the scene ends (${sceneMs} ms).`);
    if (i > 0 && k.at_ms <= list[i - 1].at_ms) out.push(`${n} must be later than the one before it (sorted, no duplicate times).`);
    for (const f of ["scale", "opacity", "x", "y", "rotation"] as const) {
      const v = k[f];
      const err = v === undefined ? null : fieldError(f, v);
      if (err) out.push(`${n}: ${f} ${err.charAt(0).toLowerCase()}${err.slice(1)}`);
    }
  });
  return out;
};
