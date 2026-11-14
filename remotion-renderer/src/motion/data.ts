// Pure number formatting and timing of the data layers (counter, bar chart, progress ring). No
// Remotion imports and no runtime imports, so it can be unit-tested with Node
// (test/motion.test.ts). Mirrors `format_number`, `count_frames` and `frame_progress` in
// rust-core/crates/vimmer-dsl/src/layer.rs.
//
// Everything is a function of the layer-relative frame, so chunked renders match full ones.

import type { Layer } from "../types/timeline";

const msToFrames = (ms: number, fps: number): number => Math.round((ms * fps) / 1000);

// `value` with `decimals` decimals, `separator` between groups of three integer digits and
// `decimalMark` before the decimals; "-" before the prefix for negatives, never "-0". Uses
// toFixed (exact, locale-free), never toLocaleString / Intl, so the text does not depend on the
// machine's locale.
export const formatNumber = (
  value: number,
  decimals: number,
  separator = ",",
  decimalMark = ".",
  prefix = "",
  suffix = "",
): string => {
  const fixed = Math.abs(value).toFixed(decimals);
  const dot = fixed.indexOf(".");
  const int = dot < 0 ? fixed : fixed.slice(0, dot);
  const frac = dot < 0 ? "" : fixed.slice(dot + 1);
  const negative = value < 0 && /[1-9]/.test(fixed);
  let grouped = "";
  for (let i = 0; i < int.length; i++) {
    if (i > 0 && (int.length - i) % 3 === 0) grouped += separator;
    grouped += int[i];
  }
  return `${negative ? "-" : ""}${prefix}${grouped}${frac ? decimalMark + frac : ""}${suffix}`;
};

// Frames an animation of `durationMs` takes, or, without a duration, the layer's frames before its
// exit (`span` = the layer's frames). At least 1.
export const countFrames = (layer: Layer, durationMs: number | null | undefined, span: number, fps: number): number => {
  const n =
    durationMs != null
      ? msToFrames(durationMs, fps)
      : span - (layer.exit ? msToFrames(layer.exit.duration_ms ?? 400, fps) : 0);
  return Math.max(1, n);
};

// Progress 0..1 of an animation `n` frames long on layer frame `frame`: 0 on the first frame, 1 from
// frame n - 1 on.
export const frameProgress = (frame: number, n: number): number => (n <= 1 ? 1 : Math.min(1, Math.max(0, frame / (n - 1))));

// The value between `from` and `to` at eased progress `e` (may overshoot with springy easings).
export const lerpValue = (from: number, to: number, e: number): number => from + (to - from) * e;

// ---- Bar chart geometry (components/LayerKinds.tsx BarChartView, vimmer-check layers.rs).

export const MAX_BARS = 12;
// Share of each bar's slot the bar fills (the rest is the gap).
export const BAR_FILL = 0.62;
// Horizontal charts: label column and value column, as fractions of the box width.
export const HBAR_LABEL_FRAC = 0.28;
export const HBAR_VALUE_FRAC = 0.16;
// Vertical charts: label row and value row heights, in font sizes.
export const VBAR_LABEL_EM = 1.6;
export const VBAR_VALUE_EM = 1.5;

// Grow progress 0..1 (before easing) of bar `i` on layer frame `frame`: bar i starts
// `staggerMs * i` after the layer and grows for `growMs`.
export const barProgress = (frame: number, i: number, fps: number, growMs: number, staggerMs: number): number => {
  const start = msToFrames(staggerMs * i, fps);
  return frameProgress(frame - start, Math.max(1, msToFrames(growMs, fps)));
};

// The scale the bars are drawn against: `max`, else the largest value (1 when all are 0).
export const chartMax = (values: readonly number[], max: number | null | undefined): number => {
  if (max != null && max > 0) return max;
  const m = Math.max(0, ...values);
  return m > 0 ? m : 1;
};

// ---- Progress ring.

// Visible arc fraction 0..1 at eased progress `e` from `from` to `value` (clamped: an elastic
// overshoot never draws more than a full ring or a negative arc).
export const ringFraction = (from: number, value: number, e: number): number =>
  Math.min(1, Math.max(0, lerpValue(from, value, e)));
