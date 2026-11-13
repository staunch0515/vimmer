// Run with: node --experimental-strip-types --test test/word-subtitles.test.ts
import assert from "node:assert/strict";
import { test } from "node:test";
import {
  WORD_HOLD_FRAMES,
  cjkLang,
  currentWordIndex,
  patchCue,
  SUBTITLE_FADE_FRAMES,
  sceneCutFrames,
  subtitleAt,
  wordStates,
  wordsMatchText,
  type WordState,
} from "../src/subtitles/words.ts";
import type { SubtitleCue, WordTiming } from "../src/types/timeline.ts";

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

// --- Chunk seams -------------------------------------------------------------------------------
// Mirror of `chunk_timeline` in rust-core/crates/vimmer-scheduler/src/plan.rs for one cue: clip
// it to the chunk's frames and rebase every word time by frame, clamped into the clipped cue.
// `chunk_timeline_rebases_words_by_frame` in plan.rs pins the Rust side to the same numbers as
// the seam example below.
type Cue = { start_ms: number; end_ms: number; words: WordTiming[] };
const toFrame = (ms: number, fps: number) => Math.round((ms * fps) / 1000);
const toMs = (frame: number, fps: number) => Math.round((frame * 1000) / fps);
const clipCue = (cue: Cue, chunkStart: number, chunkEnd: number, fps: number): Cue | null => {
  const start = Math.max(toFrame(cue.start_ms, fps), chunkStart);
  const end = Math.min(toFrame(cue.end_ms, fps), chunkEnd);
  if (start >= end) return null;
  const rebase = (ms: number) => toMs(Math.min(Math.max(toFrame(ms, fps), start), end) - chunkStart, fps);
  return {
    start_ms: toMs(start - chunkStart, fps),
    end_ms: toMs(end - chunkStart, fps),
    words: cue.words.map((x) => ({ text: x.text, start_ms: rebase(x.start_ms), end_ms: rebase(x.end_ms) })),
  };
};

type SceneSpec = { duration_ms: number; transition_in?: { kind: "fade"; duration_ms: number } };

// Word states of `cue` at every frame where it is on screen, rendered whole and rendered in
// chunks split at `splits` (scene indices that start a chunk), side by side.
const fullAndChunked = (cue: Cue, scenes: SceneSpec[], splits: number[], fps: number) => {
  const sceneFrames = scenes.map((s) => toFrame(s.duration_ms, fps));
  const sceneStart = (i: number) => sceneFrames.slice(0, i).reduce((a, b) => a + b, 0);
  const full: WordState[][] = [];
  const chunked: WordState[][] = [];
  const bounds = [0, ...splits, scenes.length];
  for (let c = 0; c + 1 < bounds.length; c++) {
    const [first, last] = [bounds[c], bounds[c + 1]];
    const chunkStart = sceneStart(first);
    const chunkEnd = sceneStart(last);
    const clipped = clipCue(cue, chunkStart, chunkEnd, fps);
    if (!clipped) continue;
    const chunkCuts = sceneCutFrames(scenes.slice(first, last), fps);
    const fullCuts = sceneCutFrames(scenes, fps);
    for (let f = toFrame(clipped.start_ms, fps); f < toFrame(clipped.end_ms, fps); f++) {
      full.push(wordStates(cue.words, f + chunkStart, fps, fullCuts));
      chunked.push(wordStates(clipped.words, f, fps, chunkCuts));
    }
  }
  return { full, chunked };
};

test("a long pause before a chunk seam does not light a spoken word again", () => {
  // Two 3 s scenes at 30 fps; the seam is frame 90. "two " ends at frame 75, 15 frames before it.
  const cue: Cue = {
    start_ms: 1000,
    end_ms: 5000,
    words: [w("one ", 1000, 1400), w("two ", 1400, 2500), w("three ", 3600, 4200), w("four", 4200, 4800)],
  };
  const scenes = [{ duration_ms: 3000 }, { duration_ms: 3000 }];
  const clipped = clipCue(cue, 90, 180, 30)!;
  assert.deepEqual(
    clipped.words.map((x) => [x.start_ms, x.end_ms]),
    [[0, 0], [0, 0], [600, 1200], [1200, 1800]],
  );
  const cuts = sceneCutFrames(scenes.slice(1), 30);
  // Before the fix, frames 0-6 of the second chunk showed "two " as current again.
  for (let f = 0; f <= WORD_HOLD_FRAMES; f++) {
    assert.deepEqual(wordStates(clipped.words, f, 30, cuts), ["spoken", "spoken", "upcoming", "upcoming"]);
  }
  const { full, chunked } = fullAndChunked(cue, scenes, [1], 30);
  assert.equal(full.length, 120);
  assert.deepEqual(chunked, full);
});

test("a word ending just before or at the seam: chunked and full agree on every frame", () => {
  const scenes = [{ duration_ms: 3000 }, { duration_ms: 3000 }];
  for (const end of [2800, 2900, 2967, 3000, 3033, 3100]) {
    const cue: Cue = {
      start_ms: 1000,
      end_ms: 5000,
      words: [w("one ", 1000, 1400), w("two ", 1400, end), w("three", 3600, 4200)],
    };
    const { full, chunked } = fullAndChunked(cue, scenes, [1], 30);
    assert.deepEqual(chunked, full, `"two " ends at ${end} ms`);
  }
});

test("chunked and full renders agree on every frame (random cues and seams)", () => {
  // Small deterministic generator, so a failure is reproducible.
  let seed = 12345;
  const rand = (n: number) => {
    seed = (seed * 1103515245 + 12345) % 2147483648;
    return seed % n;
  };
  for (let round = 0; round < 400; round++) {
    const fps = [24, 25, 30, 60][rand(4)];
    const scenes: SceneSpec[] = [];
    const count = 2 + rand(4);
    for (let i = 0; i < count; i++) {
      const scene: SceneSpec = { duration_ms: 400 + rand(2500) };
      if (i > 0 && rand(4) === 0) scene.transition_in = { kind: "fade", duration_ms: 200 };
      scenes.push(scene);
    }
    const total = scenes.reduce((a, s) => a + s.duration_ms, 0);
    const cueStart = rand(Math.floor(total / 2));
    const cueEnd = cueStart + 200 + rand(total - cueStart - 100);
    const words: WordTiming[] = [];
    let t = cueStart;
    while (t < cueEnd && words.length < 12) {
      const start = Math.min(t + (rand(3) === 0 ? rand(900) : 0), cueEnd);
      const end = Math.min(start + rand(500), cueEnd);
      words.push(w(`w${words.length} `, start, end));
      t = end;
    }
    // Chunks split only at scenes without a transition, like plan_chunks.
    const splits = scenes.map((s, i) => i).filter((i) => i > 0 && !scenes[i].transition_in && rand(2) === 0);
    const cue: Cue = { start_ms: cueStart, end_ms: cueEnd, words };
    const { full, chunked } = fullAndChunked(cue, scenes, splits, fps);
    assert.deepEqual(chunked, full, JSON.stringify({ fps, scenes, splits, cue }));
  }
});

// --- Fades at chunk seams ----------------------------------------------------------------------
// A chunk render gets the full-timeline cues that overlap it and its start frame (`chunk_props` in
// plan.rs); the Subtitles component adds the start frame to the frame and to the chunk's own cuts.
const chunkedFrames = (cues: SubtitleCue[], scenes: SceneSpec[], splits: number[], fps: number) => {
  const sceneFrames = scenes.map((s) => toFrame(s.duration_ms, fps));
  const sceneStart = (i: number) => sceneFrames.slice(0, i).reduce((a, b) => a + b, 0);
  const fullCuts = sceneCutFrames(scenes, fps);
  const full = [];
  const chunked = [];
  const bounds = [0, ...splits, scenes.length];
  for (let c = 0; c + 1 < bounds.length; c++) {
    const chunkStart = sceneStart(bounds[c]);
    const chunkEnd = sceneStart(bounds[c + 1]);
    const overlapping = cues.filter(
      (q) => Math.max(toFrame(q.start_ms, fps), chunkStart) < Math.min(toFrame(q.end_ms, fps), chunkEnd),
    );
    const cuts = sceneCutFrames(scenes.slice(bounds[c], bounds[c + 1]), fps).map((x) => x + chunkStart);
    for (let f = 0; f < chunkEnd - chunkStart; f++) {
      full.push(subtitleAt(cues, f + chunkStart, fps, fullCuts));
      chunked.push(subtitleAt(overlapping, f + chunkStart, fps, cuts));
    }
  }
  return { full, chunked };
};

test("a cue straddling a chunk seam does not fade at the seam", () => {
  // Two 3 s scenes at 30 fps, seam at frame 90; the cue runs from frame 60 to frame 150.
  const cue: SubtitleCue = {
    start_ms: 2000,
    end_ms: 5000,
    text: "across the seam",
    words: [w("across ", 2000, 2800), w("the ", 2800, 3100), w("seam", 3300, 4500)],
  };
  const scenes = [{ duration_ms: 3000 }, { duration_ms: 3000 }];
  const { full, chunked } = chunkedFrames([cue], scenes, [1], 30);
  assert.deepEqual(chunked, full);
  for (let f = 82; f < 98; f++) {
    assert.equal(chunked[f]?.opacity, 1, `frame ${f} is fully opaque`);
  }
  // The real edges still fade.
  assert.equal(chunked[60]?.opacity, 0);
  assert.ok(Math.abs(chunked[149]!.opacity - 1 / SUBTITLE_FADE_FRAMES) < 1e-9);
  // The old self-contained chunks (cue clipped to each chunk) blinked: faded out to 1/6 at the
  // last frame of the first chunk and in from 0 at the first frame of the second.
  const head = { ...cue, start_ms: 2000, end_ms: 3000, words: [] };
  const tail = { ...cue, start_ms: 0, end_ms: 2000, words: [] };
  assert.ok(Math.abs(subtitleAt([head], 89, 30)!.opacity - 1 / SUBTITLE_FADE_FRAMES) < 1e-9);
  assert.equal(subtitleAt([tail], 0, 30)?.opacity, 0);
});

test("chunked and full renders draw the same subtitle on every frame (random cues and seams)", () => {
  let seed = 777;
  const rand = (n: number) => {
    seed = (seed * 1103515245 + 12345) % 2147483648;
    return seed % n;
  };
  for (let round = 0; round < 300; round++) {
    const fps = [24, 25, 30, 60][rand(4)];
    const scenes: SceneSpec[] = [];
    const count = 2 + rand(4);
    for (let i = 0; i < count; i++) {
      const scene: SceneSpec = { duration_ms: 400 + rand(2500) };
      if (i > 0 && rand(4) === 0) scene.transition_in = { kind: "fade", duration_ms: 200 };
      scenes.push(scene);
    }
    const total = scenes.reduce((a, s) => a + s.duration_ms, 0);
    const cues: SubtitleCue[] = [];
    let t = rand(500);
    while (t + 100 < total) {
      const end = Math.min(t + 100 + rand(3000), total);
      const words: WordTiming[] = [];
      let wt = t;
      while (wt < end && words.length < 8) {
        const ws = Math.min(wt + (rand(3) === 0 ? rand(600) : 0), end);
        const we = Math.min(ws + rand(400), end);
        words.push(w(`w${words.length} `, ws, we));
        wt = we;
      }
      cues.push({ start_ms: t, end_ms: end, text: words.map((x) => x.text).join(""), words });
      t = end + rand(3) * 200;
    }
    const splits = scenes.map((s, i) => i).filter((i) => i > 0 && !scenes[i].transition_in && rand(2) === 0);
    const { full, chunked } = chunkedFrames(cues, scenes, splits, fps);
    assert.deepEqual(chunked, full, JSON.stringify({ fps, scenes, splits, cues }));
  }
});

test("a highlight stops at a scene cut but not inside a scene", () => {
  // "big " ends at frame 48; the next scene starts at frame 50.
  assert.equal(currentWordIndex(words, 49, 30, [0, 50]), 1);
  assert.equal(currentWordIndex(words, 50, 30, [0, 50]), -1);
  // A cut far away changes nothing.
  assert.equal(currentWordIndex(words, 53, 30, [0, 300]), 1);
  assert.deepEqual(sceneCutFrames([{ duration_ms: 1000 }, { duration_ms: 500, transition_in: { kind: "fade", duration_ms: 100 } }, { duration_ms: 1000 }], 30), [0, 45]);
});

test("words that no longer spell the cue text are detected", () => {
  const timed = [w("Hello ", 0, 100), w("world.", 100, 200)];
  assert.equal(wordsMatchText("Hello world.", timed), true);
  assert.equal(wordsMatchText(" Hello  world. ", timed), true, "whitespace is ignored");
  assert.equal(wordsMatchText("Hello there.", timed), false);
  assert.equal(wordsMatchText("Hello world!", timed), false, "punctuation counts");
  assert.equal(wordsMatchText("你好世界", [w("你", 0, 1), w("好", 1, 2), w("世", 2, 3), w("界", 3, 4)]), true);
});

test("editing a cue's text or times drops its words, other edits keep them", () => {
  const cue = { start_ms: 0, end_ms: 1000, text: "Hello world", words: [w("Hello ", 0, 400), w("world", 400, 900)] };
  assert.equal("words" in patchCue(cue, { text: "Hello there" }), false);
  assert.equal("words" in patchCue(cue, { start_ms: 100 }), false);
  assert.equal("words" in patchCue(cue, { end_ms: 800 }), false);
  assert.deepEqual(patchCue(cue, { text: "Hello world" }), cue, "same value: nothing changes");
  const retimed = [w("Hi", 0, 300)];
  assert.deepEqual(patchCue(cue, { text: "Hi", words: retimed }).words, retimed);
  const plain = { start_ms: 0, end_ms: 1000, text: "plain" };
  assert.deepEqual(patchCue(plain, { text: "still plain" }), { start_ms: 0, end_ms: 1000, text: "still plain" });
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
