// Text style: outline, box and shadow (mirrors rust-core/crates/vimmer-dsl/src/text_style.rs).
// Pure, no runtime imports: the Node tests import it.
//
// Each part is the element's own, else the brand's `text_style`, else the built-in look (no
// outline, no box; the scene text keeps its soft shadow, text layers have none). With nothing set
// the components take their old code path, so old timelines render exactly as before.
//
// Outline technique: `-webkit-text-stroke` at twice the width with `paint-order: stroke fill`.
// The stroke is centred on the glyph edge; painting the fill over it hides the inner half, so the
// visible ring is `width` px outside the glyph and the glyph itself keeps its full weight. That
// matters for CJK, whose strokes are thin: a plain stroke (fill first) eats into them, and
// layered text-shadows (8-16 offset copies) give a lumpy ring at 4+ px and cost a shadow pass per
// copy. The stroke also hides overlapping contours inside variable / CJK fonts, since the fill
// covers them.

import type { Brand, TextBackground, TextOutline, TextShadow, TextStyle } from "./types/timeline";

/** A box with no `color` has no fill. */
export const TRANSPARENT = "#00000000";

/** Extra space between the lines of a scene text box with a border (em), beyond its padding and
 * border; mirrors SCENE_BORDER_LINE_GAP_EM in vimmer-check/src/layout.rs. */
export const BORDER_LINE_GAP_EM = 0.12;

/** The box's border width in px on the frame (0 without one). */
export const borderPx = (box: TextBackground | null | undefined, scale: number): number => {
  const w = box?.border ? (box.border.width ?? 2) : 0;
  return w > 0 ? w * scale : 0;
};

/** CSS border of a box (nothing without one). */
export const borderCss = (box: TextBackground | null | undefined, scale: number): Record<string, string> => {
  const w = borderPx(box, scale);
  return w > 0 && box?.border ? { border: `${px(w)} solid ${box.border.color}` } : {};
};

/** The scene text's built-in shadow (also the `soft` preset at 1080p). */
export const SOFT_SHADOW = "0 4px 32px rgba(0, 0, 0, 0.6)";

/** px at 1080p -> px on a `width` x `height` frame. */
export const styleScale = (width: number, height: number): number => Math.min(width, height) / 1080;

/** Each part of `own`, else of the brand's text style. */
export const resolveTextStyle = (own: TextStyle | null | undefined, brand: Brand | null | undefined): TextStyle => {
  const b = brand?.text_style ?? null;
  const out: TextStyle = {};
  const outline = own?.outline ?? b?.outline ?? null;
  const background = own?.background ?? b?.background ?? null;
  const shadow = own?.shadow ?? b?.shadow ?? null;
  if (outline) out.outline = outline;
  if (background) out.background = background;
  if (shadow) out.shadow = shadow;
  return out;
};

/** The resolved style of a text layer (its own outline / background / shadow, else the brand's). */
export const layerTextStyle = (
  layer: { outline?: TextOutline | null; background?: TextBackground | null; shadow?: TextShadow | null },
  brand: Brand | null | undefined,
): TextStyle => resolveTextStyle({ outline: layer.outline, background: layer.background, shadow: layer.shadow }, brand);

export const isEmptyStyle = (s: TextStyle): boolean => !s.outline && !s.background && !s.shadow;

function px(v: number): string {
  return `${+v.toFixed(3)}px`;
}

/** CSS for an outline (nothing for none or width 0). */
export const outlineCss = (o: TextOutline | null | undefined, scale: number): Record<string, string> => {
  const width = o?.width ?? 4;
  if (!o || !(width > 0)) return {};
  return { WebkitTextStroke: `${px(2 * width * scale)} ${o.color ?? "#000000"}`, paintOrder: "stroke fill" };
};

/** CSS `text-shadow` for a shadow; `fallback` when none is set. "none" draws none. */
export const shadowCss = (s: TextShadow | null | undefined, scale: number, fallback?: string): string | undefined => {
  if (s == null) return fallback;
  if (s === "none") return "none";
  if (s === "soft") return scale === 1 ? SOFT_SHADOW : `0 ${px(4 * scale)} ${px(32 * scale)} rgba(0, 0, 0, 0.6)`;
  if (s === "strong")
    return `0 ${px(2 * scale)} ${px(4 * scale)} rgba(0, 0, 0, 0.85), 0 ${px(4 * scale)} ${px(18 * scale)} rgba(0, 0, 0, 0.75)`;
  return `${px((s.x ?? 0) * scale)} ${px((s.y ?? 0) * scale)} ${px((s.blur ?? 0) * scale)} ${s.color}`;
};

/** `#rgb`, `#rrggbb` or `#rrggbbaa` split into the opaque color and its alpha (0..1). */
export const splitAlpha = (hex: string): { solid: string; alpha: number } => {
  const m = /^#([0-9a-f]{3}|[0-9a-f]{6}|[0-9a-f]{8})$/i.exec(hex);
  if (!m) return { solid: hex, alpha: 1 };
  let h = m[1];
  if (h.length === 3) h = h.replace(/./g, (c) => c + c);
  const alpha = h.length === 8 ? parseInt(h.slice(6, 8), 16) / 255 : 1;
  return { solid: `#${h.slice(0, 6)}`, alpha };
};

/** A highlighted word's glow combined with the style's shadow (glow first, drawn on top). */
export const withGlow = (glow: string, shadow: string | undefined): string =>
  shadow && shadow !== "none" ? `${glow}, ${shadow}` : glow;
