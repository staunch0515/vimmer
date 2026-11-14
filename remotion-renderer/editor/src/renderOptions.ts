// Render panel options and the pre-render check, as plain data and pure functions (no Tauri,
// no React), so they can be tested with node. Only type imports here.
import type { VideoTimeline } from "../../src/types/timeline";
import type { Selection } from "./model";

export type TtsName = "say" | "kokoro";
export type AutoSubtitlesMode = "off" | "on" | "all";

/** What the panel remembers per project. `null` means "the default". */
export type RenderOptions = {
  /** `null`: the render's default engine ($VIMMER_TTS, else say). */
  tts: TtsName | null;
  autoSubtitles: AutoSubtitlesMode;
  wordSubtitles: boolean;
  chunkMs: number | null;
  preprocessJobs: number | null;
  renderJobs: number | null;
  /** A whisper code such as zh, en, ja, or `auto`; empty guesses from each scene's text. */
  whisperLanguage: string;
};

// Mirrors `RenderDefaults` in desktop/src-tauri/src/render.rs.
export type RenderDefaults = {
  tts: TtsName;
  ttsFromEnv: boolean;
  chunkMs: number;
  preprocessJobs: number;
  renderJobs: number;
};

/** Used until the desktop shell answers (and in the browser). Same values as the CLI. */
export const FALLBACK_DEFAULTS: RenderDefaults = { tts: "say", ttsFromEnv: false, chunkMs: 30_000, preprocessJobs: 4, renderJobs: 2 };

/** The CLI without flags. */
export const DEFAULT_OPTIONS: RenderOptions = {
  tts: null,
  autoSubtitles: "off",
  wordSubtitles: true,
  chunkMs: null,
  preprocessJobs: null,
  renderJobs: null,
  whisperLanguage: "",
};

export const AUTO_SUBTITLE_MODES: { value: AutoSubtitlesMode; label: string; help: string }[] = [
  { value: "off", label: "Off", help: "Only the subtitles already in the timeline (they still get word timing)." },
  { value: "on", label: "On", help: "Adds subtitles for narration that is not already shown on screen as text." },
  { value: "all", label: "All", help: "Subtitles every narrated scene, even when its text is on screen (captions)." },
];

export const TTS_ENGINES: { value: TtsName; label: string }[] = [
  { value: "say", label: "say (macOS)" },
  { value: "kokoro", label: "Kokoro (local neural)" },
];

/** What `start_render` receives as `options`; mirrors `PanelOptions` in render.rs. */
export const toPanelArgs = (o: RenderOptions, d: RenderDefaults) => ({
  tts: o.tts ?? d.tts,
  autoSubtitles: o.autoSubtitles,
  wordSubtitles: o.wordSubtitles,
  chunkMs: o.chunkMs,
  preprocessJobs: o.preprocessJobs,
  renderJobs: o.renderJobs,
  whisperLanguage: o.whisperLanguage.trim() || null,
});

// Remembered per project in local storage, keyed by the project's path ("untitled" before it
// is saved). Not in the timeline file: these are choices about this machine's render (is Kokoro
// installed here, how many cores), and the timeline is the document the CLI and others read.
const KEY_PREFIX = "vimmer-render-options:";
export const optionsKey = (path: string | null): string => KEY_PREFIX + (path ?? "untitled");

type Store = Pick<Storage, "getItem" | "setItem">;

const oneOf = <T extends string>(v: unknown, values: readonly T[], fallback: T): T =>
  values.includes(v as T) ? (v as T) : fallback;
const count = (v: unknown, min: number): number | null =>
  typeof v === "number" && Number.isFinite(v) && v >= min ? Math.round(v) : null;

/** Reads stored options, keeping only valid values (anything else falls back to the default). */
export const parseOptions = (raw: string | null): RenderOptions => {
  let v: Record<string, unknown> = {};
  try {
    const parsed: unknown = raw ? JSON.parse(raw) : {};
    if (parsed && typeof parsed === "object") v = parsed as Record<string, unknown>;
  } catch {
    /* corrupt entry: defaults */
  }
  return {
    tts: v.tts === "say" || v.tts === "kokoro" ? v.tts : null,
    autoSubtitles: oneOf(v.autoSubtitles, ["off", "on", "all"] as const, DEFAULT_OPTIONS.autoSubtitles),
    wordSubtitles: typeof v.wordSubtitles === "boolean" ? v.wordSubtitles : DEFAULT_OPTIONS.wordSubtitles,
    chunkMs: count(v.chunkMs, 1000),
    preprocessJobs: count(v.preprocessJobs, 1),
    renderJobs: count(v.renderJobs, 1),
    whisperLanguage: typeof v.whisperLanguage === "string" ? v.whisperLanguage : "",
  };
};

export const loadOptions = (store: Store | null, path: string | null): RenderOptions => {
  try {
    return parseOptions(store?.getItem(optionsKey(path)) ?? null);
  } catch {
    return { ...DEFAULT_OPTIONS };
  }
};

/** Whether `path` has remembered options (a project opened for the first time has none). */
export const hasOptions = (store: Store | null, path: string | null): boolean => {
  try {
    return store?.getItem(optionsKey(path)) != null;
  } catch {
    return false;
  }
};

export const saveOptions = (store: Store | null, path: string | null, o: RenderOptions): void => {
  try {
    store?.setItem(optionsKey(path), JSON.stringify(o));
  } catch {
    /* storage unavailable: the choice lasts for this session only */
  }
};

// ---- Pre-render check: mirrors the `vimmer-check/1` JSON report ----

export type Severity = "error" | "warning" | "info";

export type CheckFinding = {
  severity: Severity;
  code: string;
  scene_id?: string;
  cue_index?: number;
  time_ms?: number;
  message: string;
  suggestion: { text: string; path?: string; value?: unknown };
};

export type CheckReport = {
  schema: string;
  summary: { errors: number; warnings: number; infos: number };
  findings: CheckFinding[];
};

/** The check as the panel shows it. `key` says which timeline and engine it was run for. */
export type CheckView =
  | { status: "idle" }
  | { status: "running"; key: string }
  | { status: "done"; key: string; report: CheckReport }
  | { status: "failed"; key: string; error: string };

export const checkKey = (timelineJson: string, tts: TtsName): string => `${tts}\n${timelineJson}`;

/** Whether a Render click must be confirmed with "Render anyway". Only errors block. */
export const blocksRender = (report: CheckReport): boolean => report.findings.some((f) => f.severity === "error");

const RANK: Record<Severity, number> = { error: 0, warning: 1, info: 2 };

/** Errors first, then warnings, then info; the report's own order (timeline order) within each. */
export const sortFindings = (findings: CheckFinding[]): CheckFinding[] =>
  findings.map((f, i) => ({ f, i })).sort((a, b) => RANK[a.f.severity] - RANK[b.f.severity] || a.i - b.i).map(({ f }) => f);

/** "2 errors · 1 warning · 3 info", or "No problems found". */
export const summaryText = (r: CheckReport): string => {
  const parts: string[] = [];
  const { errors, warnings, infos } = r.summary;
  if (errors) parts.push(`${errors} error${errors > 1 ? "s" : ""}`);
  if (warnings) parts.push(`${warnings} warning${warnings > 1 ? "s" : ""}`);
  if (infos) parts.push(`${infos} info`);
  return parts.length ? parts.join(" · ") : "No problems found";
};

/** Splits an RFC 6901 JSON pointer into its tokens. */
export const pointerTokens = (pointer: string): string[] =>
  pointer === "" ? [] : pointer.replace(/^\//, "").split("/").map((t) => t.replace(/~1/g, "/").replace(/~0/g, "~"));

const index = (token: string | undefined): number | null => (token !== undefined && /^\d+$/.test(token) ? Number(token) : null);

/**
 * The editor selection a finding is about, using the editor's existing `Selection`: the fix's
 * JSON pointer first (a layer, keyframe, scene, cue or the music), then the finding's scene id
 * or cue index. `null` when it names nothing the editor can select (the brand, say).
 */
export const findingSelection = (f: CheckFinding, t: VideoTimeline): Selection | null => {
  const tokens = f.suggestion.path ? pointerTokens(f.suggestion.path) : [];
  const [root, a, kind, b] = tokens;
  if (root === "scenes") {
    const sceneIndex = index(a);
    const scene = sceneIndex === null ? undefined : t.scenes[sceneIndex];
    if (scene && sceneIndex !== null) {
      const i = index(b);
      if (kind === "layers" && i !== null && scene.layers?.[i]) return { kind: "layer", sceneIndex, index: i };
      if (kind === "keyframes" && i !== null && scene.keyframes?.[i]) return { kind: "keyframe", sceneIndex, index: i };
      return { kind: "scene", index: sceneIndex };
    }
  }
  if (root === "subtitles") {
    const i = index(a);
    if (i !== null && t.subtitles[i]) return { kind: "subtitle", index: i };
  }
  if (root === "music" && t.music) return { kind: "music" };
  if (f.cue_index !== undefined && t.subtitles[f.cue_index]) return { kind: "subtitle", index: f.cue_index };
  if (f.scene_id !== undefined) {
    const i = t.scenes.findIndex((s) => s.id === f.scene_id);
    if (i >= 0) return { kind: "scene", index: i };
  }
  return null;
};
