import assert from "node:assert/strict";
import { test } from "node:test";
import { LINE_BREAK, highlightParts, splitWordsAndBreaks } from "../src/highlight.ts";
import { resolveTextLayout } from "../src/textPages.ts";

const lit = (word: string, hl: string[]) =>
  highlightParts(word, hl)
    .map((p) => (p.highlight ? `[${p.text}]` : p.text))
    .join("");

test("a keyword inside Chinese text lights up on its own", () => {
  assert.equal(lit("输入文字就能生成视频", ["文字"]), "输入[文字]就能生成视频");
  assert.equal(lit("输入文字就能生成视频", ["文字", "视频"]), "输入[文字]就能生成[视频]");
});

test("keyword at the start and at the end, and the whole word", () => {
  assert.equal(lit("文字就能", ["文字"]), "[文字]就能");
  assert.equal(lit("生成视频。", ["视频"]), "生成[视频]。");
  assert.equal(lit("视频", ["视频"]), "[视频]");
});

test("mixed CJK and Latin: Latin keywords need CJK or the word edge on both sides", () => {
  assert.equal(lit("用AI生成", ["AI"]), "用[AI]生成");
  assert.equal(lit("用ai生成", ["AI"]), "用[ai]生成", "case is ignored");
  assert.equal(lit("Vimmer的功能", ["vimmer"]), "[Vimmer]的功能");
  assert.equal(lit("用AIGC生成", ["AI"]), "用AIGC生成", "AI inside AIGC stays plain");
  assert.equal(lit("用AI生成", ["用AI"]), "[用AI]生成");
});

test("overlapping keywords merge into one run", () => {
  assert.equal(lit("输入文字就能", ["文字", "字就"]), "输入[文字就]能");
  assert.equal(lit("输入文字就能", ["输入文字", "文字"]), "[输入文字]就能");
});

test("no match, and Latin words never match a part (old timelines unchanged)", () => {
  assert.deepEqual(highlightParts("输入文字", ["视频"]), [{ text: "输入文字", highlight: false }]);
  assert.deepEqual(highlightParts("concatenate", ["cat"]), [{ text: "concatenate", highlight: false }]);
  assert.deepEqual(highlightParts("AI-powered", ["AI"]), [{ text: "AI-powered", highlight: false }]);
  assert.deepEqual(highlightParts("文字", [" ", ""]), [{ text: "文字", highlight: false }]);
});

test("words and line breaks", () => {
  assert.deepEqual(splitWordsAndBreaks("one two  three"), ["one", "two", "three"], "no break: as splitWords");
  assert.deepEqual(splitWordsAndBreaks("输入文字\n就能生成视频"), ["输入文字", LINE_BREAK, "就能生成视频"]);
  assert.deepEqual(splitWordsAndBreaks("a b\nc d"), ["a", "b", LINE_BREAK, "c", "d"]);
  assert.deepEqual(splitWordsAndBreaks("\na\n\n\nb\n"), ["a", LINE_BREAK, "b"], "runs count once; edges dropped");
  assert.deepEqual(splitWordsAndBreaks(""), []);
});

test("a free point resolves over a brand name, and a name over a brand point", () => {
  const brand = { text_layout: { position: "top" as const } };
  const r = resolveTextLayout({ position: { y: 0.6 } }, brand);
  assert.equal(r.position, "middle");
  assert.deepEqual(r.at, { y: 0.6 });
  const b = resolveTextLayout({ position: "bottom" }, { text_layout: { position: { y: 0.3, x: 0.4 } } });
  assert.equal(b.at, null);
  assert.deepEqual(resolveTextLayout(null, { text_layout: { position: { y: 0.3 } } }).at, { y: 0.3 });
  assert.equal(resolveTextLayout(null, null).at, null);
});
