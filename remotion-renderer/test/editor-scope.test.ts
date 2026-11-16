import assert from "node:assert/strict";
import { test } from "node:test";
import {
  ALL_SCENES, fillWith, outOfScope, outOfScopeWarning, pruneChosen, resolveScope, sceneList, sceneOptions, scopeLine,
} from "../editor/src/scope.ts";

const s2 = { index: 1, id: "scene-02" };
const s3 = { index: 2, id: "scene-03" };
const s5 = { index: 4, id: "scene-05" };

const resolved = (r: ReturnType<typeof resolveScope>) => {
  assert.equal(r.ok, true, JSON.stringify(r));
  return (r as Extract<typeof r, { ok: true }>).value;
};

test("scope line lists the chosen scenes in timeline order", () => {
  assert.equal(sceneList([s5, s2, s3]), "第 2 幕 (scene-02)、第 3 幕 (scene-03)、第 5 幕 (scene-05)");
  assert.equal(scopeLine([s3, s2]), "只修改这些场景：第 2 幕 (scene-02)、第 3 幕 (scene-03)。其它场景不要改。");
});

test("全部: no scene id, text as typed, {场景} becomes 所有场景", () => {
  const v = resolved(resolveScope({ mode: "all", text: " 整体调成温暖色调 ", current: s2, chosen: [s3] }));
  assert.deepEqual(v, { mode: "all", instruction: "整体调成温暖色调", sceneId: null, line: null, scopeIds: null });
  assert.equal(resolved(resolveScope({ mode: "all", text: "{场景}节奏快一点", current: null, chosen: [] })).instruction, `${ALL_SCENES}节奏快一点`);
});

test("当前这一幕: scene id of the selected scene, {场景} filled; text otherwise unchanged", () => {
  const v = resolved(resolveScope({ mode: "one", text: "把{场景}按句子拆成几幕", current: s3, chosen: [s2] }));
  assert.deepEqual(v, { mode: "one", instruction: "把第 3 幕 (scene-03)按句子拆成几幕", sceneId: "scene-03", line: null, scopeIds: ["scene-03"] });
  assert.equal(resolved(resolveScope({ mode: "one", text: "慢一点", current: s3, chosen: [] })).instruction, "慢一点");
});

test("当前这一幕 with no scene selected: like the old unticked checkbox, unless {场景} needs one", () => {
  const v = resolved(resolveScope({ mode: "one", text: "慢一点", current: null, chosen: [] }));
  assert.deepEqual(v, { mode: "all", instruction: "慢一点", sceneId: null, line: null, scopeIds: null });
  assert.deepEqual(resolveScope({ mode: "one", text: "{场景}慢一点", current: null, chosen: [] }), { ok: false, error: "needsScene" });
});

test("选几幕: no scene id, the scope line first, {场景} becomes the same list", () => {
  const v = resolved(resolveScope({ mode: "selected", text: "把{场景}的节奏放慢", current: s2, chosen: [s5, s2] }));
  assert.equal(v.sceneId, null);
  assert.equal(v.line, "只修改这些场景：第 2 幕 (scene-02)、第 5 幕 (scene-05)。其它场景不要改。");
  assert.equal(v.instruction, `${v.line}\n把第 2 幕 (scene-02)、第 5 幕 (scene-05)的节奏放慢`);
  assert.deepEqual(v.scopeIds, ["scene-02", "scene-05"]);
  // Text without a placeholder is kept as typed after the line.
  assert.equal(resolved(resolveScope({ mode: "selected", text: "加淡入", current: null, chosen: [s3] })).instruction, "只修改这些场景：第 3 幕 (scene-03)。其它场景不要改。\n加淡入");
  assert.deepEqual(resolveScope({ mode: "selected", text: "加淡入", current: s2, chosen: [] }), { ok: false, error: "noScenesChosen" });
});

test("fillWith replaces every placeholder form and keeps $ patterns literal", () => {
  assert.equal(fillWith("{场景}/{ scene }/{SCENE}", "X"), "X/X/X");
  assert.equal(fillWith("{场景}", "$&$1"), "$&$1");
});

test("out-of-scope: existing scenes changed or removed outside the scope; new scenes and global lines ignored", () => {
  const known = ["scene-01", "scene-02", "scene-03"];
  const summary = [
    "scene-02: duration 4.0 s -> 3.0 s",
    "scene-02a: added as scene 3 (2.0 s, text \"x\")",
    "scene-03: transition in none -> fade",
    "scene-01: removed",
    "scene-03: text \"a\" -> \"b\"",
    "total length: 15.0 s -> 14.0 s",
    "format: 1920x1080 -> 1080x1920",
  ];
  assert.deepEqual(outOfScope(summary, ["scene-02"], known), ["scene-03", "scene-01"]);
  assert.deepEqual(outOfScope(summary, ["scene-01", "scene-02", "scene-03"], known), []);
  assert.deepEqual(outOfScope(summary, null, known), []); // 全部
  assert.equal(outOfScopeWarning(["scene-03", "scene-01"]).startsWith("这个方案改动了范围以外的场景：scene-03、scene-01"), true);
});

test("scene options: number, id and first words; chosen ids pruned to existing scenes", () => {
  const opts = sceneOptions([{ id: "scene-01", text: "  一二三四五六七八九十十一 " }, { id: "scene-02" }]);
  assert.deepEqual(opts, [
    { index: 0, id: "scene-01", preview: "一二三四五六七八九十…" },
    { index: 1, id: "scene-02", preview: "" },
  ]);
  assert.deepEqual(pruneChosen(["scene-02", "gone", "scene-01"], [{ id: "scene-01" }, { id: "scene-02" }]), ["scene-02", "scene-01"]);
});
