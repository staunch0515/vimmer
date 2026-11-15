import assert from "node:assert/strict";
import { test } from "node:test";
import {
  KEY_STEP,
  KEY_STEP_LARGE,
  clampSize,
  commit,
  dragSize,
  keyResize,
  loadStored,
  maxSize,
  openSize,
  parseStored,
  restore,
  saveStored,
  shownSize,
  storageKey,
  type Limits,
} from "../editor/src/components/layout/layout.ts";

const side: Limits = { min: 200, max: 600, minRest: 400, collapsible: true };
const fixed: Limits = { min: 200, max: 600, minRest: 400 };

test("sizes clamp to min and max", () => {
  assert.equal(clampSize(100, fixed, 1400), 200);
  assert.equal(clampSize(900, fixed, 1400), 600);
  assert.equal(clampSize(321.6, fixed, 1400), 322);
});

test("the other pane keeps its minimum, and the window edge is never crossed", () => {
  // 900 px: the pane may take at most 900 - 400.
  assert.equal(maxSize(fixed, 900), 500);
  assert.equal(clampSize(600, fixed, 900), 500);
  // Too small for both minimums: the pane gives way (below its own min) rather than overflow.
  assert.equal(clampSize(300, fixed, 500), 100);
  assert.equal(clampSize(300, fixed, 300), 0);
  assert.equal(maxSize(fixed, 0), 0);
});

test("dragging below the threshold collapses and keeps the open size for reopening", () => {
  // Threshold defaults to half the min (100).
  const r = dragSize(300, -220, side, 1400, 300);
  assert.deepEqual(r, { size: 300, collapsed: true });
  // Between the threshold and the min: open, at the min.
  assert.deepEqual(dragSize(300, -150, side, 1400, 300), { size: 200, collapsed: false });
  // A pane that cannot collapse stops at its min.
  assert.deepEqual(dragSize(300, -280, fixed, 1400, 300), { size: 200, collapsed: false });
});

test("dragging a collapsed pane back out reopens it", () => {
  assert.deepEqual(dragSize(0, 60, side, 1400, 300), { size: 300, collapsed: true });
  assert.deepEqual(dragSize(0, 140, side, 1400, 300), { size: 200, collapsed: false });
  assert.deepEqual(dragSize(0, 380, side, 1400, 300), { size: 380, collapsed: false });
  assert.deepEqual(dragSize(0, 50, { ...side, collapseBelow: 40 }, 1400, 300), { size: 200, collapsed: false });
});

test("a collapsed pane shows as 0 and keeps its open size", () => {
  const s = commit(350, true, 1400);
  assert.equal(shownSize(s, side, 1400), 0);
  assert.equal(openSize(s, side, 1400), 350);
});

test("restore goes back to the default size, open", () => {
  const r = restore(320);
  assert.deepEqual(r, { size: 320, basis: null, collapsed: false });
  assert.equal(shownSize(r, side, 1400), 320);
  // A plain default does not scale with the window.
  assert.equal(shownSize(r, side, 1000), 320);
});

test("a remembered size keeps its proportion when the window resizes", () => {
  const s = commit(400, false, 1600); // a quarter
  assert.equal(shownSize(s, side, 1600), 400);
  assert.equal(shownSize(s, side, 2000), 500);
  assert.equal(shownSize(s, side, 1200), 300);
  // ...then the limits apply: a quarter of 4000 is past max, a quarter of 600 below min (and
  // 600 - minRest leaves only 200).
  assert.equal(shownSize(s, side, 4000), 600);
  assert.equal(shownSize(s, side, 600), 200);
  assert.equal(shownSize(s, side, 500), 100);
});

test("keys: arrows step, Shift steps further, the sign follows the pane's side", () => {
  assert.deepEqual(keyResize("ArrowRight", false, "x", 1), { kind: "delta", delta: KEY_STEP });
  assert.deepEqual(keyResize("ArrowRight", true, "x", 1), { kind: "delta", delta: KEY_STEP_LARGE });
  // A pane on the right shrinks when its divider moves right.
  assert.deepEqual(keyResize("ArrowRight", false, "x", -1), { kind: "delta", delta: -KEY_STEP });
  assert.deepEqual(keyResize("ArrowUp", false, "y", -1), { kind: "delta", delta: KEY_STEP });
  assert.equal(keyResize("ArrowUp", false, "x", 1), null);
  assert.deepEqual(keyResize("Home", false, "x", 1), { kind: "min" });
  assert.deepEqual(keyResize("End", false, "y", 1), { kind: "max" });
  assert.deepEqual(keyResize("Enter", false, "y", 1), { kind: "toggle" });
  assert.equal(keyResize("a", false, "x", 1), null);
});

test("stored sizes round-trip and bad entries are ignored", () => {
  const map = new Map<string, string>();
  const store = {
    getItem: (k: string) => map.get(k) ?? null,
    setItem: (k: string, v: string) => void map.set(k, v),
    removeItem: (k: string) => void map.delete(k),
  };
  saveStored(store, "inspector", commit(333, false, 1200));
  assert.deepEqual(loadStored(store, "inspector"), { size: 333, basis: 1200, collapsed: false });
  assert.equal(loadStored(store, "timeline"), null);
  saveStored(store, "inspector", null);
  assert.equal(map.has(storageKey("inspector")), false);

  assert.equal(parseStored("not json"), null);
  assert.equal(parseStored(JSON.stringify({ size: -5 })), null);
  assert.equal(parseStored(JSON.stringify({ size: "300" })), null);
  assert.deepEqual(parseStored(JSON.stringify({ size: 300, basis: 0 })), { size: 300, basis: null, collapsed: false });
});

test("storage that throws is survived", () => {
  const broken = {
    getItem: () => {
      throw new Error("denied");
    },
    setItem: () => {
      throw new Error("full");
    },
    removeItem: () => {
      throw new Error("denied");
    },
  };
  assert.equal(loadStored(broken, "x"), null);
  assert.doesNotThrow(() => saveStored(broken, "x", restore(10)));
  assert.equal(loadStored(null, "x"), null);
});
