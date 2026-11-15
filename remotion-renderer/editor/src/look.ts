// The look (画面氛围) fields of the editor: pure helpers that turn one field change into the next
// `look` object. Type imports only, so it is unit-tested with Node (test/editor-look.test.ts).
// The model and the renderer side are in src/motion/look.ts; the DSL in vimmer-dsl/src/look.rs.
//
// Saved JSON stays minimal: a part that is unset is left out (the scene then inherits it), and a
// look with no parts is removed, so clearing everything gives back the timeline as it was.
import type { Grade, Look, Overlay, OverlayKind } from "../../src/types/timeline";

export const GRADE_CHOICES: { value: string; label: string }[] = [
  { value: "none", label: "无 None" },
  { value: "auto", label: "自动 Auto (from the music mood)" },
  { value: "warm", label: "温暖文艺 Warm" },
  { value: "tech_blue", label: "科技蓝 Tech blue" },
  { value: "moody", label: "悬疑暗调 Moody" },
  { value: "noir", label: "黑白 Noir" },
  { value: "vintage", label: "复古 Vintage" },
  { value: "vivid", label: "鲜艳 Vivid" },
];

export const OVERLAY_CHOICES: { value: OverlayKind; label: string }[] = [
  { value: "snow", label: "雪花 Snow" },
  { value: "dust", label: "尘埃 Dust" },
  { value: "bokeh", label: "光斑 Bokeh" },
  { value: "film_grain", label: "胶片颗粒 Film grain" },
];

const TINTED: OverlayKind[] = ["dust", "bokeh"];
export const takesColor = (kind: OverlayKind): boolean => TINTED.includes(kind);
export const DEFAULT_TINT: Record<string, string> = { dust: "#fff4dc", bokeh: "#ffd8a0" };

/** The look without empty parts; `undefined` when nothing is left (the key is then dropped). */
export const compactLook = (look: Look | null | undefined): Look | undefined => {
  if (!look) return undefined;
  const out: Look = {};
  if (look.grade) out.grade = look.grade;
  if (look.overlays) out.overlays = look.overlays;
  return Object.keys(out).length ? out : undefined;
};

/** Sets the grade (`undefined`: inherit / no grade). */
export const withGrade = (look: Look | null | undefined, grade: Grade | undefined): Look | undefined =>
  compactLook({ ...look, grade });

/** Picks a preset, keeping the strength. `""` clears the grade (inherit). */
export const withPreset = (look: Look | null | undefined, preset: string): Look | undefined =>
  withGrade(look, preset ? { preset, strength: look?.grade?.strength ?? 1 } : undefined);

export const withStrength = (look: Look | null | undefined, strength: number): Look | undefined =>
  look?.grade ? withGrade(look, { ...look.grade, strength: clamp01(strength) }) : compactLook(look);

/** Sets the overlay list (`undefined`: inherit; `[]`: none here). */
export const withOverlays = (look: Look | null | undefined, overlays: Overlay[] | undefined): Look | undefined =>
  compactLook({ ...look, overlays });

/** A new overlay of the first kind not yet in the list (snow when all are used). */
export const addOverlay = (list: Overlay[]): Overlay[] => {
  const kind = OVERLAY_CHOICES.map((c) => c.value).find((k) => !list.some((o) => o.kind === k)) ?? "snow";
  return [...list, { kind, intensity: 0.5 }];
};

/** Changes one overlay. A kind without a color drops its color; `color: undefined` removes it. */
export const patchOverlay = (list: Overlay[], index: number, patch: Partial<Overlay>): Overlay[] =>
  list.map((o, i) => {
    if (i !== index) return o;
    const next: Overlay = { ...o, ...patch };
    if (next.intensity !== undefined) next.intensity = clamp01(next.intensity);
    if (next.color === undefined || !takesColor(next.kind)) delete next.color;
    return next;
  });

export const removeOverlay = (list: Overlay[], index: number): Overlay[] => list.filter((_, i) => i !== index);

const clamp01 = (v: number) => Math.round(Math.max(0, Math.min(1, Number.isFinite(v) ? v : 0)) * 100) / 100;
