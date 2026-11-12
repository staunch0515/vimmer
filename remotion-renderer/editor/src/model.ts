import type { Scene, SubtitleCue, VideoTimeline } from "../../src/types/timeline";
import { totalFrames } from "../../src/layout";

export const MAX_DURATION_MS = 200_000;
export const ALLOWED_RESOLUTIONS = [
  { width: 1920, height: 1080, label: "Landscape 1080p" },
  { width: 1080, height: 1920, label: "Vertical 1080p" },
] as const;

export const emptyScene = (index: number): Scene => ({
  id: `scene-${String(index + 1).padStart(2, "0")}`,
  duration_ms: 5000,
  text: "New scene",
  highlights: [],
  image: null,
  motion: "static",
  audio: null,
});

export const totalMs = (t: VideoTimeline): number =>
  t.scenes.reduce((sum, s) => sum + s.duration_ms, 0);

// Mirrors the Rust `validate` rules so problems show up before export.
export const checkTimeline = (t: VideoTimeline): string[] => {
  const problems: string[] = [];
  if (t.scenes.length === 0) problems.push("Timeline has no scenes.");
  if (t.fps !== 30) problems.push(`FPS is ${t.fps}; the renderer targets 30.`);
  if (!ALLOWED_RESOLUTIONS.some((r) => r.width === t.resolution.width && r.height === t.resolution.height)) {
    problems.push(`Resolution ${t.resolution.width}x${t.resolution.height} is not supported.`);
  }
  const ids = new Set<string>();
  for (const s of t.scenes) {
    if (ids.has(s.id)) problems.push(`Duplicate scene id “${s.id}”.`);
    ids.add(s.id);
    if (s.duration_ms <= 0) problems.push(`Scene “${s.id}” has zero duration.`);
    if (!s.text?.trim() && !s.image) problems.push(`Scene “${s.id}” needs text or an image.`);
  }
  const total = totalMs(t);
  if (total > MAX_DURATION_MS) problems.push(`Timeline is ${(total / 1000).toFixed(1)} s; the limit is 200 s.`);
  t.subtitles?.forEach((c, i) => {
    if (c.start_ms >= c.end_ms) problems.push(`Subtitle ${i + 1} must start before it ends.`);
    else if (c.end_ms > total) problems.push(`Subtitle ${i + 1} runs past the end of the timeline.`);
    if (!c.text.trim()) problems.push(`Subtitle ${i + 1} is empty.`);
  });
  return problems;
};

export const frameCount = (t: VideoTimeline): number => Math.max(1, totalFrames(t));

export const timecode = (ms: number): string => {
  const safe = Math.max(0, ms);
  const m = Math.floor(safe / 60_000);
  const s = Math.floor((safe % 60_000) / 1000);
  const frac = Math.floor((safe % 1000) / 33.33);
  return `${String(m).padStart(2, "0")}:${String(s).padStart(2, "0")}:${String(frac).padStart(2, "0")}`;
};

export const frameToMs = (frame: number, fps: number): number => Math.round((frame * 1000) / fps);

export type Selection =
  | { kind: "scene"; index: number }
  | { kind: "subtitle"; index: number }
  | { kind: "none" };

export const sortCues = (cues: SubtitleCue[]): SubtitleCue[] =>
  [...cues].sort((a, b) => a.start_ms - b.start_ms);
