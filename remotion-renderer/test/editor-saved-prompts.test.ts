import assert from "node:assert/strict";
import { test } from "node:test";
import {
  EXPORT_FORMAT, MAX_PROMPTS, MAX_NAME, MAX_TEXT, STARTERS, STORAGE_KEY,
  addPrompt, defaultName, deletePrompt, editPromptText, exportJson, fillPlaceholders, loadPrompts, mergeImport,
  movePrompt, needsScene, parseImport, renamePrompt, restorePrompt, savePrompts, sceneLabel, starterPrompts,
  updatePrompt, generalizeScene, categoriesOf, filterPrompts, NO_CATEGORY, MAX_CATEGORY,
  type SavedPrompt,
} from "../editor/src/savedPrompts.ts";

const ok = <T extends { ok: boolean }>(r: T): Extract<T, { ok: true }> => {
  assert.equal(r.ok, true, JSON.stringify(r));
  return r as Extract<T, { ok: true }>;
};

const memoryStorage = () => {
  const m = new Map<string, string>();
  return { getItem: (k: string) => m.get(k) ?? null, setItem: (k: string, v: string) => void m.set(k, v), m };
};

const three = (): SavedPrompt[] => [
  { id: "a", name: "A", text: "one", useScene: false },
  { id: "b", name: "B", text: "two", useScene: true },
  { id: "c", name: "C", text: "three", useScene: false },
];

test("add: default name is the first 12 characters; scene flag kept; text trimmed", () => {
  const r = ok(addPrompt([], { text: "  把这一幕按句子拆成几幕，每幕一句，用同一张图  ", useScene: true }, "x1"));
  assert.deepEqual(r.list, [{ id: "x1", name: "把这一幕按句子拆成几幕，…", text: "把这一幕按句子拆成几幕，每幕一句，用同一张图", useScene: true }]);
  assert.equal(defaultName("短的"), "短的");
  const named = ok(addPrompt([], { name: "  拆幕  ", text: "abc", useScene: false }, "x2"));
  assert.equal(named.list[0].name, "拆幕");
  assert.equal(ok(addPrompt([], { name: "n".repeat(100), text: "abc", useScene: false })).list[0].name.length, MAX_NAME);
});

test("add: refuses empty, too long and a full list", () => {
  assert.equal(addPrompt([], { text: "   ", useScene: false }).ok, false);
  assert.equal(addPrompt([], { text: "x".repeat(MAX_TEXT + 1), useScene: false }).ok, false);
  const full = Array.from({ length: MAX_PROMPTS }, (_, i) => ({ id: `c${i}`, name: "n", text: `t${i}`, useScene: false }));
  assert.equal(addPrompt(full, { text: "one more", useScene: false }).ok, false);
});

test("rename / edit text / update scene flag", () => {
  let list = three();
  list = ok(renamePrompt(list, "b", "新名字")).list;
  assert.equal(list[1].name, "新名字");
  list = ok(renamePrompt(list, "b", "  ")).list; // empty name -> default from text
  assert.equal(list[1].name, "two");
  list = ok(editPromptText(list, "b", " {场景}慢一点 ")).list;
  assert.equal(list[1].text, "{场景}慢一点");
  assert.equal(list[1].name, "two"); // editing the text keeps the name
  list = ok(updatePrompt(list, "b", { useScene: false })).list;
  assert.equal(list[1].useScene, false);
  assert.equal(editPromptText(list, "b", "").ok, false);
  assert.equal(renamePrompt(list, "zzz", "x").ok, false);
  assert.deepEqual(list.map((c) => c.id), ["a", "b", "c"]);
});

test("delete and undo puts it back in place", () => {
  const { list, removed } = deletePrompt(three(), "b");
  assert.deepEqual(list.map((c) => c.id), ["a", "c"]);
  assert.ok(removed);
  assert.deepEqual(restorePrompt(list, removed).map((c) => c.id), ["a", "b", "c"]);
  // Undo twice does not duplicate.
  const back = restorePrompt(list, removed);
  assert.equal(restorePrompt(back, removed), back);
  assert.equal(deletePrompt(three(), "nope").removed, null);
});

test("move up / down, stopping at the ends", () => {
  assert.deepEqual(movePrompt(three(), "c", -1).map((c) => c.id), ["a", "c", "b"]);
  assert.deepEqual(movePrompt(three(), "a", 1).map((c) => c.id), ["b", "a", "c"]);
  const l = three();
  assert.equal(movePrompt(l, "a", -1), l);
  assert.equal(movePrompt(l, "c", 1), l);
});

test("placeholder: {场景} and {scene} become the selected scene's label", () => {
  assert.equal(sceneLabel(2, "scene-03"), "第 3 幕 (scene-03)");
  assert.equal(needsScene("把{场景}拆开"), true);
  assert.equal(needsScene("make {Scene} slower"), true);
  assert.equal(needsScene("整体调成温暖色调"), false);
  const r = fillPlaceholders("把{场景}按句子拆成几幕，{ scene }也一样", { index: 2, id: "scene-03" });
  assert.deepEqual(r, { ok: true, text: "把第 3 幕 (scene-03)按句子拆成几幕，第 3 幕 (scene-03)也一样" });
  assert.deepEqual(fillPlaceholders("{场景}节奏快一点", null), { ok: false, reason: "needsScene" });
  assert.deepEqual(fillPlaceholders("整体调成温暖色调", null), { ok: true, text: "整体调成温暖色调" });
  // Called repeatedly (global regex state must not leak).
  assert.equal(needsScene("{场景}"), true);
  assert.equal(needsScene("{场景}"), true);
});

test("starters: 3-5 Chinese prompts, valid entries with unique ids", () => {
  assert.ok(STARTERS.length >= 3 && STARTERS.length <= 5);
  const s = starterPrompts();
  assert.equal(new Set(s.map((c) => c.id)).size, s.length);
  assert.ok(s.some((c) => c.text === "把{场景}按句子拆成几幕，每幕一句，用同一张图"));
  for (const c of s) assert.equal(needsScene(c.text), c.useScene);
});

test("storage: nothing saved = null; round trip; broken data and throwing storage are survived", () => {
  const st = memoryStorage();
  assert.equal(loadPrompts(st), null);
  assert.equal(savePrompts(st, three()), true);
  assert.deepEqual(loadPrompts(st), three());
  assert.ok(st.m.has(STORAGE_KEY));
  savePrompts(st, []);
  assert.deepEqual(loadPrompts(st), []); // all deleted stays empty (no starters coming back)
  st.setItem(STORAGE_KEY, "{not json");
  assert.equal(loadPrompts(st), null);
  st.setItem(STORAGE_KEY, JSON.stringify([{ id: "a", name: "A", text: "ok", useScene: false }, { id: "b", text: 5 }, "x"]));
  assert.deepEqual(loadPrompts(st), [{ id: "a", name: "A", text: "ok", useScene: false }]);
  const throwing = { getItem: () => { throw new Error("denied"); }, setItem: () => { throw new Error("quota"); } };
  assert.equal(loadPrompts(throwing), null);
  assert.equal(savePrompts(throwing, three()), false);
  assert.equal(loadPrompts(null), null);
  assert.equal(savePrompts(undefined, three()), false);
});

test("storage: duplicate or odd ids get fresh ones", () => {
  const st = memoryStorage();
  st.setItem(STORAGE_KEY, JSON.stringify([
    { id: "a", name: "A", text: "1", useScene: false },
    { id: "a", name: "B", text: "2", useScene: false },
    { id: "<img>", name: "C", text: "3", useScene: false },
  ]));
  const list = loadPrompts(st)!;
  assert.equal(list.length, 3);
  assert.equal(new Set(list.map((c) => c.id)).size, 3);
  assert.equal(list[0].id, "a");
  assert.ok(list.every((c) => /^[A-Za-z0-9_-]+$/.test(c.id)));
});

test("export / import round trip (ids not exported)", () => {
  const json = exportJson(three());
  const data = JSON.parse(json);
  assert.equal(data.format, EXPORT_FORMAT);
  assert.equal(data.version, 1);
  assert.equal("id" in data.prompts[0], false);
  const parsed = ok(parseImport(json));
  assert.deepEqual(parsed.prompts, three().map(({ name, text, useScene }) => ({ name, text, useScene })));
});

test("import is strict: wrong format, version, types, lengths or count refuse the whole file", () => {
  const file = (commands: unknown, extra: object = {}) => JSON.stringify({ format: EXPORT_FORMAT, version: 1, prompts: commands, ...extra });
  const good = { name: "n", text: "t", useScene: false };
  assert.equal(parseImport("nope").ok, false);
  assert.equal(parseImport("[]").ok, false);
  assert.equal(parseImport(JSON.stringify({ format: "other", version: 1, prompts: [] })).ok, false);
  assert.equal(parseImport(file([], { version: 2 })).ok, false);
  assert.equal(parseImport(file("x")).ok, false);
  assert.equal(parseImport(file([good, { ...good, useScene: "yes" }])).ok, false);
  assert.equal(parseImport(file([{ ...good, text: 42 }])).ok, false);
  assert.equal(parseImport(file([{ ...good, text: "   " }])).ok, false);
  assert.equal(parseImport(file([{ ...good, text: "x".repeat(MAX_TEXT + 1) }])).ok, false);
  assert.equal(parseImport(file([{ ...good, name: "x".repeat(MAX_NAME + 1) }])).ok, false);
  assert.equal(parseImport(file(Array.from({ length: MAX_PROMPTS + 1 }, () => good))).ok, false);
  assert.equal(parseImport(" ".repeat(300_000)).ok, false);
  // Extra unknown fields are dropped, never kept.
  const r = ok(parseImport(file([{ ...good, onClick: "alert(1)" }])));
  assert.deepEqual(r.prompts, [good]);
});

test("merge import: appends, skips same text, stops at the maximum", () => {
  let n = 0;
  const id = () => `new${++n}`;
  const r = mergeImport(three(), [{ name: "X", text: "two", useScene: false }, { name: "Y", text: "four", useScene: true }], id);
  assert.equal(r.added, 1);
  assert.equal(r.skipped, 1);
  assert.deepEqual(r.list.map((c) => c.id), ["a", "b", "c", "new1"]);
  const many = Array.from({ length: MAX_PROMPTS }, (_, i) => ({ name: "n", text: `m${i}`, useScene: false }));
  const capped = mergeImport(three(), many, id);
  assert.equal(capped.list.length, MAX_PROMPTS);
  assert.equal(capped.added, MAX_PROMPTS - 3);
});

test("saving a sent message can turn its scene label back into {场景}", () => {
  const sent = fillPlaceholders("把{场景}按句子拆成几幕", { index: 2, id: "scene-03" });
  assert.ok(sent.ok);
  assert.equal(generalizeScene(sent.text), "把{场景}按句子拆成几幕");
  assert.equal(generalizeScene("第 3 幕也这样"), "第 3 幕也这样"); // the user's own words are left alone
});

test("category is optional: set, trimmed, capped, removed by empty; old entries without it stay valid", () => {
  let list = ok(addPrompt([], { text: "节奏快一点", useScene: false, category: "  节奏 " }, "p1")).list;
  assert.equal(list[0].category, "节奏");
  list = ok(addPrompt(list, { text: "无分类", useScene: false, category: "   " }, "p2")).list;
  assert.equal("category" in list[1], false);
  list = ok(updatePrompt(list, "p1", { category: "x".repeat(50) })).list;
  assert.equal(list[0].category!.length, MAX_CATEGORY);
  list = ok(updatePrompt(list, "p1", { category: "" })).list;
  assert.equal("category" in list[0], false);
  list = ok(updatePrompt(list, "p1", { category: "调色" })).list;
  list = ok(updatePrompt(list, "p1", { name: "new" })).list; // other edits keep it
  assert.equal(list[0].category, "调色");
  // Stored items from before categories existed still load.
  const st = memoryStorage();
  st.setItem(STORAGE_KEY, JSON.stringify([{ id: "old", name: "O", text: "old one", useScene: false }, { id: "n", name: "N", text: "x", useScene: false, category: "拆分" }]));
  assert.deepEqual(loadPrompts(st), [{ id: "old", name: "O", text: "old one", useScene: false }, { id: "n", name: "N", text: "x", useScene: false, category: "拆分" }]);
  st.setItem(STORAGE_KEY, JSON.stringify([{ id: "bad", name: "B", text: "x", useScene: false, category: 3 }]));
  assert.deepEqual(loadPrompts(st), []);
});

test("export / import keep the category; wrong category type or length refuses the file", () => {
  const list: SavedPrompt[] = [{ id: "a", name: "A", text: "one", useScene: false, category: "拆分" }, { id: "b", name: "B", text: "two", useScene: true }];
  const parsed = ok(parseImport(exportJson(list)));
  assert.deepEqual(parsed.prompts, [{ name: "A", text: "one", useScene: false, category: "拆分" }, { name: "B", text: "two", useScene: true }]);
  const file = (p: unknown) => JSON.stringify({ format: EXPORT_FORMAT, version: 1, prompts: [p] });
  assert.equal(parseImport(file({ name: "n", text: "t", useScene: false, category: 1 })).ok, false);
  assert.equal(parseImport(file({ name: "n", text: "t", useScene: false, category: "x".repeat(MAX_CATEGORY + 1) })).ok, false);
});

test("search by name / text / category, every word must match; filter by category", () => {
  const list: SavedPrompt[] = [
    { id: "a", name: "按句子拆幕", text: "把{场景}按句子拆成几幕", useScene: true, category: "拆分" },
    { id: "b", name: "Faster", text: "{场景}节奏快一点", useScene: true, category: "节奏" },
    { id: "c", name: "温暖色调", text: "整体调成温暖色调", useScene: false, category: "调色" },
    { id: "d", name: "其他", text: "加字幕", useScene: false },
  ];
  const ids = (l: SavedPrompt[]) => l.map((p) => p.id);
  assert.deepEqual(ids(filterPrompts(list, "")), ["a", "b", "c", "d"]);
  assert.deepEqual(ids(filterPrompts(list, "faster")), ["b"]); // case-insensitive name
  assert.deepEqual(ids(filterPrompts(list, "节奏")), ["b"]); // text or category
  assert.deepEqual(ids(filterPrompts(list, "{场景}")), ["a", "b"]);
  assert.deepEqual(ids(filterPrompts(list, "调色")), ["c"]); // category only… and name
  assert.deepEqual(ids(filterPrompts(list, "场景 句子")), ["a"]);
  assert.deepEqual(ids(filterPrompts(list, "", "拆分")), ["a"]);
  assert.deepEqual(ids(filterPrompts(list, "节奏", "拆分")), []);
  assert.deepEqual(ids(filterPrompts(list, "", NO_CATEGORY)), ["d"]);
  assert.deepEqual(categoriesOf([...list, { id: "e", name: "e", text: "e", useScene: false, category: "拆分" }]), ["拆分", "节奏", "调色"]);
});
