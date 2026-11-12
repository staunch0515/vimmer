// Which word of a subtitle cue is being spoken at a frame. Pure and free of Remotion imports so
// it can be unit-tested directly (see test/word-subtitles.test.ts).
//
// Word times follow the DSL rules in rust-core/crates/vimmer-dsl/src/validation.rs: absolute
// milliseconds, inside the cue, in order, never overlapping. Zero-length words are allowed (a
// chunked render clamps words outside the chunk to its edge).

import type { WordTiming } from "../types/timeline";

export type WordState = "spoken" | "current" | "upcoming";

// After a word ends, it stays highlighted for this many frames when nothing follows right away,
// so a short pause does not make the highlight flicker off.
export const WORD_HOLD_FRAMES = 6;

// Same rounding as `msToFrames` in layout.ts and the Rust side (round half up).
const toFrame = (ms: number, fps: number): number => Math.round((ms * fps) / 1000);

// The index of the word being spoken, or -1 when none is: before the first word starts, or
// in a pause longer than WORD_HOLD_FRAMES after a word ends.
export const currentWordIndex = (words: WordTiming[], frame: number, fps: number): number => {
  let active = -1;
  for (let i = 0; i < words.length; i++) {
    if (toFrame(words[i].start_ms, fps) <= frame) {
      active = i;
    } else {
      break;
    }
  }
  if (active < 0) {
    return -1;
  }
  const start = toFrame(words[active].start_ms, fps);
  const end = Math.max(toFrame(words[active].end_ms, fps), start + 1);
  return frame < end + WORD_HOLD_FRAMES ? active : -1;
};

// The state of every word at a frame. Words before the current one (or, in a pause, every word
// already started) are spoken; the rest are upcoming.
export const wordStates = (words: WordTiming[], frame: number, fps: number): WordState[] => {
  const current = currentWordIndex(words, frame, fps);
  return words.map((word, i) => {
    if (i === current) {
      return "current";
    }
    if (current >= 0) {
      return i < current ? "spoken" : "upcoming";
    }
    return toFrame(word.start_ms, fps) <= frame ? "spoken" : "upcoming";
  });
};

// BCP 47 tag for a line, so the browser picks Japanese or Chinese glyph shapes for shared
// ideographs. Any kana means Japanese; other ideographs mean Chinese.
export const cjkLang = (text: string): "ja" | "zh-CN" | undefined => {
  if (/[぀-ヿｦ-ﾟ]/u.test(text)) {
    return "ja";
  }
  if (/[㐀-䶿一-鿿]/u.test(text)) {
    return "zh-CN";
  }
  return undefined;
};
