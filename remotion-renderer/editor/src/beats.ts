import { invoke } from "@tauri-apps/api/core";
import { inTauri } from "./project";
import type { Music } from "../../src/types/timeline";

/** What `vimmer beats --json` reports for a music file (beat times from the start of the file). */
export type DetectedBeats = {
  source: string;
  bpm: number;
  confidence: number;
  low_confidence: boolean;
  downbeat: number;
  duration_ms: number;
  beats_ms: number[];
};

type BeatsReply = ({ ok: true } & Omit<DetectedBeats, "source">) | { ok: false; error: string };

const BEATS_ENDPOINT = "/__vimmer/beats";

/**
 * Detects the beats of the music file at `source` (public-relative) with the same Rust code and
 * cache a render uses: `vimmer beats --json` through the dev server, or the desktop shell.
 */
export const detectBeats = async (source: string): Promise<DetectedBeats | { error: string }> => {
  try {
    const reply = inTauri()
      ? await invoke<BeatsReply>("detect_beats", { source })
      : await fetch(BEATS_ENDPOINT, {
          method: "POST",
          headers: { "content-type": "application/json" },
          body: JSON.stringify({ source }),
        }).then((r) => {
          if (r.status === 404) return { ok: false, error: "Beat detection needs the editor dev server (npm run editor) or the desktop app." } as BeatsReply;
          return r.json() as Promise<BeatsReply>;
        });
    if (!reply.ok) return { error: reply.error };
    const { ok: _ok, ...rest } = reply;
    return { source, ...rest };
  } catch (e) {
    return { error: `Beat detection is unavailable: ${String(e)}` };
  }
};

export type BeatMark = { ms: number; bar: boolean };

/**
 * Beat markers in video time for the ruler: the detected beats (or the hand-written grid in
 * `music.beats`), repeated each time the music loops, every `beat_sync.every`-th one counted from
 * the downbeat. Mirrors `beat_grid` / `manual_beats` in rust-core/crates/vimmer-dsl/src/beatsync.rs.
 */
export const beatMarks = (music: Music | null | undefined, detected: DetectedBeats | null, totalMs: number): BeatMark[] => {
  if (!music) return [];
  let beats: number[];
  let downbeat = 0;
  let fileMs: number;
  if (music.beats && music.beats.bpm > 0) {
    fileMs = detected && detected.source === music.source ? detected.duration_ms : Number.POSITIVE_INFINITY;
    const period = 60_000 / music.beats.bpm;
    beats = [];
    for (let t = music.beats.offset_ms ?? 0; t < Math.min(fileMs, totalMs + period); t += period) beats.push(t);
  } else if (detected && detected.source === music.source) {
    beats = detected.beats_ms;
    downbeat = detected.downbeat;
    fileMs = detected.duration_ms;
  } else {
    return [];
  }
  const every = music.beat_sync?.every ?? 1;
  const out: BeatMark[] = [];
  const step = beats
    .map((b, i) => ({ b, i }))
    .filter(({ i }) => (((i - downbeat) % every) + every) % every === 0)
    .map(({ b, i }) => ({ b, bar: (((i - downbeat) % 4) + 4) % 4 === 0 }));
  if (step.length === 0 || !(fileMs > 0)) return out;
  for (let base = 0; base <= totalMs; base += fileMs) {
    for (const { b, bar } of step) {
      if (base + b > totalMs) return out;
      out.push({ ms: base + b, bar });
    }
    if (!Number.isFinite(fileMs)) break;
  }
  return out;
};

/** The sound effects that ship in public/assets/sfx (see the README there). */
export const BUNDLED_SFX: { src: string; label: string }[] = [
  { src: "assets/sfx/whoosh.wav", label: "Whoosh" },
  { src: "assets/sfx/swoosh-down.wav", label: "Swoosh down" },
  { src: "assets/sfx/pop.wav", label: "Pop" },
  { src: "assets/sfx/click.wav", label: "Click" },
  { src: "assets/sfx/ding.wav", label: "Ding" },
  { src: "assets/sfx/riser.wav", label: "Riser (2 s)" },
];
