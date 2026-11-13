// Pure keyframe sampling for a scene's image layer. Has no Remotion imports so it can be
// unit-tested directly (see test/motion.test.ts).
//
// Semantics, matching the validation in rust-core/crates/vimmer-dsl/src/validation.rs:
// - keyframes are sorted by at_ms, with no duplicates, and lie within the scene;
// - before the first keyframe the first pose is held; after the last, the last pose is held;
// - between two keyframes, every value is interpolated with the easing of the earlier one
//   (its `ease` governs the segment that leaves it).

import type { Easing, Keyframe } from "../types/timeline";

export type KeyframePose = {
  scale: number;
  x: number;
  y: number;
  opacity: number;
  rotation: number;
};

export const IDENTITY_POSE: KeyframePose = { scale: 1, x: 0, y: 0, opacity: 1, rotation: 0 };

// Maps progress t in [0, 1] (clamped) to eased progress. Every curve starts at 0 and ends at 1;
// back_out and elastic_out go above 1 on the way. The quadratic curves keep motion gentle.
// Mirrors `Easing::apply` in rust-core/crates/vimmer-dsl/src/model.rs.
export const applyEasing = (ease: Easing | undefined, t: number): number => {
  const x = Math.min(1, Math.max(0, t));
  switch (ease ?? "ease_in_out") {
    case "linear":
      return x;
    case "ease_in":
      return x * x;
    case "ease_out":
      return 1 - (1 - x) * (1 - x);
    case "ease_in_out":
      return x < 0.5 ? 2 * x * x : 1 - ((-2 * x + 2) ** 2) / 2;
    case "back_out": {
      const c1 = 1.70158;
      const c3 = c1 + 1;
      return 1 + c3 * (x - 1) ** 3 + c1 * (x - 1) ** 2;
    }
    case "elastic_out": {
      if (x === 0 || x === 1) return x;
      const c4 = (2 * Math.PI) / 3;
      return 2 ** (-10 * x) * Math.sin((x * 10 - 0.75) * c4) + 1;
    }
    case "bounce_out": {
      const n1 = 7.5625;
      const d1 = 2.75;
      if (x < 1 / d1) return n1 * x * x;
      if (x < 2 / d1) return n1 * (x - 1.5 / d1) ** 2 + 0.75;
      if (x < 2.5 / d1) return n1 * (x - 2.25 / d1) ** 2 + 0.9375;
      return n1 * (x - 2.625 / d1) ** 2 + 0.984375;
    }
  }
};

const poseOf = (kf: Keyframe): KeyframePose => ({
  scale: kf.scale ?? 1,
  x: kf.x ?? 0,
  y: kf.y ?? 0,
  opacity: kf.opacity ?? 1,
  rotation: kf.rotation ?? 0,
});

const lerp = (a: number, b: number, t: number): number => a + (b - a) * t;

// Pose of the image layer at `timeMs` from the scene start. `keyframes` must be validated
// (sorted, unique). An empty list yields the identity pose; callers normally use `motion` then.
export const sampleKeyframes = (keyframes: readonly Keyframe[], timeMs: number): KeyframePose => {
  if (keyframes.length === 0) return IDENTITY_POSE;
  const first = keyframes[0];
  if (timeMs <= first.at_ms) return poseOf(first);
  const last = keyframes[keyframes.length - 1];
  if (timeMs >= last.at_ms) return poseOf(last);

  // Find the segment [keyframes[i], keyframes[i + 1]] that contains timeMs.
  let i = 0;
  while (keyframes[i + 1].at_ms <= timeMs) i += 1;
  const from = keyframes[i];
  const to = keyframes[i + 1];
  // Validation guarantees to.at_ms > from.at_ms, so this never divides by zero.
  const t = applyEasing(from.ease, (timeMs - from.at_ms) / (to.at_ms - from.at_ms));
  const a = poseOf(from);
  const b = poseOf(to);
  return {
    scale: lerp(a.scale, b.scale, t),
    x: lerp(a.x, b.x, t),
    y: lerp(a.y, b.y, t),
    opacity: lerp(a.opacity, b.opacity, t),
    rotation: lerp(a.rotation, b.rotation, t),
  };
};
