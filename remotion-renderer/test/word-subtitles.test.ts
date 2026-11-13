// Run with: node --experimental-strip-types --test test/word-subtitles.test.ts
import assert from "node:assert/strict";
import { test } from "node:test";
import { WORD_HOLD_FRAMES, cjkLang, currentWordIndex, wordStates } from "../src/subtitles/words.ts";
import type { WordTiming } from "../src/types/timeline.ts";

const w = (text: string, start_ms: number, end_ms: number): WordTiming => ({ text, start_ms, end_ms });
// 30 fps: 100 ms = 3 frames.
const words = [w("Hello ", 1000, 1300), w("big ", 1300, 1600), w("world", 2000, 2400)];

test("nothing is current before the first word", () => {
  assert.equal(currentWordIndex(words, 0, 30), -1);
  assert.deepEqual(wordStates(words, 29, 30), ["upcoming", "upcoming", "upcoming"]);
});

test("the word whose time has come is current, earlier ones are spoken", () => {
  assert.deepEqual(wordStates(words, 30, 30), ["current", "upcoming", "upcoming"]);
  assert.deepEqual(wordStates(words, 39, 30), ["spoken", "current", "upcoming"]);
  assert.deepEqual(wordStates(words, 60, 30), ["spoken", "spoken", "current"]);
});

test("a short pause keeps the highlight, a long one drops it", () => {
  // "big " ends at frame 48; "world" starts at frame 60.
  assert.equal(currentWordIndex(words, 48 + WORD_HOLD_FRAMES - 1, 30), 1);
  assert.equal(currentWordIndex(words, 48 + WORD_HOLD_FRAMES, 30), -1);
  assert.deepEqual(wordStates(words, 48 + WORD_HOLD_FRAMES, 30), ["spoken", "spoken", "upcoming"]);
  // After the last word the line reads as fully spoken.
  assert.deepEqual(wordStates(words, 200, 30), ["spoken", "spoken", "spoken"]);
});

test("zero-length words at chunk edges", () => {
  // Second chunk of a line: the first character was said before the chunk began.
  const tail = [w("一", 0, 0), w("二", 0, 200), w("三", 400, 1000)];
  assert.deepEqual(wordStates(tail, 0, 30), ["spoken", "current", "upcoming"]);
  // First chunk: later characters wait at the chunk end and never start inside it.
  const head = [w("一", 2000, 2600), w("二", 2600, 3000), w("三", 3000, 3000)];
  assert.deepEqual(wordStates(head, 89, 30), ["spoken", "current", "upcoming"]);
});

test("empty word list", () => {
  assert.equal(currentWordIndex([], 10, 30), -1);
  assert.deepEqual(wordStates([], 10, 30), []);
});

test("language tag for CJK lines", () => {
  assert.equal(cjkLang("今日はいい天気"), "ja");
  assert.equal(cjkLang("西安钟楼"), "zh-CN");
  assert.equal(cjkLang("Hello"), undefined);
});
