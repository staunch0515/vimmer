// Pure timing of the typewriter text reveal. No Remotion imports and no runtime imports, so it can
// be unit-tested with Node (test/motion.test.ts).
//
// Contract, mirrored by `typing_units` / `TextLayer::typing_ms` in
// rust-core/crates/vimmer-dsl/src/layer.rs:
// - the text is typed in units: a code point plus everything that attaches to it (combining marks,
//   variation selectors, emoji skin tones, ZWJ sequences, regional-indicator flag pairs). A CJK
//   character, an emoji or an accented letter is one unit; UTF-16 surrogate pairs never split.
// - typing starts on the layer's first frame at `cps` units per second: on layer frame f,
//   floor(f * cps / fps) units are shown (none on the first frame).
// - the cursor is solid while typing and blinks (on/off every BLINK_MS) once the text is complete.

export const DEFAULT_TYPING_CPS = 20;
export const BLINK_MS = 530;

const joinsPrevious = (cp: number): boolean =>
  (cp >= 0x0300 && cp <= 0x036f) ||
  (cp >= 0x1ab0 && cp <= 0x1aff) ||
  (cp >= 0x1dc0 && cp <= 0x1dff) ||
  (cp >= 0x20d0 && cp <= 0x20ff) ||
  (cp >= 0xfe20 && cp <= 0xfe2f) ||
  (cp >= 0xfe00 && cp <= 0xfe0f) ||
  cp === 0x200d ||
  (cp >= 0x1f3fb && cp <= 0x1f3ff) ||
  (cp >= 0xe0020 && cp <= 0xe007f) ||
  (cp >= 0xe0100 && cp <= 0xe01ef);

const isRegionalIndicator = (cp: number): boolean => cp >= 0x1f1e6 && cp <= 0x1f1ff;

// The units a typewriter types one at a time (see the contract above).
export const typingUnits = (text: string): string[] => {
  const out: string[] = [];
  let cur = "";
  let prev: number | null = null;
  let ri = 0;
  for (const ch of text) {
    // for..of iterates by code point, so surrogate pairs stay together.
    const cp = ch.codePointAt(0)!;
    const attach = prev !== null && (joinsPrevious(cp) || prev === 0x200d || (isRegionalIndicator(cp) && ri === 1));
    if (!attach && cur !== "") {
      out.push(cur);
      cur = "";
      ri = 0;
    }
    cur += ch;
    if (isRegionalIndicator(cp)) ri += 1;
    prev = cp;
  }
  if (cur !== "") out.push(cur);
  return out;
};

// Units shown on layer frame `frame` (0 = the layer's first frame).
export const typedCount = (frame: number, fps: number, cps: number, total: number): number =>
  Math.max(0, Math.min(total, Math.floor((frame * cps) / fps)));

// Whether the cursor is drawn on layer frame `frame`.
export const cursorVisible = (frame: number, fps: number, cps: number, total: number): boolean => {
  const doneFrame = Math.ceil((total * fps) / cps);
  if (frame < doneFrame) return true;
  const ms = ((frame - doneFrame) * 1000) / fps;
  return Math.floor(ms / BLINK_MS) % 2 === 0;
};

export type TypedSegment = { text: string; highlight: boolean; shown: string; hidden: string; cursorAfter: boolean };

// Splits `text` into whitespace and word segments (words flagged by `isHighlight`) and says how
// much of each is typed when `count` units are shown. The cursor goes between `shown` and `hidden`
// of the segment marked `cursorAfter` (the last one with typed text; the first when none is).
export const typedSegments = (text: string, count: number, isHighlight: (word: string) => boolean): TypedSegment[] => {
  const parts = text.split(/(\s+)/).filter((p) => p !== "");
  let left = count;
  const out: TypedSegment[] = parts.map((p) => {
    const units = typingUnits(p);
    const n = Math.max(0, Math.min(units.length, left));
    left -= n;
    return {
      text: p,
      highlight: !/^\s+$/.test(p) && isHighlight(p),
      shown: units.slice(0, n).join(""),
      hidden: units.slice(n).join(""),
      cursorAfter: false,
    };
  });
  let at = -1;
  out.forEach((s, i) => {
    if (s.shown !== "") at = i;
  });
  if (out.length > 0) out[Math.max(0, at)].cursorAfter = true;
  return out;
};
