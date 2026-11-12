// Run with: npm run test:motion  (Node >= 22.6, type stripping; no bundler involved)
import assert from "node:assert/strict";
import { test } from "node:test";
import { IDENTITY_POSE, applyEasing, sampleKeyframes } from "../src/motion/keyframes.ts";
import { transitionPose } from "../src/motion/transitions.ts";
import type { Keyframe, TransitionKind } from "../src/types/timeline.ts";

const kf = (at_ms: number, extra: Partial<Keyframe> = {}): Keyframe => ({ at_ms, ...extra });
const close = (actual: number, expected: number) => assert.ok(Math.abs(actual - expected) < 1e-9, `${actual} != ${expected}`);

test("empty keyframes give the identity pose", () => {
  assert.deepEqual(sampleKeyframes([], 1234), IDENTITY_POSE);
});

test("holds the first pose before the first keyframe and the last pose after the last", () => {
  const keys = [kf(1000, { scale: 1.5, x: 10, opacity: 0.5, rotation: 30 }), kf(3000, { scale: 2 })];
  assert.deepEqual(sampleKeyframes(keys, 0), { scale: 1.5, x: 10, y: 0, opacity: 0.5, rotation: 30 });
  assert.deepEqual(sampleKeyframes(keys, 1000), { scale: 1.5, x: 10, y: 0, opacity: 0.5, rotation: 30 });
  assert.deepEqual(sampleKeyframes(keys, 9000), { scale: 2, x: 0, y: 0, opacity: 1, rotation: 0 });
  assert.deepEqual(sampleKeyframes(keys, 3000), { scale: 2, x: 0, y: 0, opacity: 1, rotation: 0 });
});

test("omitted fields default to the identity pose", () => {
  const pose = sampleKeyframes([kf(0), kf(1000, { x: 20 })], 500);
  assert.equal(pose.scale, 1);
  assert.equal(pose.opacity, 1);
  close(pose.x, 10); // ease_in_out at t = 0.5 is exactly 0.5
});

test("linear segment interpolates every channel", () => {
  const keys = [
    kf(0, { scale: 1, x: 0, y: 0, opacity: 1, rotation: 0, ease: "linear" }),
    kf(2000, { scale: 2, x: 10, y: -20, opacity: 0, rotation: 90 }),
  ];
  const pose = sampleKeyframes(keys, 500);
  close(pose.scale, 1.25);
  close(pose.x, 2.5);
  close(pose.y, -5);
  close(pose.opacity, 0.75);
  close(pose.rotation, 22.5);
});

test("the easing of the earlier keyframe governs the segment that leaves it", () => {
  const keys = [kf(0, { ease: "ease_in" }), kf(1000, { scale: 2, ease: "linear" }), kf(2000, { scale: 3 })];
  // Segment 0->1 uses ease_in: at t = 0.5 progress is 0.25, so scale = 1 + 0.25.
  close(sampleKeyframes(keys, 500).scale, 1.25);
  // Segment 1->2 uses linear: at t = 0.5 scale is 2.5.
  close(sampleKeyframes(keys, 1500).scale, 2.5);
});

test("easing curves hit their endpoints and midpoint", () => {
  for (const ease of ["linear", "ease_in", "ease_out", "ease_in_out"] as const) {
    close(applyEasing(ease, 0), 0);
    close(applyEasing(ease, 1), 1);
  }
  close(applyEasing("ease_in", 0.5), 0.25);
  close(applyEasing("ease_out", 0.5), 0.75);
  close(applyEasing("ease_in_out", 0.5), 0.5);
  assert.ok(applyEasing("ease_in_out", 0.25) < 0.25);
  assert.ok(applyEasing("ease_in_out", 0.75) > 0.75);
});

test("exact keyframe times return that keyframe's values", () => {
  const keys = [kf(0, { scale: 1 }), kf(1000, { scale: 1.2 }), kf(4000, { scale: 1.1 })];
  close(sampleKeyframes(keys, 1000).scale, 1.2);
  close(sampleKeyframes(keys, 4000).scale, 1.1);
});

test("transition poses: progress 0 hides the incoming scene, progress 1 shows it in place", () => {
  const kinds: TransitionKind[] = ["fade", "slide_left", "slide_right", "slide_up", "slide_down", "zoom_in", "wipe_left"];
  for (const kind of kinds) {
    const end = transitionPose(kind, 1);
    assert.equal(end.opacity, 1, `${kind} ends opaque`);
    const restingTransform: Record<TransitionKind, string> = {
      fade: "none",
      slide_left: "translate3d(0%, 0, 0)",
      slide_right: "translate3d(0%, 0, 0)",
      slide_up: "translate3d(0, 0%, 0)",
      slide_down: "translate3d(0, 0%, 0)",
      zoom_in: "scale(1)",
      wipe_left: "none",
    };
    assert.equal(end.transform, restingTransform[kind], kind);
    assert.equal(end.clipPath, kind === "wipe_left" ? "inset(0 0 0 0%)" : "none");
  }
  assert.equal(transitionPose("fade", 0).opacity, 0);
  assert.equal(transitionPose("zoom_in", 0).opacity, 0);
  assert.equal(transitionPose("slide_left", 0).transform, "translate3d(100%, 0, 0)");
  assert.equal(transitionPose("slide_right", 0).transform, "translate3d(-100%, 0, 0)");
  assert.equal(transitionPose("slide_up", 0).transform, "translate3d(0, 100%, 0)");
  assert.equal(transitionPose("slide_down", 0).transform, "translate3d(0, -100%, 0)");
  assert.equal(transitionPose("wipe_left", 0).clipPath, "inset(0 0 0 100%)");
  assert.equal(transitionPose("fade", 0.5).opacity, 0.5);
});

test("transition progress is clamped", () => {
  assert.equal(transitionPose("fade", -1).opacity, 0);
  assert.equal(transitionPose("fade", 3).opacity, 1);
});
