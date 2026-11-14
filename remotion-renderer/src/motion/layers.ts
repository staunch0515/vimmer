// Pure geometry and timing of scene layers. No Remotion imports and no runtime imports from other
// modules, so it can be unit-tested directly with Node (see test/motion.test.ts).
//
// Contract, mirrored by rust-core/crates/vimmer-dsl/src/layer.rs (README "Layers"):
// - x, width are fractions of the frame width; y, height of the frame height; origin top-left.
// - anchor "top_left" (default) places the box's top-left corner at (x, y); "center" its center.
// - start_ms / end_ms and keyframe at_ms count from the scene start. Everything is computed from
//   the scene-relative frame, so a scene renders the same in a chunk as in a full render
//   (chunks always start on a scene cut: vimmer-scheduler/src/plan.rs never splits a scene).
// - keyframe x / y offset the box in percent of the frame width / height, scale multiplies its
//   size, opacity multiplies the layer opacity, rotation adds to the layer rotation. Rotation and
//   scale turn and grow the box about its center.
// - enter runs from the layer's first frame, exit ends on its last frame.
// - stacking in a scene: scene image, layers with z < 0, scene text, layers with z >= 0; equal z
//   keeps list order (splitLayers).

import type { Easing, Layer, LayerAnimationKind } from "../types/timeline";

// Pose from the layer's keyframes (motion/keyframes.ts sampleKeyframes), passed in by the caller.
export type KeyframePoseLike = { scale: number; x: number; y: number; opacity: number; rotation: number };

const NO_KEYFRAMES: KeyframePoseLike = { scale: 1, x: 0, y: 0, opacity: 1, rotation: 0 };

// Must match `Scene::duration_frames` rounding in Rust and layout.ts msToFrames.
const msToFrames = (ms: number, fps: number): number => Math.round((ms * fps) / 1000);
const clamp01 = (v: number): number => Math.min(1, Math.max(0, v));

export const DEFAULT_ANIMATION_MS = 400;
// Slide distance, as a fraction of the frame's shorter side.
export const SLIDE_DISTANCE = 0.06;
// Pop starts (and ends) at this scale.
export const POP_FROM = 0.5;

// [start, end) of the layer in scene frames.
export const layerFrames = (layer: Layer, fps: number, sceneFrames: number): { start: number; end: number } => {
  const start = msToFrames(layer.start_ms ?? 0, fps);
  const end = layer.end_ms == null ? sceneFrames : Math.min(sceneFrames, msToFrames(layer.end_ms, fps));
  return { start, end };
};

const easeOutCubic = (t: number): number => 1 - (1 - t) ** 3;
const easeInCubic = (t: number): number => t ** 3;
// Overshoots to about 1.1 before settling at 1.
const easeOutBack = (t: number): number => {
  const c1 = 1.70158;
  const c3 = c1 + 1;
  return 1 + c3 * (t - 1) ** 3 + c1 * (t - 1) ** 2;
};

export type AnimationPose = { dx: number; dy: number; scale: number; opacity: number };
export const REST: AnimationPose = { dx: 0, dy: 0, scale: 1, opacity: 1 };

// Pose of an enter animation at progress p (0 = start, 1 = done) or of an exit animation at
// progress p (0 = start of the exit, fully shown; 1 = gone). Offsets in px. `curve` replaces the
// kind's own progress curve (the animation's `ease`); it may overshoot 1 (back_out, elastic_out).
export const animationPose = (
  kind: LayerAnimationKind,
  p: number,
  phase: "enter" | "exit",
  frameWidth: number,
  frameHeight: number,
  curve?: (t: number) => number,
): AnimationPose => {
  const t = clamp01(p);
  const d = SLIDE_DISTANCE * Math.min(frameWidth, frameHeight);
  if (phase === "enter") {
    const e = curve ? curve(t) : easeOutCubic(t);
    const away = d * (1 - e);
    switch (kind) {
      case "fade":
        return { dx: 0, dy: 0, scale: 1, opacity: e };
      case "slide_up": // from below
        return { dx: 0, dy: away, scale: 1, opacity: e };
      case "slide_down": // from above
        return { dx: 0, dy: 0 - away, scale: 1, opacity: e };
      case "slide_left": // from the right
        return { dx: away, dy: 0, scale: 1, opacity: e };
      case "slide_right": // from the left
        return { dx: 0 - away, dy: 0, scale: 1, opacity: e };
      case "pop":
        return { dx: 0, dy: 0, scale: POP_FROM + (1 - POP_FROM) * (curve ? e : easeOutBack(t)), opacity: clamp01(t / 0.35) };
    }
  }
  const e = curve ? curve(t) : easeInCubic(t);
  const gone = d * e;
  switch (kind) {
    case "fade":
      return { dx: 0, dy: 0, scale: 1, opacity: 1 - e };
    case "slide_up": // out upward
      return { dx: 0, dy: 0 - gone, scale: 1, opacity: 1 - e };
    case "slide_down":
      return { dx: 0, dy: gone, scale: 1, opacity: 1 - e };
    case "slide_left":
      return { dx: 0 - gone, dy: 0, scale: 1, opacity: 1 - e };
    case "slide_right":
      return { dx: gone, dy: 0, scale: 1, opacity: 1 - e };
    case "pop":
      return { dx: 0, dy: 0, scale: 1 - (1 - POP_FROM) * e, opacity: 1 - clamp01((t - 0.65) / 0.35) };
  }
};

export type LayerFrameState = {
  // False outside [start, end): draw nothing.
  visible: boolean;
  // Box before the transform, in px. `height` null: as tall as the content (text).
  left: number;
  top: number;
  width: number;
  height: number | null;
  // CSS transform about the box center (transform-origin: center).
  transform: string;
  opacity: number;
};

const r4 = (v: number): number => Math.round(v * 1e4) / 1e4;

// Easing curves by name (motion/keyframes.ts applyEasing), passed in by the caller so this module
// keeps no runtime imports.
export type ApplyEasing = (ease: Easing, t: number) => number;

const curveOf = (ease: Easing | undefined, easing: ApplyEasing | undefined): ((t: number) => number) | undefined =>
  ease && easing ? (t) => easing(ease, t) : undefined;

// State of `layer` on scene frame `frame`. `kf` is the keyframe pose at that frame (identity when
// the layer has no keyframes). `easing` is needed only for enter/exit animations with an `ease`.
export const layerStateAt = (
  layer: Layer,
  frame: number,
  fps: number,
  sceneFrames: number,
  frameWidth: number,
  frameHeight: number,
  kf: KeyframePoseLike = NO_KEYFRAMES,
  easing?: ApplyEasing,
): LayerFrameState => {
  const { start, end } = layerFrames(layer, fps, sceneFrames);
  const width = layer.width * frameWidth;
  const height = layer.height == null ? null : layer.height * frameHeight;
  const base = {
    left: layer.x * frameWidth,
    top: layer.y * frameHeight,
    width,
    height,
  };
  if (frame < start || frame >= end) {
    return { ...base, visible: false, transform: "none", opacity: 0 };
  }
  const local = frame - start;
  const span = end - start;
  let anim = REST;
  if (layer.enter) {
    const n = Math.max(1, msToFrames(layer.enter.duration_ms ?? DEFAULT_ANIMATION_MS, fps));
    if (local < n) {
      anim = animationPose(layer.enter.kind, local / n, "enter", frameWidth, frameHeight, curveOf(layer.enter.ease, easing));
    }
  }
  if (layer.exit) {
    const n = Math.max(1, msToFrames(layer.exit.duration_ms ?? DEFAULT_ANIMATION_MS, fps));
    const into = local - (span - n) + 1; // 1..n over the last n frames
    if (into > 0) {
      const out = animationPose(layer.exit.kind, into / n, "exit", frameWidth, frameHeight, curveOf(layer.exit.ease, easing));
      anim = { dx: anim.dx + out.dx, dy: anim.dy + out.dy, scale: anim.scale * out.scale, opacity: anim.opacity * out.opacity };
    }
  }
  const dx = (kf.x / 100) * frameWidth + anim.dx;
  const dy = (kf.y / 100) * frameHeight + anim.dy;
  const rotation = (layer.rotation ?? 0) + kf.rotation;
  const scale = kf.scale * anim.scale;
  const parts: string[] = [];
  if (layer.anchor === "center") parts.push("translate(-50%, -50%)");
  if (dx !== 0 || dy !== 0) parts.push(`translate(${r4(dx)}px, ${r4(dy)}px)`);
  if (rotation !== 0) parts.push(`rotate(${r4(rotation)}deg)`);
  if (scale !== 1) parts.push(`scale(${r4(scale)})`);
  return {
    ...base,
    visible: true,
    transform: parts.length ? parts.join(" ") : "none",
    opacity: clamp01((layer.opacity ?? 1) * kf.opacity * anim.opacity),
  };
};

// The scene's layers drawn under the scene text (z < 0) and over it (z >= 0), each in z order with
// list order kept for equal z.
export const splitLayers = (layers: readonly Layer[] | undefined): { below: Layer[]; above: Layer[] } => {
  const ordered = (layers ?? [])
    .map((layer, index) => ({ layer, index }))
    .sort((a, b) => (a.layer.z ?? 0) - (b.layer.z ?? 0) || a.index - b.index)
    .map((e) => e.layer);
  return {
    below: ordered.filter((l) => (l.z ?? 0) < 0),
    above: ordered.filter((l) => (l.z ?? 0) >= 0),
  };
};

// Stroke width in px of a shape (0 with a stroke color draws the default 0.004 of the height).
export const DEFAULT_STROKE = 0.004;
export const strokePx = (strokeWidth: number | undefined, hasStroke: boolean, frameHeight: number): number =>
  hasStroke ? (strokeWidth && strokeWidth > 0 ? strokeWidth : DEFAULT_STROKE) * frameHeight : 0;
