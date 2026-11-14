import assert from "node:assert/strict";
import { test } from "node:test";
import { SAFE_MARGIN, boxProblems, clampBox, defaultBox, nudgeBox, outsideSafe, snapBox } from "../editor/src/subtitleBox.ts";

const LAND = { width: 1920, height: 1080 };
const PORT = { width: 1080, height: 1920 };
const SIZE = { w: 0.4, h: 0.1 };

test("the default box is the renderer's built-in place (bottom 4 % of the width up)", () => {
  assert.equal(defaultBox(LAND).x, 0.5);
  assert.ok(Math.abs(defaultBox(LAND).y * 1080 - (1080 - 0.04 * 1920)) < 1e-9);
  assert.ok(Math.abs(defaultBox(PORT).y * 1920 - (1920 - 0.04 * 1080)) < 1e-9);
});

test("snapping pulls to the center lines and the safe area, with guides", () => {
  // Near the vertical center line and the horizontal center line (box middle at 0.5).
  let s = snapBox({ x: 0.505, y: 0.556 }, SIZE, LAND);
  assert.deepEqual(s.box, { x: 0.5, y: 0.55 });
  assert.deepEqual(s.guides, { x: [0.5], y: [0.5] });
  // Left side onto the safe edge; bottom onto the safe bottom.
  s = snapBox({ x: SAFE_MARGIN + SIZE.w / 2 + 0.008, y: 0.945 }, SIZE, LAND);
  assert.deepEqual(s.box, { x: 0.25, y: 0.95 });
  assert.deepEqual(s.guides, { x: [SAFE_MARGIN], y: [0.95] });
  // Top onto the safe top.
  s = snapBox({ x: 0.3, y: SAFE_MARGIN + SIZE.h - 0.01 }, SIZE, LAND);
  assert.equal(s.box.y, 0.15);
  assert.deepEqual(s.guides.y, [SAFE_MARGIN]);
  // Far from everything: unchanged, no guides.
  s = snapBox({ x: 0.33, y: 0.7 }, SIZE, LAND);
  assert.deepEqual(s, { box: { x: 0.33, y: 0.7 }, guides: { x: [], y: [] } });
  // Back onto the built-in place.
  s = snapBox({ x: 0.5, y: defaultBox(LAND).y + 0.005 }, SIZE, LAND);
  assert.equal(s.box.y, clampBox(defaultBox(LAND)).y);
});

test("nudging moves by frame pixels and stays valid", () => {
  assert.deepEqual(nudgeBox({ x: 0.5, y: 0.5 }, 10, -108, LAND), { x: 0.5052, y: 0.4 });
  assert.deepEqual(nudgeBox({ x: 0.999, y: 0.9999 }, 10, 10, LAND), { x: 1, y: 1 });
  assert.equal(nudgeBox({ x: 0, y: 0.001 }, -5, -50, LAND).y, 0.001);
});

test("safe area and validation", () => {
  assert.ok(!outsideSafe({ x: 0.5, y: 0.9 }, SIZE));
  assert.ok(outsideSafe({ x: 0.2, y: 0.9 }, SIZE));
  assert.ok(outsideSafe({ x: 0.5, y: 0.97 }, SIZE));
  assert.ok(outsideSafe({ x: 0.5, y: 0.12 }, SIZE));
  assert.deepEqual(boxProblems(undefined), []);
  assert.deepEqual(boxProblems({ x: 0.5, y: 0.5 }), []);
  assert.equal(boxProblems({ x: 1.2, y: 0 }).length, 2);
});
