import { useEffect, useMemo, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import type { Brand, Scene, SceneTemplate, VideoTimeline } from "../../src/types/timeline";
import { inTauri } from "./project";
// Written by the Rust tests from `vimmer_dsl::template::CATALOG` (one source of truth): the
// templates, their params and limits. Regenerate: VIMMER_UPDATE_CATALOG=1 cargo test -p vimmer-dsl catalog
import catalogJson from "./template-catalog.json";

export type ParamSpec = {
  name: string;
  label: string;
  required: boolean;
  help: string;
} & (
  | { type: "text"; max_chars: number }
  | { type: "lines"; min_items: number; max_items: number; max_chars: number }
  | { type: "bool"; default: boolean }
);

export type TemplateInfo = {
  name: string;
  label: string;
  description: string;
  default_ms: number;
  params: ParamSpec[];
};

export const CATALOG = catalogJson as TemplateInfo[];

export const templateInfo = (name: string): TemplateInfo | undefined => CATALOG.find((t) => t.name === name);

/** Example params so an inserted template shows something right away. */
const SAMPLE: Record<string, SceneTemplate["params"]> = {
  intro: { title: "Your title here", subtitle: "A short subtitle" },
  outro: { cta: "Thanks for watching", lines: ["@yourhandle"] },
  title_card: { number: "01", title: "Section title" },
  quote: { quote: "A short, memorable quote.", author: "Someone" },
  list: { title: "Three things", items: ["First point", "Second point", "Third point"] },
};

/** A new scene made from template `name` (an unused id, the template's default length). */
export const templateScene = (name: string, timeline: VideoTimeline, params?: SceneTemplate["params"]): Scene => {
  const info = templateInfo(name);
  const ids = new Set(timeline.scenes.map((s) => s.id));
  let n = 1;
  while (ids.has(`${name.replace("_", "-")}-${n}`)) n += 1;
  return {
    id: `${name.replace("_", "-")}-${n}`,
    duration_ms: info?.default_ms ?? 4000,
    text: null,
    highlights: [],
    image: null,
    motion: "static",
    audio: null,
    template: { name, params: params ?? SAMPLE[name] ?? {} },
  };
};

/** Problems of a template's params, as the Rust expansion reports them (lengths, required). */
export const templateProblems = (tpl: SceneTemplate): string[] => {
  const info = templateInfo(tpl.name);
  if (!info) return [`Unknown template “${tpl.name}”.`];
  const out: string[] = [];
  const params = tpl.params ?? {};
  for (const key of Object.keys(params)) {
    if (!info.params.some((p) => p.name === key)) out.push(`Unknown param “${key}”.`);
  }
  for (const p of info.params) {
    const v = params[p.name];
    if (p.type === "text") {
      const s = v == null ? "" : String(v).trim();
      if (p.required && !s) out.push(`${p.label} is required.`);
      if ([...s].length > p.max_chars) out.push(`${p.label} is ${[...s].length} characters; the limit is ${p.max_chars}.`);
    } else if (p.type === "lines") {
      const list = Array.isArray(v) ? v : [];
      if (p.required && list.length === 0) out.push(`${p.label}: at least ${p.min_items} needed.`);
      if (list.length > p.max_items) out.push(`${p.label}: at most ${p.max_items}.`);
      list.forEach((l, i) => {
        if ([...l].length > p.max_chars) out.push(`${p.label} ${i + 1} is longer than ${p.max_chars} characters.`);
        if (!l.trim()) out.push(`${p.label} ${i + 1} is empty.`);
      });
    }
  }
  return out;
};

export type Expanded = { ok: true; timeline: VideoTimeline } | { ok: false; errors: string[] };

const EXPAND_ENDPOINT = "/__vimmer/expand";

/**
 * Expands template scenes with the Rust code the renderer uses (`vimmer expand` through the dev
 * server, or the desktop shell's command), so the preview matches the render exactly.
 */
export const expandTimeline = async (timeline: VideoTimeline): Promise<Expanded> => {
  const timelineJson = JSON.stringify(timeline);
  try {
    if (inTauri()) return await invoke<Expanded>("expand_timeline", { timelineJson });
    const res = await fetch(EXPAND_ENDPOINT, {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ timelineJson }),
    });
    if (res.status === 404) return { ok: false, errors: ["Template preview needs the editor dev server (npm run editor) or the desktop app."] };
    return (await res.json()) as Expanded;
  } catch (e) {
    return { ok: false, errors: [`Template preview is unavailable: ${String(e)}`] };
  }
};

const hasTemplates = (t: VideoTimeline): boolean => t.scenes.some((s) => s.template);

/**
 * The timeline to preview: unchanged without template scenes; otherwise the expanded one, fetched
 * shortly after each change (the last good expansion stays on screen meanwhile). Until the first
 * expansion arrives, template scenes show only the brand background.
 */
export const usePreviewTimeline = (timeline: VideoTimeline): { timeline: VideoTimeline; errors: string[] } => {
  const [result, setResult] = useState<{ source: VideoTimeline; expanded: VideoTimeline | null; errors: string[] }>({
    source: timeline,
    expanded: null,
    errors: [],
  });
  const needs = hasTemplates(timeline);
  useEffect(() => {
    if (!needs) return;
    let live = true;
    const timer = window.setTimeout(() => {
      void expandTimeline(timeline).then((r) => {
        if (!live) return;
        if (r.ok) setResult({ source: timeline, expanded: r.timeline, errors: [] });
        else setResult((prev) => ({ ...prev, errors: r.errors }));
      });
    }, 150);
    return () => {
      live = false;
      window.clearTimeout(timer);
    };
  }, [timeline, needs]);
  // Memoized: the Player re-renders on every new `inputProps` object, so the merged timeline must
  // keep its identity until the document or the expansion changes.
  const merged = useMemo(() => {
    if (!needs) return timeline;
    // An expansion of an older document is used only while scene lengths still match (so the
    // frame count is right); template scenes come from it, everything else is current.
    const expanded = result.expanded;
    const usable =
      expanded &&
      expanded.scenes.length === timeline.scenes.length &&
      expanded.scenes.every((s, i) => s.duration_ms === timeline.scenes[i].duration_ms);
    if (!usable) return timeline;
    const scenes = timeline.scenes.map((s, i) => (s.template && expanded.scenes[i]?.id === s.id ? expanded.scenes[i] : s));
    return { ...timeline, scenes };
  }, [timeline, needs, result.expanded]);
  return { timeline: merged, errors: needs ? result.errors : NO_ERRORS };
};

const NO_ERRORS: string[] = [];

/** Parses a brand file (the `brand` object on its own). */
export const parseBrand = (text: string): Brand => {
  const v: unknown = JSON.parse(text);
  if (!v || typeof v !== "object" || Array.isArray(v)) throw new Error("A brand file is a JSON object.");
  const b = v as Record<string, unknown>;
  if ("fps" in b || "scenes" in b) throw new Error("This is a timeline, not a brand file.");
  for (const key of Object.keys(b)) {
    if (!["colors", "fonts", "logo"].includes(key)) throw new Error(`Unknown brand field “${key}”.`);
  }
  return b as Brand;
};
