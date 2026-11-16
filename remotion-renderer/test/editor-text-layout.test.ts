import assert from "node:assert/strict";
import { test } from "node:test";
import { PAGE_BREAK, insertPageBreak, layoutProblems, withLayoutPart } from "../editor/src/textLayout.ts";
import { mergeKey, record, undo, type Merge, type Undoable } from "../editor/src/history.ts";
import { rememberClips, withNarration, type ListenClip } from "../editor/src/narration.ts";
import { heardPages, pagePreview, scenePageTexts } from "../src/textPages.ts";
import type { Scene, TextLayout, VideoTimeline } from "../src/types/timeline.ts";

const LAND = { width: 1920, height: 1080 };
const TEXT = "第一句话在这里。第二句话也在这里。第三句话就在这里！";

const scene = (p: Partial<Scene>): Scene => ({
  id: "s",
  duration_ms: 9000,
  text: TEXT,
  highlights: [],
  image: null,
  motion: "static",
  audio: null,
  ...p,
});

test("layout parts are set and removed; clearing everything removes the key", () => {
  let l = withLayoutPart(null, "size", 120);
  assert.deepEqual(l, { size: 120 });
  l = withLayoutPart(l, "position", "top");
  l = withLayoutPart(l, "paging", "auto");
  assert.deepEqual(l, { size: 120, position: "top", paging: "auto" });
  l = withLayoutPart(l, "size", undefined);
  l = withLayoutPart(l, "position", undefined);
  assert.deepEqual(l, { paging: "auto" });
  assert.equal(withLayoutPart(l, "paging", undefined), null, "nothing left: the scene saves no text_layout");
  // The saved scene JSON goes back to exactly what it was.
  const before = scene({});
  const after: Scene = { ...before, text_layout: withLayoutPart(withLayoutPart(null, "size", 100), "size", undefined) ?? undefined };
  assert.equal(JSON.stringify(after), JSON.stringify(before));
});

test("layout problems mirror the DSL's", () => {
  assert.deepEqual(layoutProblems(null), []);
  assert.deepEqual(layoutProblems({ size: 92, position: "middle", paging: "manual" }), []);
  assert.equal(layoutProblems({ size: 200 }).length, 1);
  assert.equal(layoutProblems({ position: "left" as TextLayout["position"] }).length, 1);
  assert.equal(layoutProblems({ paging: "sentences" as TextLayout["paging"] }).length, 1);
});

test("a page break is a blank line at the caret", () => {
  const r = insertPageBreak("第一页。 第二页。", 4);
  assert.equal(r.text, `第一页。${PAGE_BREAK}第二页。`);
  assert.equal(r.caret, 6);
  assert.deepEqual(scenePageTexts({ text: r.text, text_layout: { paging: "manual" } }, null, LAND), ["第一页。", "第二页。"]);
  assert.equal(insertPageBreak("abc", 99).text, `abc${PAGE_BREAK}`);
});

test("the page list: by characters over the scene, over a heard clip once listened to", () => {
  const s = scene({ text_layout: { paging: "auto", size: 160 } });
  const plain = pagePreview(s, null, LAND);
  assert.equal(plain.length, 3);
  assert.equal(plain[0].startMs, 0);
  assert.equal(plain[2].endMs, 9000);
  const heard = pagePreview(s, null, LAND, 4600);
  // 7 / 8 / 8 characters over 4600 ms; the scene is not stretched (9000 > 4600 + 300).
  assert.deepEqual(
    heard.map((p) => p.startMs),
    [0, Math.floor((4600 * 7) / 23), Math.floor((4600 * 15) / 23)],
  );
  assert.equal(pagePreview(scene({}), null, LAND).length, 0, "unpaged: no list");
});

test("Listen to all: heard paged scenes carry pages timed over their clip, the saved timeline is untouched", () => {
  const narrated = scene({ text_layout: { paging: "auto", size: 160 }, narration: { voice: null } });
  const t = { fps: 30, resolution: LAND, scenes: [narrated], subtitles: [] } as unknown as VideoTimeline;
  const clip: ListenClip = {
    scene_id: "s",
    index: 0,
    text: TEXT,
    voice: null,
    source: "generated/tts/s.wav",
    speech_ms: 10_000,
    scene_ms: 9000,
    render_ms: 10_300,
    cached: true,
  };
  const heard = rememberClips({}, "say", [clip]);
  const saved = JSON.stringify(t);
  const p = withNarration(t, heard, "say", (sc, ms) => heardPages(sc, t.brand, t.resolution, ms));
  assert.equal(JSON.stringify(t), saved);
  assert.deepEqual(p.missing, []);
  const out = p.timeline.scenes[0];
  assert.equal(out.duration_ms, 10_300);
  assert.deepEqual(
    out.text_pages?.map((x) => x.start_ms),
    [0, Math.floor((10_000 * 7) / 23), Math.floor((10_000 * 15) / 23)],
  );
  // Without a pager (or for a scene saying a different line) no pages are set.
  assert.equal(withNarration(t, heard, "say").timeline.scenes[0].text_pages, undefined);
  const other = { ...t, scenes: [{ ...narrated, narration: { voice: null, text: "Something else" } }] };
  const otherClip = { ...clip, text: "Something else" };
  const p2 = withNarration(other, rememberClips({}, "say", [otherClip]), "say", (sc, ms) => heardPages(sc, null, LAND, ms));
  assert.equal(p2.timeline.scenes[0].text_pages, undefined);
});

test("a size drag is one undo step; a position choice is another", () => {
  type S = Undoable<Scene>;
  const same = (a: Scene, b: Scene) => JSON.stringify(a) === JSON.stringify(b);
  const edit = (st: S, l: TextLayout | null, merge: Merge): S =>
    record(st, { ...st, timeline: { ...st.timeline, text_layout: l ?? undefined } }, merge, same);
  let s: S = { timeline: scene({}), history: [], future: [], merge: null };
  const key = mergeKey(7, { type: "patchScene", index: 0 });
  for (const [i, v] of [94, 100, 110, 120].entries()) s = edit(s, withLayoutPart(s.timeline.text_layout, "size", v), { key, at: i * 50 });
  assert.equal(s.history.length, 1);
  s = edit(s, withLayoutPart(s.timeline.text_layout, "position", "top"), null);
  assert.equal(s.history.length, 2);
  s = undo(s)!;
  assert.deepEqual(s.timeline.text_layout, { size: 120 });
  s = undo(s)!;
  assert.equal(s.timeline.text_layout, undefined);
  assert.equal(JSON.stringify(s.timeline), JSON.stringify(scene({})));
});

// ---- Free position and keywords (cjk-highlight)
import { DEFAULT_POINT, KEYWORD_YELLOW, addHighlight, removeHighlight, withPointAxis } from "../editor/src/textLayout.ts";

test("free point: axes are rounded and clamped, x 0.5 is left out, problems are named", () => {
  assert.deepEqual(DEFAULT_POINT, { y: 0.6 });
  assert.deepEqual(withPointAxis(null, "y", 0.6234), { y: 0.62 });
  assert.deepEqual(withPointAxis({ y: 0.6 }, "x", 0.3), { y: 0.6, x: 0.3 });
  assert.deepEqual(withPointAxis({ y: 0.6, x: 0.3 }, "x", 0.5), { y: 0.6 });
  assert.deepEqual(withPointAxis({ y: 0.6 }, "y", 1.7), { y: 1 });
  assert.deepEqual(layoutProblems({ position: { y: 0.6, x: 0.2 } }), []);
  assert.deepEqual(layoutProblems({ position: { y: 1.4 } }), ["text position y 1.4 must be between 0 (top) and 1 (bottom)"]);
  assert.deepEqual(layoutProblems({ position: { y: 0.5, x: -1 } }), ["text position x -1 must be between 0 (left) and 1 (right)"]);
});

test("keywords: added once (ignoring case), trimmed, removed", () => {
  assert.deepEqual(addHighlight([], " 文字 "), ["文字"]);
  assert.deepEqual(addHighlight(["文字"], "文字"), ["文字"]);
  assert.deepEqual(addHighlight(["AI"], "ai"), ["AI"]);
  assert.deepEqual(addHighlight(["AI"], "  "), ["AI"]);
  assert.deepEqual(removeHighlight(["文字", "视频"], "文字"), ["视频"]);
  assert.equal(KEYWORD_YELLOW, "#f5c400");
});

test("a Y drag is one undo step; a keyword from the selection is one; removing it is one", () => {
  type S = Undoable<Scene>;
  const same = (a: Scene, b: Scene) => JSON.stringify(a) === JSON.stringify(b);
  const edit = (st: S, next: Scene, merge: Merge): S => record(st, { ...st, timeline: next }, merge, same);
  const start = scene({ text: "输入文字就能生成视频" });
  let s: S = { timeline: start, history: [], future: [], merge: null };
  // Choosing 自由位置 is one step; dragging the Y slider (one input, coalesced) is one more.
  s = edit(s, { ...s.timeline, text_layout: withLayoutPart(s.timeline.text_layout, "position", DEFAULT_POINT) ?? undefined }, null);
  const key = mergeKey(9, { type: "patchScene", index: 0 });
  for (const [i, y] of [0.61, 0.63, 0.66].entries()) {
    const p = withPointAxis(s.timeline.text_layout?.position as { y: number }, "y", y);
    s = edit(s, { ...s.timeline, text_layout: withLayoutPart(s.timeline.text_layout, "position", p) ?? undefined }, { key, at: i * 40 });
  }
  assert.equal(s.history.length, 2);
  assert.deepEqual(s.timeline.text_layout, { position: { y: 0.66 } });
  // ★ 高亮选中的字 and the × on its chip: a step each.
  s = edit(s, { ...s.timeline, highlights: addHighlight(s.timeline.highlights, "文字") }, null);
  s = edit(s, { ...s.timeline, highlights: removeHighlight(s.timeline.highlights, "文字") }, null);
  assert.equal(s.history.length, 4);
  s = undo(s)!;
  assert.deepEqual(s.timeline.highlights, ["文字"]);
  s = undo(s)!;
  assert.deepEqual(s.timeline.highlights, []);
  s = undo(s)!;
  assert.deepEqual(s.timeline.text_layout, { position: DEFAULT_POINT });
  s = undo(s)!;
  assert.equal(JSON.stringify(s.timeline), JSON.stringify(start));
});
