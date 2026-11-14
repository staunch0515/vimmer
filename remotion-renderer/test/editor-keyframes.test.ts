import assert from "node:assert/strict";
import { test } from "node:test";
import {
  MAX_LAYER_KEYFRAMES,
  addKeyframe,
  deleteKeyframe,
  fieldError,
  keyframeProblems,
  keyframeRange,
  layerSpan,
  moveKeyframe,
  patchKeyframe,
  timeError,
} from "../editor/src/layerKeyframes.ts";
import type { Keyframe, Layer } from "../src/types/timeline.ts";

const IDENTITY = { scale: 1, x: 0, y: 0, opacity: 1, rotation: 0 };
const layer = (extra: Partial<Layer> = {}): Layer =>
  ({ id: "star", kind: "shape", shape: "rect", fill: "#fff", x: 0.1, y: 0.1, width: 0.2, height: 0.2, ...extra }) as Layer;

test("the layer span defaults to the whole scene and is clamped to it", () => {
  assert.deepEqual(layerSpan(layer(), 4000), { start: 0, end: 4000 });
  assert.deepEqual(layerSpan(layer({ start_ms: 500, end_ms: 3000 }), 4000), { start: 500, end: 3000 });
  assert.deepEqual(layerSpan(layer({ start_ms: 500, end_ms: 9000 }), 4000), { start: 500, end: 4000 });
});

test("adding a keyframe captures the pose, sorts, and is clamped to the layer's time", () => {
  const l = layer({ start_ms: 1000, end_ms: 3000, keyframes: [{ at_ms: 2000, x: 10, ease: "back_out" }] });
  const before = addKeyframe(l, 4000, 1500.4, { scale: 1.23456, x: 5, y: -2, opacity: 0.5, rotation: 0 });
  assert.ok(before);
  assert.equal(before.index, 0);
  assert.deepEqual(before.layer.keyframes![0], { at_ms: 1500, scale: 1.235, x: 5, y: -2, opacity: 0.5, rotation: 0 });
  // After the existing keyframe it keeps that segment's ease.
  const after = addKeyframe(l, 4000, 2500, IDENTITY)!;
  assert.equal(after.index, 1);
  assert.equal(after.layer.keyframes![1].ease, "back_out");
  // Outside the layer: clamped to its end.
  assert.equal(addKeyframe(l, 4000, 3900, IDENTITY)!.layer.keyframes![1].at_ms, 3000);
  assert.equal(addKeyframe(l, 4000, 0, IDENTITY)!.layer.keyframes![0].at_ms, 1000);
});

test("no duplicate time and no more than the limit", () => {
  const l = layer({ keyframes: [{ at_ms: 100 }] });
  assert.equal(addKeyframe(l, 1000, 100, IDENTITY), null);
  const full = layer({ keyframes: Array.from({ length: MAX_LAYER_KEYFRAMES }, (_, i) => ({ at_ms: i * 10 })) });
  assert.equal(addKeyframe(full, 1000, 999, IDENTITY), null);
});

test("moving stays between the neighbours and inside the layer", () => {
  const l = layer({ start_ms: 100, end_ms: 900, keyframes: [{ at_ms: 200 }, { at_ms: 500 }, { at_ms: 800 }] });
  assert.deepEqual(keyframeRange(l, 1000, 1), { lo: 201, hi: 799 });
  assert.deepEqual(keyframeRange(l, 1000, 0), { lo: 100, hi: 499 });
  assert.deepEqual(keyframeRange(l, 1000, 2), { lo: 501, hi: 900 });
  assert.equal(moveKeyframe(l, 1000, 1, 50).keyframes![1].at_ms, 201);
  assert.equal(moveKeyframe(l, 1000, 1, 9999).keyframes![1].at_ms, 799);
  assert.equal(moveKeyframe(l, 1000, 2, 9999).keyframes![2].at_ms, 900);
  assert.equal(moveKeyframe(l, 1000, 0, 0).keyframes![0].at_ms, 100);
  assert.equal(moveKeyframe(l, 1000, 1, 500), l, "no change returns the same layer");
  assert.equal(timeError(l, 1000, 1, 300), null);
  assert.match(timeError(l, 1000, 1, 800)!, /201 to 799/);
  assert.match(timeError(l, 1000, 1, 300.5)!, /Whole/);
});

test("patching keeps the time and drops undefined keys; delete removes an empty list", () => {
  const l = layer({ keyframes: [{ at_ms: 0, scale: 2, ease: "linear" }, { at_ms: 400 }] });
  const p = patchKeyframe(l, 0, { scale: undefined, opacity: 0.4, ease: "bounce_out", at_ms: 999 } as Partial<Keyframe>);
  assert.deepEqual(p.keyframes![0], { at_ms: 0, ease: "bounce_out", opacity: 0.4 });
  const one = deleteKeyframe(l, 0);
  assert.deepEqual(one.keyframes, [{ at_ms: 400 }]);
  const none = deleteKeyframe(one, 0);
  assert.ok(!("keyframes" in none));
  assert.equal(deleteKeyframe(l, 5), l);
});

test("value ranges match vimmer-dsl layer_keyframe_problems", () => {
  assert.equal(fieldError("scale", 10), null);
  assert.ok(fieldError("scale", 0));
  assert.ok(fieldError("scale", 10.1));
  assert.equal(fieldError("opacity", 0), null);
  assert.ok(fieldError("opacity", 1.1));
  assert.equal(fieldError("x", -200), null);
  assert.ok(fieldError("y", 201));
  assert.ok(fieldError("rotation", -361));
  assert.ok(fieldError("x", NaN));
});

test("problems: unsorted, duplicate, after the scene, bad values", () => {
  assert.deepEqual(keyframeProblems("L", [{ at_ms: 0 }, { at_ms: 500 }], 500), []);
  const out = keyframeProblems("L", [{ at_ms: 300 }, { at_ms: 300 }, { at_ms: 100 }, { at_ms: 900, scale: 0 }], 500);
  assert.equal(out.filter((p) => /sorted, no duplicate/.test(p)).length, 2);
  assert.ok(out.some((p) => /keyframe 4 is after the scene ends/.test(p)));
  assert.ok(out.some((p) => /keyframe 4: scale/.test(p)));
  assert.ok(keyframeProblems("L", Array.from({ length: 25 }, (_, i) => ({ at_ms: i })), 1000)[0].includes("limit is 24"));
});
