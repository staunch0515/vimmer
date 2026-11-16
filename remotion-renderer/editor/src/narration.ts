// The spoken line (台词) of a scene's narration. Pure (type imports only), so it is unit-tested
// with Node (test/editor-narration.test.ts). Mirrors `Scene::spoken_text` in vimmer-dsl/src/model.rs.
import type { Narration, Scene, TextPage } from "../../src/types/timeline";
import type { TtsName } from "./renderOptions";

const said = (t: string | null | undefined): string | null => (t && t.trim() ? t : null);

/** What the narration says: its spoken line when set, else the scene text; null without narration. */
export const spokenText = (s: Pick<Scene, "text" | "narration">): string | null =>
  s.narration ? said(s.narration.text) ?? said(s.text) : null;

/**
 * `n` with the spoken line `line`. A blank line removes the field, so the narration speaks the
 * on-screen text again and the saved JSON is exactly what it was before a line was typed.
 */
export const withSpokenLine = (n: Narration, line: string): Narration => {
  const { text: _old, ...rest } = n;
  return line.trim() ? { ...rest, text: line } : rest;
};

/** `n` with another voice, keeping its spoken line. */
export const withVoice = (n: Narration, voice: string | null): Narration => ({ ...n, voice });

/** Mirrors the Rust rule: a spoken line that is blank, over a scene without text, says nothing. */
export const nothingToSpeak = (s: Pick<Scene, "text" | "narration" | "audio">): boolean =>
  !!s.narration && s.narration.text !== undefined && !s.audio && spokenText(s) === null;

// ---- Listening before a render ("试听") --------------------------------------------------------
// The narration is synthesized by the render's own code (desktop: `listen_narration`; browser dev
// server: `vimmer tts`) into the render's TTS cache, so these are only the editor's bookkeeping:
// which clip belongs to which scene as it is now, and what the preview plays.

export type TtsChoice = TtsName;

/** One synthesized clip, as `vimmer tts --json` / `listen_narration` return it. */
export type ListenClip = {
  scene_id: string;
  index: number;
  text: string;
  voice: string | null;
  /** Public-relative WAV, the very file a render attaches. */
  source: string;
  speech_ms: number;
  scene_ms: number;
  render_ms: number;
  cached: boolean;
};

export type ListenResult =
  | { ok: true; engine: TtsChoice; clips: ListenClip[]; warnings: string[] }
  | { ok: false; error: string };

/** The pause a render leaves after the speech before a scene may end (`TAIL_MS` in narrate.rs). */
export const TAIL_MS = 300;

const voiceOf = (s: Pick<Scene, "narration">): string | null => {
  const v = s.narration?.voice;
  return v && v.trim() ? v : null;
};

/**
 * What decides a scene's clip: its id, the engine asked for, its voice and its spoken text (the
 * render's cache key is made of the same things). Null when the render does not narrate it.
 */
export const listenKey = (s: Pick<Scene, "id" | "text" | "narration" | "audio">, engine: TtsChoice): string | null => {
  if (!s.narration || s.audio) return null;
  const text = spokenText(s);
  return text === null ? null : JSON.stringify([s.id, engine, voiceOf(s) ?? "", text]);
};

/** Clips heard so far, by `listenKey`. */
export type HeardClips = Readonly<Record<string, ListenClip>>;

/** `heard` plus the clips of a result (each keyed by what it was made from). */
export const rememberClips = (heard: HeardClips, engine: TtsChoice, clips: ListenClip[]): HeardClips => {
  const next: Record<string, ListenClip> = { ...heard };
  for (const c of clips) {
    next[JSON.stringify([c.scene_id, engine, c.voice ?? "", c.text])] = c;
  }
  return next;
};

/** The clip for the scene as it is now; null after its text, voice or id changed. */
export const clipFor = (heard: HeardClips, s: Pick<Scene, "id" | "text" | "narration" | "audio">, engine: TtsChoice): ListenClip | null => {
  const key = listenKey(s, engine);
  return key ? heard[key] ?? null : null;
};

/** The scene's length in a render with this speech: stretched to the speech plus the pause. */
export const renderLength = (sceneMs: number, speechMs: number): number => Math.max(sceneMs, speechMs + TAIL_MS);

const secs = (ms: number) => (ms / 1000).toFixed(1);

/** Two lengths in seconds, with enough digits to tell them apart ("3.00→3.01"). */
export const secondsPair = (a: number, b: number): [string, string] =>
  secs(a) === secs(b) && a !== b ? [(a / 1000).toFixed(2), (b / 1000).toFixed(2)] : [secs(a), secs(b)];

/** "念完需 4.2 秒，本场景 3.0 秒，渲染时会自动拉长到 4.5 秒" style hint. */
export const lengthHint = (sceneMs: number, speechMs: number): { text: string; stretched: boolean } => {
  const to = renderLength(sceneMs, speechMs);
  const [scene, longer] = secondsPair(sceneMs, to);
  return to > sceneMs
    ? { text: `念完需 ${secs(speechMs)} 秒，本场景 ${scene} 秒，渲染时会自动拉长到 ${longer} 秒`, stretched: true }
    : { text: `念完需 ${secs(speechMs)} 秒，本场景 ${secs(sceneMs)} 秒，时长够用`, stretched: false };
};

/** The preview with narration: each heard scene plays its clip and is as long as in the render. */
export type NarratedPreview<T extends { scenes: Scene[] }> = {
  timeline: T;
  /** Scenes the render will stretch: id, saved length, render length. */
  stretched: { id: string; from: number; to: number }[];
  /** Narrated scenes without a current clip (never heard, or edited since). */
  missing: string[];
  /** Start of each scene in the saved timeline and in the preview, in ms. */
  savedStarts: number[];
  previewStarts: number[];
  savedLengths: number[];
};

/**
 * The timeline as a render will play it: each narrated scene with a current clip gets that clip as
 * its audio and the render's length. The saved timeline is not touched (this is a copy for the
 * Player); scenes with their own audio keep it, exactly as a render does.
 */
export const withNarration = <T extends { scenes: Scene[] }>(
  t: T,
  heard: HeardClips,
  engine: TtsChoice,
  // Pages of a heard scene that speaks its own text, timed over its clip (src/textPages.ts
  // `heardPages`); empty for an unpaged scene. Missing: no pages are set.
  pager?: (scene: Scene, speechMs: number) => TextPage[],
): NarratedPreview<T> => {
  const stretched: NarratedPreview<T>["stretched"] = [];
  const missing: string[] = [];
  const savedStarts: number[] = [];
  const previewStarts: number[] = [];
  const savedLengths: number[] = [];
  let saved = 0;
  let preview = 0;
  const scenes = t.scenes.map((s) => {
    savedStarts.push(saved);
    previewStarts.push(preview);
    savedLengths.push(s.duration_ms);
    saved += s.duration_ms;
    const key = listenKey(s, engine);
    const clip = key ? heard[key] : undefined;
    if (key && !clip) missing.push(s.id);
    if (!clip) {
      preview += s.duration_ms;
      return s;
    }
    const to = renderLength(s.duration_ms, clip.speech_ms);
    if (to > s.duration_ms) stretched.push({ id: s.id, from: s.duration_ms, to });
    preview += to;
    const pages = pager && spokenText(s) === s.text ? pager(s, clip.speech_ms) : [];
    return {
      ...s,
      duration_ms: to,
      audio: { source: clip.source, offset_ms: 0 },
      ...(pages.length > 1 ? { text_pages: pages } : {}),
    };
  });
  return { timeline: { ...t, scenes }, stretched, missing, savedStarts, previewStarts, savedLengths };
};

const sceneAt = (starts: number[], ms: number): number => {
  let i = 0;
  while (i + 1 < starts.length && starts[i + 1] <= ms) i += 1;
  return i;
};

type TimeMap = { savedStarts: number[]; previewStarts: number[]; savedLengths: number[] };

/** A time of the saved timeline in the narrated preview (same place in the same scene). */
export const toPreviewMs = (p: TimeMap, ms: number): number => {
  if (!p.savedStarts.length) return ms;
  const i = sceneAt(p.savedStarts, ms);
  return p.previewStarts[i] + (ms - p.savedStarts[i]);
};

/** A time of the narrated preview in the saved timeline; the stretched tail of a scene maps to its end. */
export const toSavedMs = (p: TimeMap, ms: number): number => {
  if (!p.previewStarts.length) return ms;
  const i = sceneAt(p.previewStarts, ms);
  const into = Math.min(ms - p.previewStarts[i], Math.max(0, p.savedLengths[i] - 1));
  return p.savedStarts[i] + into;
};
