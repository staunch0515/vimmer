// Editor helpers for scene layers: defaults for new layers, stacking moves, and the checks that
// mirror vimmer-dsl validation (rust-core/crates/vimmer-dsl/src/validation.rs, layer_problems).

import type { Layer, LayerAnimationKind, LayerKind, Resolution, Scene } from "../../src/types/timeline";
import { splitLayers } from "../../src/motion/layers";

export const LAYER_KINDS: LayerKind[] = ["text", "shape", "image", "icon", "lower_third"];
export const ANIMATION_KINDS: LayerAnimationKind[] = ["fade", "slide_up", "slide_down", "slide_left", "slide_right", "pop"];
export const MAX_LAYERS = 32;
export const MAX_LAYER_TEXT_CHARS = 400;

const uniqueId = (base: string, taken: Set<string>): string => {
  if (!taken.has(base)) return base;
  for (let i = 2; ; i++) if (!taken.has(`${base}-${i}`)) return `${base}-${i}`;
};

// A visible, valid layer of `kind`, centered-ish, drawn over the scene text. Lower thirds and
// charts sit inside the title-safe area, above the subtitle band.
export const newLayer = (kind: LayerKind, scene: Scene, res: Resolution = { width: 1920, height: 1080 }): Layer => {
  const id = uniqueId(kind, new Set((scene.layers ?? []).map((l) => l.id)));
  const portrait = res.height > res.width;
  switch (kind) {
    case "text":
      return { id, kind, x: 0.1, y: 0.1, width: 0.5, text: "New text", font_size: 0.06, align: "left" };
    case "shape":
      return { id, kind, shape: "rect", x: 0.35, y: 0.35, width: 0.3, height: 0.3, fill: "#6366f1", corner_radius: 0.02 };
    case "image":
      return { id, kind, src: "assets/sample.png", x: 0.6, y: 0.1, width: 0.3, height: 0.3, fit: "cover" };
    case "icon":
      return { id, kind, src: "assets/icons/star.svg", color: "#facc15", x: 0.45, y: 0.4, width: 0.1, height: 0.18 };
    case "lower_third":
      return portrait
        ? { id, kind, title: "Name Surname", subtitle: "Role, Company", x: 0.06, y: 0.62, width: 0.8, font_size: 0.03 }
        : { id, kind, title: "Name Surname", subtitle: "Role, Company", x: 0.06, y: 0.66, width: 0.5 };
  }
};

// The layer list after moving layer `index` one step up (+1, drawn later) or down (-1) in the
// stacking order. Next to a layer with the same z it swaps list positions; next to a layer with
// another z it takes that layer's z and goes just past it. Only the moved layer changes, so
// moving one layer never pushes another across the scene text.
export const moveInStack = (layers: Layer[], index: number, delta: 1 | -1): Layer[] => {
  const { below, above } = splitLayers(layers);
  const order = [...below, ...above];
  const at = order.indexOf(layers[index]);
  const other = order[at + delta];
  if (!other) return layers;
  const moved = layers[index];
  const z = other.z ?? 0;
  const rest = layers.filter((_, i) => i !== index);
  const j = rest.indexOf(other);
  const next: Layer = (moved.z ?? 0) === z ? moved : z === 0 ? withoutZ(moved) : { ...moved, z };
  rest.splice(delta === 1 ? j + 1 : j, 0, next);
  return rest;
};

const withoutZ = (l: Layer): Layer => {
  const { z: _z, ...rest } = l;
  return rest as Layer;
};

const isColor = (c: unknown): boolean => typeof c === "string" && /^#([0-9a-fA-F]{3}|[0-9a-fA-F]{6}|[0-9a-fA-F]{8})$/.test(c);
const assetProblem = (p: string): string | null => {
  if (!p.trim()) return "has no file";
  if (/^[/~]/.test(p) || /^[^/]*:/.test(p) || p.split("/").some((s) => s === "" || s === "." || s === "..") || /[\\%]/.test(p)) {
    return "must be a path inside public/ (no URL, no leading /, no ..)";
  }
  return null;
};

// Rough height (fraction of the frame height) of a layer without `height`, for the drag outline.
export const outlineHeight = (layer: Layer): number => {
  if (layer.height != null) return layer.height;
  switch (layer.kind) {
    case "text":
      return (layer.font_size ?? 0.06) * (layer.line_height ?? 1.2);
    case "lower_third": {
      const f = layer.font_size ?? 0.045;
      return f * (2 * 0.38 + 1.15 + (layer.subtitle ? 0.62 * (1.3 + 0.25) : 0));
    }
    default:
      return 0.05;
  }
};

// Kinds whose box may leave out `height` (mirrors LayerContent::auto_height).
const AUTO_HEIGHT = new Set<LayerKind>(["text", "lower_third"]);

const colors = (name: string, out: string[], list: (string | null | undefined)[]) => {
  if (list.some((c) => c != null && !isColor(c))) out.push(`${name}: colors must be #rgb, #rrggbb or #rrggbbaa.`);
};

/** Problems with the scene's layers, in words for the problem bar. */
export const checkLayers = (s: Scene): string[] => {
  const out: string[] = [];
  const layers = s.layers ?? [];
  if (layers.length > MAX_LAYERS) out.push(`Scene “${s.id}” has ${layers.length} layers; the limit is ${MAX_LAYERS}.`);
  const ids = new Set<string>();
  for (const l of layers) {
    const name = `Scene “${s.id}” layer “${l.id}”`;
    if (ids.has(l.id)) out.push(`${name}: duplicate id.`);
    ids.add(l.id);
    if (!/^[A-Za-z0-9_-]{1,64}$/.test(l.id)) out.push(`${name}: id must be letters, digits, - or _.`);
    if (!(l.x >= -1 && l.x <= 2 && l.y >= -1 && l.y <= 2)) out.push(`${name}: x and y must be between -1 and 2.`);
    const line = l.kind === "shape" && l.shape === "line";
    const sizeOk = (v: number) => (line ? v >= 0 && v <= 4 : v > 0 && v <= 4);
    if (!sizeOk(l.width)) out.push(`${name}: width must be above 0 and at most 4.`);
    if (l.height == null ? !AUTO_HEIGHT.has(l.kind) : !sizeOk(l.height)) out.push(`${name}: needs a height above 0 and at most 4.`);
    const end = l.end_ms ?? s.duration_ms;
    if (!((l.start_ms ?? 0) < end && end <= s.duration_ms)) out.push(`${name}: start must be before end, inside the scene.`);
    const anim = (l.enter ? l.enter.duration_ms ?? 400 : 0) + (l.exit ? l.exit.duration_ms ?? 400 : 0);
    if (anim > end - (l.start_ms ?? 0)) out.push(`${name}: enter and exit are longer than the layer is shown.`);
    if (l.opacity !== undefined && !(l.opacity >= 0 && l.opacity <= 1)) out.push(`${name}: opacity must be between 0 and 1.`);
    switch (l.kind) {
      case "text":
        if (!l.text.trim()) out.push(`${name}: the text is empty.`);
        if (l.text.length > MAX_LAYER_TEXT_CHARS) out.push(`${name}: the text is over ${MAX_LAYER_TEXT_CHARS} characters.`);
        if (l.color !== undefined && !isColor(l.color)) out.push(`${name}: color must be #rgb, #rrggbb or #rrggbbaa.`);
        if (l.background && !isColor(l.background.color)) out.push(`${name}: background color must be #rgb, #rrggbb or #rrggbbaa.`);
        if (l.typing_cps !== undefined && !(l.typing_cps >= 1 && l.typing_cps <= 200)) out.push(`${name}: typing speed must be 1 to 200 characters per second.`);
        break;
      case "shape":
        if (!l.fill && !l.stroke) out.push(`${name}: needs a fill or a stroke color.`);
        if ((l.fill && !isColor(l.fill)) || (l.stroke && !isColor(l.stroke))) out.push(`${name}: colors must be #rgb, #rrggbb or #rrggbbaa.`);
        if (line && !l.stroke) out.push(`${name}: a line needs a stroke color.`);
        break;
      case "image": {
        const p = assetProblem(l.src);
        if (p) out.push(`${name}: image ${p}.`);
        break;
      }
      case "icon": {
        const p = assetProblem(l.src) ?? (/\.svg$/i.test(l.src) ? null : "must be an .svg file");
        if (p) out.push(`${name}: icon ${p}.`);
        if (l.color !== undefined && !isColor(l.color)) out.push(`${name}: color must be #rgb, #rrggbb or #rrggbbaa.`);
        break;
      }
      case "lower_third":
        if (!l.title.trim()) out.push(`${name}: the title is empty.`);
        if ([...l.title].length > 120) out.push(`${name}: the title is over 120 characters.`);
        if (l.subtitle && [...l.subtitle].length > 160) out.push(`${name}: the subtitle is over 160 characters.`);
        if (l.font_size !== undefined && !(l.font_size > 0 && l.font_size <= 0.2)) out.push(`${name}: font size must be above 0 and at most 0.2.`);
        colors(name, out, [l.color, l.subtitle_color, l.accent_color, l.background_color]);
        break;
    }
  }
  return out;
};

// Keeps layer timing inside a scene whose length changed.
export const clampLayers = (layers: Layer[] | undefined, sceneMs: number): Layer[] | undefined => {
  if (!layers?.some((l) => (l.end_ms ?? 0) > sceneMs || (l.start_ms ?? 0) >= sceneMs || l.keyframes?.some((k) => k.at_ms > sceneMs))) {
    return layers;
  }
  return layers.map((l) => {
    let next: Layer = l;
    if ((next.start_ms ?? 0) >= sceneMs) next = { ...next, start_ms: 0 };
    if (next.end_ms != null && next.end_ms > sceneMs) next = { ...next, end_ms: sceneMs };
    if (next.keyframes?.some((k) => k.at_ms > sceneMs)) {
      const kept = [];
      for (const k of next.keyframes) {
        const at = Math.min(k.at_ms, sceneMs);
        if (kept.length && kept[kept.length - 1].at_ms >= at) continue;
        kept.push({ ...k, at_ms: at });
      }
      next = { ...next, keyframes: kept };
    }
    return next;
  });
};
