// Scene text size, position and paging (文字 Text) in the editor. Pure (type imports only), so
// it is unit-tested with Node (test/editor-text-layout.test.ts). Pages and their preview timing
// are the renderer's own (src/textPages.ts `pagePreview`).

import type { TextLayout } from "../../src/types/timeline";

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
  if (layout?.position != null && !["top", "middle", "bottom"].includes(layout.position)) out.push(`unknown text position “${layout.position}”`);
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
