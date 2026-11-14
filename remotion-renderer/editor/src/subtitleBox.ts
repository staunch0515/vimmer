// Subtitle box placement for the editor: the built-in place, snapping while dragging, nudging.
// Pure (type imports only), unit-tested with Node (test/editor-subtitle-box.test.ts).
//
// `subtitle_box` (vimmer-dsl model.rs SubtitleBox): x = the box center as a fraction of the frame
// width, y = its bottom edge as a fraction of the frame height. Without it the renderer centers
// the box with its bottom edge 4 % of the frame *width* above the bottom (Subtitles.tsx).

import type { Resolution, SubtitleBox } from "../../src/types/timeline";

/** Title-safe margin on every side (vimmer-check layout.rs TITLE_SAFE_MARGIN). */
export const SAFE_MARGIN = 0.05;
/** Pull distance of a snap target, as a fraction of the frame side. */
export const SNAP = 0.012;

const r4 = (v: number): number => Math.round(v * 10_000) / 10_000;

/** Where the renderer puts the box when the timeline has no `subtitle_box`. */
export const defaultBox = (res: Resolution): SubtitleBox => ({ x: 0.5, y: 1 - (0.04 * res.width) / res.height });

/** The box in use. */
export const boxOf = (box: SubtitleBox | null | undefined, res: Resolution): SubtitleBox => box ?? defaultBox(res);

/** Keeps a box valid for the DSL (x 0..1, y above 0 to 1) and rounds it for the JSON. */
export const clampBox = (b: SubtitleBox): SubtitleBox => ({
  x: r4(Math.min(1, Math.max(0, b.x))),
  y: r4(Math.min(1, Math.max(0.001, b.y))),
});

/** Size of the shown box, as fractions of the frame width (w) and height (h). */
export type BoxSize = { w: number; h: number };

/** Guides to draw: vertical lines at these x, horizontal lines at these y (frame fractions). */
export type Guides = { x: number[]; y: number[] };

const nearest = (value: number, targets: { at: number; guide: number }[]): { at: number; guide: number } | null => {
  let best: { at: number; guide: number } | null = null;
  for (const t of targets) {
    if (Math.abs(t.at - value) <= SNAP && (!best || Math.abs(t.at - value) < Math.abs(best.at - value))) best = t;
  }
  return best;
};

/**
 * Snaps a dragged box: its center to the vertical center line, its sides to the safe area's sides;
 * its middle to the horizontal center line, its bottom to the safe bottom (and to the built-in
 * place), its top to the safe top. Returns the snapped box and the guides to show.
 */
export const snapBox = (b: SubtitleBox, size: BoxSize, res: Resolution): { box: SubtitleBox; guides: Guides } => {
  const xs = nearest(b.x, [
    { at: 0.5, guide: 0.5 },
    { at: SAFE_MARGIN + size.w / 2, guide: SAFE_MARGIN },
    { at: 1 - SAFE_MARGIN - size.w / 2, guide: 1 - SAFE_MARGIN },
  ]);
  const ys = nearest(b.y, [
    { at: 1 - SAFE_MARGIN, guide: 1 - SAFE_MARGIN },
    { at: defaultBox(res).y, guide: defaultBox(res).y },
    { at: 0.5 + size.h / 2, guide: 0.5 },
    { at: SAFE_MARGIN + size.h, guide: SAFE_MARGIN },
  ]);
  return {
    box: clampBox({ x: xs ? xs.at : b.x, y: ys ? ys.at : b.y }),
    guides: { x: xs ? [xs.guide] : [], y: ys ? [ys.guide] : [] },
  };
};

/** Moves the box by a number of pixels of the frame (arrow keys: 1 px, Shift: 10 px). */
export const nudgeBox = (b: SubtitleBox, dxPx: number, dyPx: number, res: Resolution): SubtitleBox =>
  clampBox({ x: b.x + dxPx / res.width, y: b.y + dyPx / res.height });

/** Whether the box would leave the title-safe area (for the hint under the fields). */
export const outsideSafe = (b: SubtitleBox, size: BoxSize): boolean =>
  b.x - size.w / 2 < SAFE_MARGIN - 1e-6 ||
  b.x + size.w / 2 > 1 - SAFE_MARGIN + 1e-6 ||
  b.y > 1 - SAFE_MARGIN + 1e-6 ||
  b.y - size.h < SAFE_MARGIN - 1e-6;

/** Problems with a `subtitle_box` (mirrors SubtitleBox::problems in model.rs). */
export const boxProblems = (b: SubtitleBox | null | undefined): string[] => {
  if (!b) return [];
  const out: string[] = [];
  if (!(Number.isFinite(b.x) && b.x >= 0 && b.x <= 1)) out.push("Subtitle position x must be between 0 and 1.");
  if (!(Number.isFinite(b.y) && b.y > 0 && b.y <= 1)) out.push("Subtitle position y must be above 0 and at most 1.");
  return out;
};
