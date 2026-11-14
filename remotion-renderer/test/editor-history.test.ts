import assert from "node:assert/strict";
import { test } from "node:test";
import { HISTORY_LIMIT, MERGE_IDLE_MS, endMerge, mergeKey, mergesInput, record, redo, undo, type Merge, type Undoable } from "../editor/src/history.ts";

type Doc = { text: string };
type S = Undoable<Doc> & { selection: string };
const same = (a: Doc, b: Doc) => a.text === b.text;
const start: S = { timeline: { text: "" }, history: [], future: [], merge: null, selection: "none" };
// One edit through `record`, the way the editor reducer does it.
const edit = (s: S, text: string, merge: Merge = null): S => record(s, { ...s, timeline: { text } }, merge, same);

test("a discrete edit is one step; undo and redo walk the steps", () => {
  let s = edit(start, "a");
  s = edit(s, "ab");
  assert.equal(s.history.length, 2);
  s = undo(s)!;
  assert.equal(s.timeline.text, "a");
  s = redo(s)!;
  assert.equal(s.timeline.text, "ab");
  assert.equal(redo(s), null);
  assert.equal(undo(undo(undo(s)!)!), null);
});

test("keystrokes in one field within the idle time are one step", () => {
  let s = start;
  "hello".split("").forEach((_, i) => {
    s = edit(s, "hello".slice(0, i + 1), { key: "f1", at: i * 200 });
  });
  assert.equal(s.history.length, 1);
  s = undo(s)!;
  assert.equal(s.timeline.text, "", "one undo reverts the whole word");
});

test("an idle pause, a blur or another field starts a new step", () => {
  let s = edit(start, "a", { key: "f1", at: 0 });
  s = edit(s, "ab", { key: "f1", at: MERGE_IDLE_MS + 1 });
  assert.equal(s.history.length, 2, "after the idle time");
  s = endMerge(edit(s, "abc", { key: "f1", at: MERGE_IDLE_MS + 100 }));
  s = edit(s, "abcd", { key: "f1", at: MERGE_IDLE_MS + 200 });
  assert.equal(s.history.length, 3, "\"abc\" joined \"ab\"; after the blur \"abcd\" is new");
  s = edit(s, "abcde", { key: "f2", at: MERGE_IDLE_MS + 300 });
  assert.equal(s.history.length, 4, "another field");
  // A discrete edit between keystrokes breaks the run too.
  s = edit(s, "x");
  s = edit(s, "xy", { key: "f2", at: MERGE_IDLE_MS + 400 });
  assert.equal(s.history.length, 6);
});

test("assistant patches (discrete, no merge key) stay one step each, even back to back", () => {
  let s = edit(start, "proposal 1");
  s = edit(s, "proposal 2");
  assert.equal(s.history.length, 2);
  assert.equal(undo(s)!.timeline.text, "proposal 1");
});

test("selection changes and no-op edits are not steps", () => {
  const sel: S = { ...start, selection: "layer 1" };
  assert.equal(sel.history.length, 0);
  const s = edit(start, "a");
  const same1 = record(s, { ...s, timeline: { text: "a" }, selection: "scene 2" }, null, same);
  assert.equal(same1.history.length, 1);
  assert.equal(same1.timeline, s.timeline, "keeps the document object");
  assert.equal(same1.selection, "scene 2");
});

test("a new edit clears redo; history is capped", () => {
  let s = edit(edit(start, "a"), "b");
  s = undo(s)!;
  assert.equal(s.future.length, 1);
  s = edit(s, "c");
  assert.equal(s.future.length, 0);
  for (let i = 0; i < HISTORY_LIMIT + 20; i++) s = edit(s, `n${i}`);
  assert.equal(s.history.length, HISTORY_LIMIT);
});

test("which inputs merge, and merge keys include the edit's target", () => {
  assert.ok(mergesInput("INPUT", "number"));
  assert.ok(mergesInput("INPUT", null));
  assert.ok(mergesInput("TEXTAREA", null));
  assert.ok(mergesInput("INPUT", "range"));
  assert.ok(mergesInput("DIV", null, true));
  assert.ok(!mergesInput("INPUT", "checkbox"));
  assert.ok(!mergesInput("SELECT", null));
  assert.notEqual(
    mergeKey(1, { type: "patchLayer", sceneIndex: 0, index: 1 }),
    mergeKey(1, { type: "patchLayer", sceneIndex: 0, index: 2 }),
  );
  assert.equal(mergeKey(3, { type: "patchScene", index: 0, patch: { text: "a" } }), mergeKey(3, { type: "patchScene", index: 0, patch: { text: "ab" } }));
});
