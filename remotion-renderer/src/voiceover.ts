// The voice-over track (旁白音轨) in frames, for the preview: where each clip starts, how much of
// its file is skipped and how long it plays. Pure; mirrors `place_voiceover` in
// rust-core/crates/vimmer-scheduler/src/audio.rs (start on the frame of `start_ms`).
import { createContext } from "react";
import type { VoiceoverClip } from "./types/timeline";
import { msToFrames } from "./layout";

// Lengths of audio files in ms, by timeline path, as far as the host knows them (the editor
// measures them). Without a length a clip plays to the end of its file.
export const AudioLengthsContext = createContext<Record<string, number> | null>(null);

export const MAX_VOICEOVER_GAIN = 4;

export const clipGain = (c: VoiceoverClip): number => c.gain ?? 1;

/** How long the clip plays for a file `fileMs` long (its trims taken off). */
export const playedMs = (c: VoiceoverClip, fileMs: number): number =>
  Math.max(0, fileMs - (c.trim_start_ms ?? 0) - (c.trim_end_ms ?? 0));

/** Where the clip ends on the timeline, for a file `fileMs` long. */
export const clipEndMs = (c: VoiceoverClip, fileMs: number): number => c.start_ms + playedMs(c, fileMs);

export type ClipFrames = {
  from: number;
  trimBefore: number;
  // Frames played; null = to the end of the file (length unknown).
  durationInFrames: number | null;
};

export const clipFrames = (c: VoiceoverClip, fps: number, fileMs: number | undefined): ClipFrames => ({
  from: msToFrames(c.start_ms, fps),
  trimBefore: msToFrames(c.trim_start_ms ?? 0, fps),
  durationInFrames: fileMs === undefined ? null : msToFrames(playedMs(c, fileMs), fps),
});
