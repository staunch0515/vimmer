import assert from "node:assert/strict";
import { test } from "node:test";
import { nothingToSpeak, spokenText, withSpokenLine, withVoice } from "../editor/src/narration.ts";
import { record, undo, redo, type Merge, type Undoable } from "../editor/src/history.ts";
import type { Scene } from "../src/types/timeline.ts";

const scene = (p: Partial<Scene>): Scene =>
  ({ id: "a", duration_ms: 1000, text: "On screen", highlights: [], image: null, motion: "static", audio: null, ...p }) as Scene;

test("the narration speaks its spoken line, else the scene text", () => {
  assert.equal(spokenText(scene({ narration: { voice: null } })), "On screen");
  assert.equal(spokenText(scene({ narration: { voice: null, text: "Said" } })), "Said");
  assert.equal(spokenText(scene({ narration: { voice: null, text: "  " } })), "On screen");
  assert.equal(spokenText(scene({ text: null, narration: { voice: null, text: "Said" } })), "Said");
  assert.equal(spokenText(scene({ narration: null })), null);
});

test("typing a line sets it; clearing it removes the field and keeps the voice", () => {
  const n = withSpokenLine({ voice: "Tingting" }, "你好");
  assert.deepEqual(n, { voice: "Tingting", text: "你好" });
  assert.deepEqual(withSpokenLine(n, ""), { voice: "Tingting" });
  assert.ok(!("text" in withSpokenLine(n, "   ")));
  assert.equal(JSON.stringify(withSpokenLine(n, "")), JSON.stringify({ voice: "Tingting" }), "saved JSON as before");
  // Changing the voice keeps the line.
  assert.deepEqual(withVoice(n, "Samantha"), { voice: "Samantha", text: "你好" });
  assert.deepEqual(withVoice({ voice: null }, null), { voice: null });
});

test("a blank line over a scene without text has nothing to speak", () => {
  assert.ok(nothingToSpeak(scene({ text: null, image: "x.png", narration: { voice: null, text: "" } })));
  assert.ok(!nothingToSpeak(scene({ narration: { voice: null, text: "" } })), "the text is spoken");
  assert.ok(!nothingToSpeak(scene({ text: null, image: "x.png", narration: { voice: null } })), "old style: as before");
  assert.ok(
    !nothingToSpeak(scene({ text: null, audio: { source: "v.wav", offset_ms: 0 }, narration: { voice: null, text: "" } })),
    "not narrated: it has audio",
  );
});

test("typing a spoken line is one undo step, and undo restores the old narration exactly", () => {
  type S = Undoable<Scene>;
  const same = (a: Scene, b: Scene) => JSON.stringify(a) === JSON.stringify(b);
  const start: S = { timeline: scene({ narration: { voice: null } }), history: [], future: [], merge: null };
  // Keystrokes in the spoken-line field, the way the editor dispatches them (one merge key).
  let s = start;
  "Hi there".split("").forEach((_, i) => {
    const line = "Hi there".slice(0, i + 1);
    const merge: Merge = { key: "7:patchScene:0", at: i * 100 };
    s = record(s, { ...s, timeline: { ...s.timeline, narration: withSpokenLine(s.timeline.narration!, line) } }, merge, same);
  });
  assert.equal(s.timeline.narration!.text, "Hi there");
  assert.equal(s.history.length, 1, "one step");
  const back = undo(s)!;
  assert.deepEqual(back.timeline, start.timeline);
  assert.equal(JSON.stringify(back.timeline.narration), '{"voice":null}');
  assert.equal(redo(back)!.timeline.narration!.text, "Hi there");
  // Clearing the field later is its own step and returns to the old shape.
  const cleared = record(s, { ...s, timeline: { ...s.timeline, narration: withSpokenLine(s.timeline.narration!, "") } }, null, same);
  assert.equal(cleared.history.length, 2);
  assert.deepEqual(cleared.timeline, start.timeline);
});
