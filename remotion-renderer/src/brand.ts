// Brand kit: default colors and fonts for every component, and the logo. Mirrors
// rust-core/crates/vimmer-dsl/src/brand.rs (which documents which element uses which color).
//
// A value set on an element always wins; an unset one falls back to the brand, and without a
// brand to the built-in value below, which is exactly what the components drew before brands
// existed. Keep this file free of runtime imports: the Node tests import it.

import type { Brand, FontRole } from "./types/timeline";

// The built-in look (no brand). Same values as the DEFAULT_* constants in brand.rs.
export const BUILTIN = {
  text: "#fafafa",
  lowerThirdSubtitle: "#d4d4d8",
  chartLabel: "#e4e4e7",
  accent: "#a5b4fc",
  primary: "#6366f1",
  background: "#09090b",
  subtitleBackground: "rgba(9, 9, 11, 0.72)",
} as const;

export const BASE_FONT_STACK = "Inter, 'Geist', system-ui, sans-serif";

type ColorKey = "primary" | "accent" | "text" | "muted_text" | "background" | "subtitle_background";

/** The brand's color `key`, else `builtin`. */
export const brandColor = (brand: Brand | null | undefined, key: ColorKey, builtin: string): string =>
  brand?.colors?.[key] ?? builtin;

/** The CSS font stack for a role: the brand family first, then the built-in stack. */
export const fontStack = (brand: Brand | null | undefined, role: FontRole): string => {
  const family = brand?.fonts?.[role]?.family;
  return family ? `"${family}", ${BASE_FONT_STACK}` : BASE_FONT_STACK;
};

/** Whether the brand sets a family for `role` (the components then style that text explicitly). */
export const hasFont = (brand: Brand | null | undefined, role: FontRole): boolean => Boolean(brand?.fonts?.[role]?.family);

/** `#rgb`, `#rrggbb` or `#rrggbbaa` as `rgba(r, g, b, alpha * a)`; anything else unchanged. */
export const withAlpha = (hex: string, alpha: number): string => {
  const m = /^#([0-9a-f]{3}|[0-9a-f]{6}|[0-9a-f]{8})$/i.exec(hex);
  if (!m) return hex;
  let h = m[1];
  if (h.length === 3) h = h.replace(/./g, (c) => c + c);
  const n = (i: number) => parseInt(h.slice(i, i + 2), 16);
  const a = h.length === 8 ? n(6) / 255 : 1;
  return `rgba(${n(0)}, ${n(2)}, ${n(4)}, ${+(alpha * a).toFixed(4)})`;
};

/** The accent treatment of a highlighted word at glow 0..1. Without a brand accent this is the
 * original indigo style, character for character. */
export const highlightColors = (brand: Brand | null | undefined, glow: number): { color: string; textShadow: string } => {
  const accent = brand?.colors?.accent;
  if (!accent) {
    return { color: BUILTIN.accent, textShadow: `0 0 ${24 * glow}px rgba(99, 102, 241, ${0.6 * glow})` };
  }
  return { color: accent, textShadow: `0 0 ${24 * glow}px ${withAlpha(accent, 0.6 * glow)}` };
};

export type LogoBox = { left?: number; right?: number; top?: number; bottom?: number; height: number; opacity: number };

/** Where the logo goes on a `width` x `height` frame (its width follows the image). */
export const logoBox = (brand: Brand | null | undefined, width: number, height: number): LogoBox | null => {
  const logo = brand?.logo;
  if (!logo) return null;
  const corner = logo.corner ?? "top_right";
  const margin = logo.margin ?? 0.05;
  const box: LogoBox = { height: (logo.size ?? 0.08) * height, opacity: logo.opacity ?? 1 };
  if (corner.endsWith("left")) box.left = margin * width;
  else box.right = margin * width;
  if (corner.startsWith("top")) box.top = margin * height;
  else box.bottom = margin * height;
  return box;
};

/** Whether the logo is drawn on the scene `sceneId`. */
export const logoShownOn = (brand: Brand | null | undefined, sceneId: string | undefined): boolean =>
  Boolean(brand?.logo) && !(sceneId != null && (brand?.logo?.hide_on ?? []).includes(sceneId));
