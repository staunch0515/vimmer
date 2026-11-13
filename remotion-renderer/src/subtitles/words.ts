// Which word of a subtitle cue is being spoken at a frame. Pure and free of Remotion imports so
// it can be unit-tested directly (see test/word-subtitles.test.ts).
//
// Word times follow the DSL rules in rust-core/crates/vimmer-dsl/src/validation.rs: absolute
// milliseconds, inside the cue, in order, never overlapping. Zero-length words are allowed (a
// chunked render clamps words outside the chunk to its edge).

import type { Scene, SubtitleCue, WordTiming } from "../types/timeline";

export type WordState = "spoken" | "current" | "upcoming";

// After a word ends, it stays highlighted for this many frames when nothing follows right away,
// so a short pause does not make the highlight flicker off.
export const WORD_HOLD_FRAMES = 6;

// Same rounding as `msToFrames` in layout.ts and the Rust side (round half up).
const toFrame = (ms: number, fps: number): number => Math.round((ms * fps) / 1000);

// Frames where a word's highlight stops (see currentWordIndex): the start of every scene that
// does not open with a transition, scene 0 included. Chunked renders only split there (a
// transition always stays in the chunk of the scene before it), so a chunk's first frame is
// always one of these, in the chunk and in the full timeline alike. Same rounding as layout.ts.
export const sceneCutFrames = (scenes: readonly Pick<Scene, "duration_ms" | "transition_in">[], fps: number): number[] => {
  const cuts: number[] = [];
  let cursor = 0;
  for (const scene of scenes) {
    if (!scene.transition_in) {
      cuts.push(cursor);
    }
    cursor += toFrame(scene.duration_ms, fps);
  }
  return cuts;
};

// The first scene start at or after `frame`, or Infinity. `sceneStarts` is sorted.
const nextCut = (sceneStarts: readonly number[], frame: number): number => {
  for (const start of sceneStarts) {
    if (start >= frame) {
      return start;
    }
  }
  return Infinity;
};

// The index of the word being spoken, or -1 when none is: before the first word starts, or
// in a pause longer than WORD_HOLD_FRAMES after a word ends.
//
// `sceneStarts` (from sceneCutFrames, sorted) stops the highlight at a scene cut: a word that
// ends at or before such a frame is never highlighted past it, neither its minimum frame nor its
// hold. A chunked render clamps the words said before the chunk to zero length at its first
// frame, which is always a cut. With the cut, those words read as spoken in every frame exactly
// as in a full render, instead of lighting up again for up to WORD_HOLD_FRAMES + 1 frames (see
// test/word-subtitles.test.ts). Without `sceneStarts` nothing is cut.
export const currentWordIndex = (
  words: WordTiming[],
  frame: number,
  fps: number,
  sceneStarts: readonly number[] = [],
): number => {
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
  const endFrame = toFrame(words[active].end_ms, fps);
  const end = Math.max(endFrame, start + 1);
  const until = Math.min(end + WORD_HOLD_FRAMES, nextCut(sceneStarts, endFrame));
  return frame < until ? active : -1;
};

// The state of every word at a frame. Words before the current one (or, in a pause, every word
// already started) are spoken; the rest are upcoming.
export const wordStates = (
  words: WordTiming[],
  frame: number,
  fps: number,
  sceneStarts: readonly number[] = [],
): WordState[] => {
  const current = currentWordIndex(words, frame, fps, sceneStarts);
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

// Whether a cue's words still spell its text, ignoring whitespace. A cue edited after its words
// were timed (new text, old words) fails this and is drawn as one plain line instead of showing
// stale words. Same rule as `words_match_text` in vimmer-scheduler/src/wordsubs.rs.
export const wordsMatchText = (text: string, words: readonly WordTiming[]): boolean => {
  const squash = (s: string) => s.replace(/\s+/gu, "");
  return squash(words.map((w) => w.text).join("")) === squash(text);
};

// Applies an edit to a cue. Word timing belongs to one text at one time, so changing the text or
// either end drops it (otherwise the old words stay on screen, or sit outside the cue and fail
// validation); the next render times the cue again from the narration. An edit that sets
// `words` itself, or changes nothing, keeps them. Used by the editor's reducer.
export const patchCue = (cue: SubtitleCue, patch: Partial<SubtitleCue>): SubtitleCue => {
  const next = { ...cue, ...patch };
  const changed = next.text !== cue.text || next.start_ms !== cue.start_ms || next.end_ms !== cue.end_ms;
  if (!changed || next.words === undefined || "words" in patch) {
    return next;
  }
  const { words: _stale, ...rest } = next;
  return rest;
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
