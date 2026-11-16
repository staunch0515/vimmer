// The look of a scene (画面氛围): a color grade and animated atmosphere overlays. Mirrors
// rust-core/crates/vimmer-dsl/src/look.rs (inheritance, presets, mood map). Pure: no Remotion,
// no clock, no Math.random, so the same absolute timeline frame always draws the same picture,
// in a full render, in any render chunk and in the editor preview.

import type { Grade, Look, Overlay, OverlayKind, VideoTimeline } from "../types/timeline";

export const GRADE_PRESETS = ["none", "auto", "warm", "tech_blue", "noir", "moody", "vintage", "vivid"] as const;
export type GradePreset = (typeof GRADE_PRESETS)[number];
export const OVERLAY_KINDS: OverlayKind[] = ["snow", "dust", "bokeh", "film_grain"];
export const TINTED_OVERLAYS: OverlayKind[] = ["dust", "bokeh"];
export const MAX_OVERLAYS = 4;
export const DEFAULT_INTENSITY = 0.5;
export const DEFAULT_STRENGTH = 1;

// ---- Inheritance

export type EffectiveLook = {
  grade: Grade | null;
  overlays: Overlay[];
  // The default's overlays: their particles continue across scene cuts.
  inherited: boolean;
};

// The scene's own part replaces the default's part when present (an empty overlay list = none).
export const effectiveLook = (defaultLook: Look | null | undefined, own: Look | null | undefined): EffectiveLook => {
  const grade = own?.grade ?? defaultLook?.grade ?? null;
  if (own?.overlays) return { grade, overlays: own.overlays, inherited: false };
  return { grade, overlays: defaultLook?.overlays ?? [], inherited: true };
};

// ---- Mood -> preset (same table as `preset_for_mood` in look.rs, documented in the README)

const MOOD_PRESETS: Record<string, GradePreset> = {
  warm: "warm", hopeful: "warm", romantic: "warm", nostalgic: "warm",
  calm: "vintage", reflective: "vintage", ambient: "vintage",
  tense: "moody", suspense: "moody", dark: "moody", mysterious: "moody", sad: "moody",
  happy: "vivid", playful: "vivid", upbeat: "vivid", energetic: "vivid", fun: "vivid",
  epic: "tech_blue", dramatic: "tech_blue", inspiring: "tech_blue", tech: "tech_blue", corporate: "tech_blue", futuristic: "tech_blue",
};

export const presetForMood = (mood: string | null | undefined): GradePreset =>
  MOOD_PRESETS[(mood ?? "").trim().toLowerCase()] ?? "none";

// What `auto` stands for in this timeline: the music pick's mood, else none.
export const autoPreset = (t: Pick<VideoTimeline, "music">): GradePreset => presetForMood(t.music?.pick?.mood);

// Every `auto` grade replaced by `preset`; the same object when there is none (old timelines
// pass through untouched). The editor calls this before it swaps the pick for the chosen file.
export const resolveAutoGrades = <T extends VideoTimeline>(t: T, preset: GradePreset = autoPreset(t)): T => {
  const fix = (look: Look | null | undefined): Look | null | undefined =>
    look?.grade?.preset === "auto" ? { ...look, grade: { ...look.grade, preset } } : look;
  const hasAuto = t.look?.grade?.preset === "auto" || t.scenes.some((s) => s.look?.grade?.preset === "auto");
  if (!hasAuto) return t;
  return { ...t, look: fix(t.look), scenes: t.scenes.map((s) => (s.look?.grade?.preset === "auto" ? { ...s, look: fix(s.look) } : s)) };
};

// ---- Color grade: one 4x5 feColorMatrix (sRGB) plus a vignette, both scaled by `strength`.

// Rows R, G, B, A; columns r, g, b, a, offset (as in SVG feColorMatrix "matrix" values).
export type ColorMatrix = number[];
type M5 = number[][]; // 5x5, last row 0 0 0 0 1

const IDENTITY5: M5 = [
  [1, 0, 0, 0, 0],
  [0, 1, 0, 0, 0],
  [0, 0, 1, 0, 0],
  [0, 0, 0, 1, 0],
  [0, 0, 0, 0, 1],
];
const mul = (a: M5, b: M5): M5 => a.map((row) => b[0].map((_, j) => row.reduce((s, v, k) => s + v * b[k][j], 0)));
const LUMA = [0.2126, 0.7152, 0.0722];
const saturation = (s: number): M5 => [
  ...[0, 1, 2].map((i) => [...LUMA.map((l, j) => l * (1 - s) + (i === j ? s : 0)), 0, 0]),
  [0, 0, 0, 1, 0],
  [0, 0, 0, 0, 1],
];
// Around mid-gray: x' = c (x - 0.5) + 0.5.
const contrast = (c: number): M5 => [
  [c, 0, 0, 0, 0.5 * (1 - c)],
  [0, c, 0, 0, 0.5 * (1 - c)],
  [0, 0, c, 0, 0.5 * (1 - c)],
  [0, 0, 0, 1, 0],
  [0, 0, 0, 0, 1],
];
// Per-channel gain and offset (tint, lift, exposure).
const channels = (gain: [number, number, number], offset: [number, number, number] = [0, 0, 0]): M5 => [
  [gain[0], 0, 0, 0, offset[0]],
  [0, gain[1], 0, 0, offset[1]],
  [0, 0, gain[2], 0, offset[2]],
  [0, 0, 0, 1, 0],
  [0, 0, 0, 0, 1],
];
const SEPIA = [
  [0.393, 0.769, 0.189],
  [0.349, 0.686, 0.168],
  [0.272, 0.534, 0.131],
];
const sepia = (amount: number): M5 => [
  ...[0, 1, 2].map((i) => [...SEPIA[i].map((v, j) => v * amount + (i === j ? 1 - amount : 0)), 0, 0]),
  [0, 0, 0, 1, 0],
  [0, 0, 0, 0, 1],
];
// Applied right to left: the first op in the list acts first.
const chain = (...ops: M5[]): M5 => ops.reduce((acc, op) => mul(op, acc), IDENTITY5);

type PresetDef = { matrix: M5; vignette: number };

// Tuned to read as a grade, not a filter: small gains, gentle contrast, lifted blacks where it
// suits. `vignette` is the darkening at the corners at strength 1.
const PRESETS: Record<Exclude<GradePreset, "none" | "auto">, PresetDef> = {
  // 温暖文艺: golden highlights, slightly lifted shadows, a touch less blue.
  warm: { matrix: chain(saturation(1.06), contrast(1.04), channels([1.06, 1.01, 0.88], [0.02, 0.012, 0])), vignette: 0.18 },
  // 科技蓝: cool teal-blue, crisper contrast, a little less saturation.
  tech_blue: { matrix: chain(saturation(0.88), contrast(1.1), channels([0.88, 1.0, 1.12], [-0.01, 0.012, 0.04])), vignette: 0.22 },
  // Black and white, strong contrast.
  noir: { matrix: chain(saturation(0), contrast(1.28), channels([0.97, 0.97, 0.97], [-0.01, -0.01, -0.01])), vignette: 0.5 },
  // 悬疑暗调: muted, darker, teal shadows.
  moody: { matrix: chain(saturation(0.62), contrast(1.14), channels([0.84, 0.9, 0.96], [-0.012, 0.004, 0.025])), vignette: 0.48 },
  // Faded film: some sepia, low contrast, lifted blacks.
  vintage: { matrix: chain(sepia(0.38), saturation(0.82), contrast(0.88), channels([1.04, 1.0, 0.9], [0.045, 0.035, 0.03])), vignette: 0.32 },
  // Punchy color.
  vivid: { matrix: chain(saturation(1.38), contrast(1.08)), vignette: 0 },
};

export type GradeStyle = { matrix: ColorMatrix; vignette: number };

// The grade to draw, or null when it changes nothing (absent, `none`, unresolved `auto` without a
// mood, strength 0). `autoAs` is what `auto` stands for (see autoPreset).
export const gradeStyle = (grade: Grade | null | undefined, autoAs: GradePreset = "none"): GradeStyle | null => {
  if (!grade) return null;
  const preset = grade.preset === "auto" ? autoAs : grade.preset;
  const def = PRESETS[preset as keyof typeof PRESETS];
  const s = Math.max(0, Math.min(1, grade.strength ?? DEFAULT_STRENGTH));
  if (!def || s <= 0) return null;
  const m = def.matrix;
  const matrix = [0, 1, 2, 3].flatMap((i) => m[i].map((v, j) => round6(IDENTITY5[i][j] + s * (v - IDENTITY5[i][j]))));
  return { matrix, vignette: round6(def.vignette * s) };
};

const round6 = (v: number) => Math.round(v * 1e6) / 1e6;

// Applies a 4x5 matrix to an sRGB color in 0..1 (what feColorMatrix does with
// color-interpolation-filters="sRGB"), clamped. For tests and the editor's swatches.
export const applyMatrix = (m: ColorMatrix, [r, g, b]: [number, number, number]): [number, number, number] =>
  [0, 1, 2].map((i) => Math.max(0, Math.min(1, m[i * 5] * r + m[i * 5 + 1] * g + m[i * 5 + 2] * b + m[i * 5 + 3] * 1 + m[i * 5 + 4]))) as [
    number,
    number,
    number,
  ];

// ---- Determinism: a small integer hash, so nothing depends on Math.random or the clock.

// FNV-1a of a string, as an unsigned 32-bit integer.
export const hashString = (s: string): number => {
  let h = 0x811c9dc5;
  for (let i = 0; i < s.length; i++) {
    h ^= s.charCodeAt(i);
    h = Math.imul(h, 0x01000193) >>> 0;
  }
  return h >>> 0;
};

// A number in [0, 1) from a seed and two indices (a lowbias32 integer mix).
export const rand = (seed: number, i: number, k: number): number => {
  let x = (seed ^ Math.imul(i + 1, 0x9e3779b1) ^ Math.imul(k + 1, 0x85ebca6b)) >>> 0;
  x ^= x >>> 16;
  x = Math.imul(x, 0x7feb352d) >>> 0;
  x ^= x >>> 15;
  x = Math.imul(x, 0x846ca68b) >>> 0;
  x ^= x >>> 16;
  return (x >>> 0) / 4294967296;
};

// Seed of overlay `index`: the timeline default's overlays share one seed in every scene (so
// snow keeps falling across a cut); a scene's own overlays get the scene id in theirs.
export const overlaySeed = (overlay: Overlay, index: number, inherited: boolean, sceneId: string): number =>
  hashString(`${inherited ? "" : `${sceneId}/`}look/${index}/${overlay.kind}`);

const mod = (a: number, n: number) => ((a % n) + n) % n;

// ---- Particles. Positions are in pixels of a `width` x `height` frame; sizes scale with the
// shorter side so a 9:16 video gets the same look. Counts are capped for render speed.

export type Particle = { x: number; y: number; r: number; opacity: number };

export const MAX_PARTICLES: Record<Exclude<OverlayKind, "film_grain">, number> = { snow: 150, dust: 120, bokeh: 20 };

export const particleCount = (kind: OverlayKind, intensity: number): number => {
  const k = clamp01(intensity);
  if (k <= 0) return 0;
  switch (kind) {
    case "snow":
      return Math.min(MAX_PARTICLES.snow, Math.round(30 + 120 * k));
    case "dust":
      return Math.min(MAX_PARTICLES.dust, Math.round(25 + 95 * k));
    case "bokeh":
      return Math.min(MAX_PARTICLES.bokeh, Math.round(6 + 14 * k));
    case "film_grain":
      return 0;
  }
};

const clamp01 = (v: number) => Math.max(0, Math.min(1, Number.isFinite(v) ? v : 0));

// Every particle of an overlay at absolute timeline frame `frame`. A pure function of its
// arguments: the frame must be the frame of the full timeline, never a chunk-local one.
export const particlesAt = (kind: OverlayKind, intensity: number, seed: number, frame: number, width: number, height: number): Particle[] => {
  const n = particleCount(kind, intensity);
  const k = clamp01(intensity);
  const u = Math.min(width, height) / 1080; // size unit
  const out: Particle[] = [];
  for (let i = 0; i < n; i++) {
    const R = (j: number) => rand(seed, i, j);
    if (kind === "snow") {
      // Depth 0 (far: small, slow, faint) .. 1 (near: big, fast, bright).
      const d = R(0);
      const r = (2.2 + 6.5 * d ** 1.4) * u;
      const m = r * 4 + 30 * u; // off-screen margin so flakes enter and leave smoothly
      const vy = (0.7 + 2.5 * d) * u; // px per frame
      const drift = 0.22 * vy;
      const amp = (5 + 16 * d) * u;
      const w = 0.018 + 0.03 * R(1);
      const phase = R(2) * Math.PI * 2;
      const y = mod(R(3) * (height + 2 * m) + vy * frame, height + 2 * m) - m;
      const x = mod(R(4) * (width + 2 * m) + drift * frame + amp * Math.sin(phase + w * frame), width + 2 * m) - m;
      out.push({ x, y, r, opacity: (0.45 + 0.5 * d) * (0.7 + 0.3 * k) });
    } else if (kind === "dust") {
      const d = R(0);
      const r = (1.4 + 4 * d * d) * u;
      const m = 20 * u;
      const vx = (R(1) - 0.5) * 0.5 * u;
      const vy = -(0.04 + 0.22 * R(2)) * u; // rises slowly, like motes in a light beam
      const w = 0.01 + 0.02 * R(3);
      const phase = R(4) * Math.PI * 2;
      const x = mod(R(5) * (width + 2 * m) + vx * frame + 9 * u * Math.sin(phase + w * frame), width + 2 * m) - m;
      const y = mod(R(6) * (height + 2 * m) + vy * frame + 7 * u * Math.cos(phase * 1.3 + w * 0.8 * frame), height + 2 * m) - m;
      const twinkle = 0.55 + 0.45 * Math.sin(R(7) * Math.PI * 2 + (0.03 + 0.05 * R(8)) * frame);
      out.push({ x, y, r, opacity: (0.35 + 0.6 * d) * twinkle * (0.75 + 0.25 * k) });
    } else if (kind === "bokeh") {
      const r = (45 + 115 * R(0)) * u;
      const m = r;
      const vx = (R(1) - 0.5) * 0.45 * u;
      const vy = (R(2) - 0.6) * 0.3 * u;
      const x = mod(R(3) * (width + 2 * m) + vx * frame, width + 2 * m) - m;
      const y = mod(R(4) * (height + 2 * m) + vy * frame, height + 2 * m) - m;
      const pulse = 0.65 + 0.35 * Math.sin(R(5) * Math.PI * 2 + (0.012 + 0.02 * R(6)) * frame);
      out.push({ x, y, r, opacity: (0.14 + 0.24 * R(7)) * pulse * (0.6 + 0.6 * k) });
    }
  }
  return out;
};

// Film grain: the noise seed of a frame (a new grain pattern every frame, like film) and the
// opacity of the grain layer.
export const grainSeed = (seed: number, frame: number): number => (seed + Math.imul(frame + 1, 7919)) % 100003;
export const grainOpacity = (intensity: number): number => {
  const k = clamp01(intensity);
  return k <= 0 ? 0 : round6(0.05 + 0.2 * k);
};

// Default tints: a warm white for dust, a soft gold for bokeh. Snow is white.
export const overlayColor = (o: Overlay): string =>
  o.color && TINTED_OVERLAYS.includes(o.kind) ? o.color : o.kind === "bokeh" ? "#ffd8a0" : o.kind === "dust" ? "#fff4dc" : "#ffffff";

// Whether a look draws anything: without one the scene keeps its old DOM exactly.
export const drawsAnything = (look: EffectiveLook, autoAs: GradePreset): boolean =>
  gradeStyle(look.grade, autoAs) !== null || look.overlays.some((o) => OVERLAY_KINDS.includes(o.kind) && clamp01(o.intensity ?? DEFAULT_INTENSITY) > 0);

// Validation messages of one look, worded like look.rs `Look::problems` (the editor shows them).
export const lookProblems = (look: Look): string[] => {
  const out: string[] = [];
  const g = look.grade;
  if (g) {
    if (!(GRADE_PRESETS as readonly string[]).includes(g.preset)) out.push(`grade preset \`${g.preset}\` is unknown; presets: ${GRADE_PRESETS.join(", ")}`);
    const s = g.strength ?? DEFAULT_STRENGTH;
    if (!(Number.isFinite(s) && s >= 0 && s <= 1)) out.push(`grade strength ${s} must be between 0 and 1`);
  }
  const list = look.overlays;
  if (list) {
    if (list.length > MAX_OVERLAYS) out.push(`${list.length} overlays; the limit is ${MAX_OVERLAYS}`);
    list.forEach((o, i) => {
      if (!OVERLAY_KINDS.includes(o.kind)) out.push(`overlay ${i} kind \`${o.kind}\` is unknown; kinds: ${OVERLAY_KINDS.join(", ")}`);
      else if (list.slice(0, i).some((p) => p.kind === o.kind)) out.push(`overlay ${i}: \`${o.kind}\` is listed twice; raise its intensity instead`);
      const v = o.intensity ?? DEFAULT_INTENSITY;
      if (!(Number.isFinite(v) && v >= 0 && v <= 1)) out.push(`overlay ${i} intensity ${v} must be between 0 and 1`);
      if (o.color != null && !/^#([0-9a-fA-F]{3}|[0-9a-fA-F]{6}|[0-9a-fA-F]{8})$/.test(o.color)) {
        out.push(`overlay ${i} color \`${o.color}\` must be a color like #rgb, #rrggbb or #rrggbbaa`);
      } else if (o.color != null && OVERLAY_KINDS.includes(o.kind) && !TINTED_OVERLAYS.includes(o.kind)) {
        out.push(`overlay ${i}: \`${o.kind}\` has no color (only dust and bokeh take one)`);
      }
    });
  }
  return out;
};
