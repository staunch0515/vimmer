// Text layout of the scene text: size, position and paging (文字分页). Mirrors
// rust-core/crates/vimmer-dsl/src/text_layout.rs (see its module docs); both are checked against
// rust-core/crates/vimmer-dsl/tests/text_pages_vectors.json. Pure, no runtime imports: the Node
// tests import it.
//
// A render gets `scene.text_pages` from the Rust core (pages timed on the narration's words).
// Without them (the editor preview) the pages are split here and timed by characters over the
// scene, exactly as the render times a scene without narration.

import type { Brand, Paging, Resolution, Scene, TextLayout, TextPage, TextPoint, TextPosition } from "./types/timeline";

export const DEFAULT_TEXT_PX = 92;
export const MIN_TEXT_PX = 40;
export const MAX_TEXT_PX = 160;
export const PAGE_MAX_LINES = 3;
export const PAGE_FILL = 0.85;
export const PAGE_SLACK = 1.15;
export const SHORT_PAGE_SHARE = 0.3;
export const MIN_PAGE_MS = 1200;
export const PAGE_LEAD_MS = 150;
export const MIN_PAGE_GAP_MS = 100;
export const PADDING_FRAC = 0.08;
export const BOTTOM_INSET_PX = 260;
export const MAX_WIDTH_FRAC = 0.88;
export const LINE_HEIGHT = 1.2;
export const LOGO_GAP_FRAC = 0.02;
/** Frames the outgoing page takes to fade out while the next one reveals. */
export const PAGE_FADE_FRAMES = 8;

// `position` is "middle" for a free point; `at` is the point, which wins.
export type ResolvedTextLayout = { size: number; position: TextPosition; at: TextPoint | null; paging: Paging };

/** Whether a text_layout.position is a free point rather than a named place. */
export const isTextPoint = (p: unknown): p is TextPoint => typeof p === "object" && p !== null && typeof (p as TextPoint).y === "number";

/** Each part of the scene's layout, else the brand's, else the built-in one. */
export const resolveTextLayout = (own: TextLayout | null | undefined, brand: Brand | null | undefined): ResolvedTextLayout => {
  const b = brand?.text_layout;
  const place = own?.position ?? b?.position ?? "bottom";
  return {
    size: own?.size ?? b?.size ?? DEFAULT_TEXT_PX,
    position: isTextPoint(place) ? "middle" : place,
    at: isTextPoint(place) ? place : null,
    paging: own?.paging ?? b?.paging ?? "off",
  };
};

/** Whether neither the scene nor the brand sets any part (the text renders exactly as before). */
export const isDefaultLayout = (own: TextLayout | null | undefined, brand: Brand | null | undefined): boolean => {
  const set = (l: TextLayout | null | undefined) => l != null && (l.size != null || l.position != null || l.paging != null);
  return !set(own) && !set(brand?.text_layout);
};

/** px at 1080p -> px on the frame (shorter side / 1080, as text_style.rs `style_scale`). */
export const fontPx = (size: number, width: number, height: number): number => (Math.fround(size) * Math.min(width, height)) / 1080;

/** Top padding: 8% of the width, or below a top-corner logo shown on the scene for top/middle. */
export const topInsetPx = (
  position: TextPosition,
  brand: Brand | null | undefined,
  sceneId: string,
  width: number,
  height: number,
): number => {
  const pad = width * PADDING_FRAC;
  if (position === "bottom") return pad;
  const logo = brand?.logo;
  if (!logo) return pad;
  const corner = logo.corner ?? "top_right";
  if ((logo.hide_on ?? []).includes(sceneId) || !corner.startsWith("top")) return pad;
  // f32 values in Rust: round-trip through Math.fround so both sides compute the same px.
  const below = (Math.fround(logo.margin ?? 0.05) + Math.fround(logo.size ?? 0.08) + LOGO_GAP_FRAC) * height;
  return Math.max(pad, below);
};

// ---- Text measure

const WIDE: Array<[number, number]> = [
  [0x1100, 0x115f], [0x2e80, 0x303e], [0x3040, 0x33ff], [0x3400, 0x4dbf], [0x4e00, 0x9fff],
  [0xa960, 0xa97f], [0xac00, 0xd7a3], [0xf900, 0xfaff], [0xfe30, 0xfe4f], [0xff00, 0xff60],
  [0xffe0, 0xffe6], [0x20000, 0x3fffd],
];

export const isWide = (c: string): boolean => {
  const cp = c.codePointAt(0) ?? 0;
  return WIDE.some(([a, b]) => cp >= a && cp <= b);
};

const isSpace = (c: string): boolean => /\s/u.test(c);

export const textEm = (text: string): number => {
  let sum = 0;
  for (const c of text) sum += isSpace(c) ? 0.28 : isWide(c) ? 1 : 0.55;
  return sum;
};

/** Letters and digits (Rust `char::is_alphanumeric`). */
export const keyLen = (text: string): number => {
  let n = 0;
  for (const c of text) if (/[\p{Alphabetic}\p{N}]/u.test(c)) n += 1;
  return n;
};

export const pageCapEm = (fontPxValue: number, width: number, height: number): number => {
  const lineWidth = width * (1 - 2 * PADDING_FRAC) * MAX_WIDTH_FRAC;
  const avail = height - width * PADDING_FRAC - BOTTOM_INSET_PX;
  const lines = Math.min(Math.max(Math.floor(avail / (fontPxValue * LINE_HEIGHT)), 1), PAGE_MAX_LINES);
  return (lineWidth / fontPxValue) * lines * PAGE_FILL;
};

// ---- Splitting (code-point ranges)

const TERMINATORS = new Set(["。", "！", "？", "｡", "．", "!", "?", ".", "…"]);
const CJK_TERMINATORS = new Set(["。", "！", "？", "｡", "．"]);
const CLOSERS = new Set(['"', "'", "”", "’", ")", "）", "」", "』", "】", "》", "〉", "]"]);
const CLAUSE_ENDS = new Set(["，", "、", "；", "：", ",", ";", ":"]);
const sticksToPrevious = (c: string) => CLOSERS.has(c) || TERMINATORS.has(c) || CLAUSE_ENDS.has(c);

type Range = [number, number];

const trimmed = (chars: string[], a: number, b: number): Range | null => {
  while (a < b && isSpace(chars[a])) a += 1;
  while (b > a && isSpace(chars[b - 1])) b -= 1;
  return a < b ? [a, b] : null;
};

const freeAfter = (next: string | undefined) => next === undefined || isSpace(next) || isWide(next);

const splitAfter = (
  chars: string[],
  r: Range,
  lines: boolean,
  ends: (c: string, next: string | undefined) => boolean,
  starts: (c: string) => boolean,
): Range[] => {
  const out: Range[] = [];
  const push = (x: Range | null) => {
    if (x) out.push(x);
  };
  let start = r[0];
  let i = r[0];
  while (i < r[1]) {
    const c = chars[i];
    if (lines && c === "\n") {
      push(trimmed(chars, start, i));
      start = i + 1;
      i += 1;
      continue;
    }
    if (starts(c)) {
      let j = i + 1;
      while (j < r[1] && (starts(chars[j]) || CLOSERS.has(chars[j]))) j += 1;
      const next = j < r[1] ? chars[j] : undefined;
      if (ends(c, next)) {
        push(trimmed(chars, start, j));
        start = j;
      }
      i = j;
      continue;
    }
    i += 1;
  }
  push(trimmed(chars, start, r[1]));
  return out;
};

const sentences = (chars: string[]): Range[] =>
  splitAfter(chars, [0, chars.length], true, (c, next) => CJK_TERMINATORS.has(c) || freeAfter(next), (c) => TERMINATORS.has(c));

const isAscii = (c: string) => (c.codePointAt(0) ?? 0) < 0x80;

const clauses = (chars: string[], r: Range): Range[] =>
  splitAfter(chars, r, false, (c, next) => !isAscii(c) || freeAfter(next), (c) => CLAUSE_ENDS.has(c));

const words = (chars: string[], r: Range): Range[] => {
  const out: Range[] = [];
  let i = r[0];
  while (i < r[1]) {
    const start = i;
    let j = i + 1;
    if (isWide(chars[i])) {
      while (j < r[1] && sticksToPrevious(chars[j])) j += 1;
    } else {
      while (j < r[1] && !isSpace(chars[j]) && !isWide(chars[j])) j += 1;
      while (j < r[1] && isWide(chars[j]) && sticksToPrevious(chars[j])) j += 1;
    }
    while (j < r[1] && isSpace(chars[j])) j += 1;
    const t = trimmed(chars, start, j);
    if (t) out.push(t);
    i = j;
  }
  return out;
};

const emOf = (chars: string[], r: Range) => textEm(chars.slice(r[0], r[1]).join(""));

const pack = (chars: string[], pieces: Range[], cap: number): Range[] => {
  const out: Range[] = [];
  for (const p of pieces) {
    const last = out[out.length - 1];
    if (last && emOf(chars, [last[0], p[1]]) <= cap) last[1] = p[1];
    else out.push([p[0], p[1]]);
  }
  return out;
};

const mergeShort = (chars: string[], pages: Range[], cap: number): Range[] => {
  const short = cap * SHORT_PAGE_SHARE;
  let i = 0;
  while (i + 1 < pages.length) {
    if (emOf(chars, pages[i]) < short && emOf(chars, [pages[i][0], pages[i + 1][1]]) <= cap) {
      pages[i][1] = pages[i + 1][1];
      pages.splice(i + 1, 1);
    } else {
      i += 1;
    }
  }
  i = pages.length;
  while (i > 1) {
    i -= 1;
    if (emOf(chars, pages[i]) < short && emOf(chars, [pages[i - 1][0], pages[i][1]]) <= cap) {
      pages[i - 1][1] = pages[i][1];
      pages.splice(i, 1);
    }
  }
  return pages;
};

export const autoPages = (text: string, capEm: number): string[] => {
  const chars = Array.from(text);
  const whole = capEm * PAGE_SLACK;
  const pages: Range[] = [];
  for (const s of sentences(chars)) {
    if (emOf(chars, s) <= whole) {
      pages.push(s);
      continue;
    }
    for (const c of pack(chars, clauses(chars, s), capEm)) {
      if (emOf(chars, c) <= whole) pages.push(c);
      else pages.push(...pack(chars, words(chars, c), capEm));
    }
  }
  return mergeShort(chars, pages, capEm).map(([a, b]) => chars.slice(a, b).join(""));
};

export const manualPages = (text: string): string[] => {
  const out: string[] = [];
  let cur: string[] = [];
  for (const line of text.split("\n")) {
    if (line.trim() === "") {
      if (cur.length > 0) {
        out.push(cur.join("\n").trim());
        cur = [];
      }
    } else {
      cur.push(line);
    }
  }
  if (cur.length > 0) out.push(cur.join("\n").trim());
  return out;
};

export const splitPages = (text: string, paging: Paging, fontPxValue: number, width: number, height: number): string[] => {
  if (text.trim() === "") return [];
  const pages =
    paging === "auto" ? autoPages(text, pageCapEm(fontPxValue, width, height)) : paging === "manual" ? manualPages(text) : [text];
  return pages.length <= 1 ? [text] : pages;
};

// ---- Timing

export const proportionalStarts = (pages: string[], fromMs: number, spanMs: number, minMs: number): number[] => {
  const n = pages.length;
  const weights = pages.map((p) => Math.max(keyLen(p), 1));
  const total = weights.reduce((a, b) => a + b, 0);
  const out: number[] = [];
  let cum = 0;
  weights.forEach((w, k) => {
    out.push(minMs * n <= spanMs ? fromMs + minMs * k + Math.floor(((spanMs - minMs * n) * cum) / total) : fromMs + Math.floor((spanMs * k) / n));
    cum += w;
  });
  return out;
};

export const monotonic = (starts: number[]): number[] => {
  const out = [...starts];
  for (let k = 0; k < out.length; k++) out[k] = k === 0 ? 0 : Math.max(out[k], out[k - 1] + MIN_PAGE_GAP_MS);
  return out;
};

/** Pages timed by characters over the scene (a scene without narration, or the preview). */
export const sceneStarts = (pages: string[], durationMs: number): number[] => monotonic(proportionalStarts(pages, 0, durationMs, MIN_PAGE_MS));

/** Pages timed by characters over a narration clip that starts `offsetMs` into the scene. */
export const clipStarts = (pages: string[], offsetMs: number, clipMs: number): number[] =>
  monotonic(proportionalStarts(pages, offsetMs, clipMs, 0));

/** The scene text's pages for the frame size (one page = the text, unpaged). */
export const scenePageTexts = (scene: Pick<Scene, "text" | "text_layout">, brand: Brand | null | undefined, res: Resolution): string[] => {
  if (!scene.text) return [];
  const l = resolveTextLayout(scene.text_layout, brand);
  return splitPages(scene.text, l.paging, fontPx(l.size, res.width, res.height), res.width, res.height);
};

/** The pages a scene is drawn with: the render's `text_pages`, else split and timed here. One
 * page or none means "draw the text unpaged". */
export const scenePages = (
  scene: Pick<Scene, "text" | "text_layout" | "text_pages" | "duration_ms">,
  brand: Brand | null | undefined,
  res: Resolution,
): TextPage[] => {
  if (scene.text_pages && scene.text_pages.length > 1) return scene.text_pages;
  const texts = scenePageTexts(scene, brand, res);
  if (texts.length <= 1) return [];
  const starts = sceneStarts(texts, scene.duration_ms);
  return texts.map((text, k) => ({ text, start_ms: starts[k] }));
};

/** Index of the page showing at `ms` from the scene start (pages sorted by start). */
export const pageAt = (pages: readonly { start_ms: number }[], ms: number): number => {
  let k = 0;
  for (let i = 1; i < pages.length; i++) if (pages[i].start_ms <= ms) k = i;
  return k;
};

export type PagePreview = { text: string; startMs: number; endMs: number };

/**
 * The pages of a scene and when each shows in the editor preview: by characters over the scene;
 * with `heardSpeechMs`, the length of a heard narration clip of the scene's own text (试听), by
 * characters over the clip on the scene stretched as the render stretches it. The render times
 * narrated pages on the spoken words instead. Empty when the text is one page.
 */
export const pagePreview = (
  scene: Pick<Scene, "text" | "text_layout" | "duration_ms">,
  brand: Brand | null | undefined,
  res: Resolution,
  heardSpeechMs?: number | null,
): PagePreview[] => {
  const texts = scenePageTexts(scene, brand, res);
  if (texts.length <= 1) return [];
  const heard = heardSpeechMs != null;
  const length = heard ? Math.max(scene.duration_ms, heardSpeechMs + 300) : scene.duration_ms;
  const starts = heard ? clipStarts(texts, 0, heardSpeechMs) : sceneStarts(texts, scene.duration_ms);
  return texts.map((text, k) => ({ text, startMs: starts[k], endMs: k + 1 < texts.length ? starts[k + 1] : length }));
};

/** `text_pages` for a heard scene in the narrated preview (see `pagePreview`); empty unpaged. */
export const heardPages = (
  scene: Pick<Scene, "text" | "text_layout" | "duration_ms">,
  brand: Brand | null | undefined,
  res: Resolution,
  speechMs: number,
): TextPage[] => pagePreview(scene, brand, res, speechMs).map((p) => ({ text: p.text, start_ms: p.startMs }));
