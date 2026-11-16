// Editor look (画面氛围) fields: pure helpers and their undo steps (editor/src/look.ts).
import assert from "node:assert/strict";
import { test } from "node:test";
import { addOverlay, compactLook, patchOverlay, removeOverlay, withOverlays, withPreset, withStrength } from "../editor/src/look.ts";
import { record, redo, undo, type Merge, type Undoable } from "../editor/src/history.ts";
import type { Look, Scene } from "../src/types/timeline.ts";

const scene = (p: Partial<Scene>): Scene =>
  ({ id: "a", duration_ms: 1000, text: "Hi", highlights: [], image: null, motion: "static", audio: null, ...p }) as Scene;
// What the inspector does: patch({ look }), where undefined drops the key from the saved JSON.
const patched = (s: Scene, look: Look | undefined): Scene => ({ ...s, look });

test("picking a preset sets the grade; clearing it gives back the old JSON exactly", () => {
  const s0 = scene({});
  const s1 = patched(s0, withPreset(s0.look, "warm"));
  assert.deepEqual(s1.look, { grade: { preset: "warm", strength: 1 } });
  const s2 = patched(s1, withStrength(s1.look, 0.42));
  assert.deepEqual(s2.look, { grade: { preset: "warm", strength: 0.42 } });
  // Changing the preset keeps the strength.
  assert.deepEqual(withPreset(s2.look, "noir"), { grade: { preset: "noir", strength: 0.42 } });
  const s3 = patched(s2, withPreset(s2.look, ""));
  assert.equal(s3.look, undefined);
  assert.equal(JSON.stringify(s3), JSON.stringify(s0), "no look key left behind");
  // Strength without a grade changes nothing.
  assert.equal(withStrength(undefined, 0.5), undefined);
  assert.equal(withStrength({ grade: { preset: "warm" } }, 7)!.grade!.strength, 1, "clamped");
});

test("overlays: add picks an unused kind, kinds without a color drop it, remove and inherit", () => {
  let list = addOverlay([]);
  assert.deepEqual(list, [{ kind: "snow", intensity: 0.5 }]);
  list = addOverlay(list);
  assert.equal(list[1].kind, "dust");
  list = patchOverlay(list, 1, { color: "#ffcc88", intensity: 0.8 });
  assert.deepEqual(list[1], { kind: "dust", intensity: 0.8, color: "#ffcc88" });
  // Snow has no color: switching kind drops it.
  assert.deepEqual(patchOverlay(list, 1, { kind: "snow" })[1], { kind: "snow", intensity: 0.8 });
  assert.deepEqual(patchOverlay(list, 1, { color: undefined })[1], { kind: "dust", intensity: 0.8 });
  assert.equal(patchOverlay(list, 0, { intensity: 3 })[0].intensity, 1);
  assert.deepEqual(removeOverlay(list, 0), [list[1]]);
  // Own list vs inherit: an empty own list is kept (none in this scene), undefined drops it.
  assert.deepEqual(withOverlays(undefined, []), { overlays: [] });
  assert.equal(withOverlays({ overlays: [] }, undefined), undefined);
  assert.deepEqual(withOverlays({ grade: { preset: "vivid" } }, undefined), { grade: { preset: "vivid" } });
  assert.equal(compactLook({}), undefined);
  assert.equal(compactLook(null), undefined);
});

test("a strength drag is one undo step; a preset pick is one step; undo restores the old scene exactly", () => {
  type S = Undoable<Scene>;
  const same = (a: Scene, b: Scene) => JSON.stringify(a) === JSON.stringify(b);
  const start: S = { timeline: scene({ narration: { voice: null } }), history: [], future: [], merge: null };
  // Select: discrete, no merge key.
  let s = record(start, { ...start, timeline: patched(start.timeline, withPreset(start.timeline.look, "tech_blue")) }, null, same);
  // Slider drag: many input events with one merge key, as the editor dispatches them.
  [0.95, 0.9, 0.8, 0.7, 0.6].forEach((v, i) => {
    const merge: Merge = { key: "4:patchScene:0", at: i * 50 };
    s = record(s, { ...s, timeline: patched(s.timeline, withStrength(s.timeline.look, v)) }, merge, same);
  });
  assert.equal(s.history.length, 2, "preset + one drag");
  assert.equal(s.timeline.look!.grade!.strength, 0.6);
  let back = undo(s)!;
  assert.equal(back.timeline.look!.grade!.strength, 1);
  back = undo(back)!;
  assert.equal(JSON.stringify(back.timeline), JSON.stringify(start.timeline));
  assert.equal(redo(redo(back)!)!.timeline.look!.grade!.strength, 0.6);
});
