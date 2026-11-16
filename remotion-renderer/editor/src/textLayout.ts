// Scene text size, position and paging (文字 Text) in the editor. Pure (type imports only), so
// it is unit-tested with Node (test/editor-text-layout.test.ts). Pages and their preview timing
// are the renderer's own (src/textPages.ts `pagePreview`).

import type { TextLayout, TextPoint } from "../../src/types/timeline";

// Same range as MIN_TEXT_PX / MAX_TEXT_PX in src/textPages.ts and text_layout.rs.
const MIN_TEXT_PX = 40;
const MAX_TEXT_PX = 160;

export type LayoutPart = keyof TextLayout;

/** `layout` with `part` set (removed when `value` is undefined/null); null when nothing is left. */
export const withLayoutPart = <K extends LayoutPart>(layout: TextLayout | null | undefined, part: K, value: TextLayout[K] | undefined): TextLayout | null => {
  const next: Record<string, unknown> = { ...(layout ?? {}) };
  if (value === undefined || value === null) delete next[part];
  else next[part] = value;
  for (const k of Object.keys(next)) if (next[k] == null) delete next[k];
  return Object.keys(next).length ? (next as TextLayout) : null;
};

/** Validation messages for a layout (mirrors TextLayout::problems in text_layout.rs). */
export const layoutProblems = (layout: TextLayout | null | undefined): string[] => {
  const out: string[] = [];
  const s = layout?.size;
  if (s != null && !(Number.isFinite(s) && s >= MIN_TEXT_PX && s <= MAX_TEXT_PX)) {
    out.push(`text size ${s} must be between ${MIN_TEXT_PX} and ${MAX_TEXT_PX} px`);
  }
  const p = layout?.position;
  if (p != null && typeof p === "object") {
    if (!(Number.isFinite(p.y) && p.y >= 0 && p.y <= 1)) out.push(`text position y ${p.y} must be between 0 (top) and 1 (bottom)`);
    if (p.x != null && !(Number.isFinite(p.x) && p.x >= 0 && p.x <= 1)) out.push(`text position x ${p.x} must be between 0 (left) and 1 (right)`);
  } else if (p != null && !["top", "middle", "bottom"].includes(p)) out.push(`unknown text position “${p}”`);
  if (layout?.paging != null && !["off", "auto", "manual"].includes(layout.paging)) out.push(`unknown paging “${layout.paging}”`);
  return out;
};

/** The manual page break: a blank line. */
export const PAGE_BREAK = "\n\n";

/** `text` with a page break at `at` (a caret position); returns the new text and caret. */
export const insertPageBreak = (text: string, at: number): { text: string; caret: number } => {
  const pos = Math.max(0, Math.min(at, text.length));
  const before = text.slice(0, pos).replace(/\s+$/u, "");
  const after = text.slice(pos).replace(/^\s+/u, "");
  const joined = before + PAGE_BREAK + after;
  return { text: joined, caret: before.length + PAGE_BREAK.length };
};

// ---- Free position (位置 → 自由位置)

/** Where a new free point starts: just under the middle, below a picture in the top half. */
export const DEFAULT_POINT: TextPoint = { y: 0.6 };

/** `point` with `axis` set to `value` (rounded to 0.01, clamped to 0..1); x 0.5 is left out. */
export const withPointAxis = (point: TextPoint | null | undefined, axis: "x" | "y", value: number): TextPoint => {
  const v = Math.round(Math.min(1, Math.max(0, value)) * 100) / 100;
  const next: TextPoint = { y: point?.y ?? DEFAULT_POINT.y, ...(point?.x != null ? { x: point.x } : {}) };
  if (axis === "y") next.y = v;
  else if (v === 0.5) delete next.x;
  else next.x = v;
  return next;
};

// ---- Keywords (关键词高亮)

/** `list` with `word` added (trimmed; nothing when empty or already there, ignoring case). */
export const addHighlight = (list: readonly string[] | null | undefined, word: string): string[] => {
  const w = word.trim();
  const out = [...(list ?? [])];
  if (!w || out.some((x) => x.toLowerCase() === w.toLowerCase())) return out;
  return [...out, w];
};

/** `list` without `word`. */
export const removeHighlight = (list: readonly string[] | null | undefined, word: string): string[] => (list ?? []).filter((x) => x !== word);

/** The yellow of keyword captions in short tutorial videos. */
export const KEYWORD_YELLOW = "#f5c400";
