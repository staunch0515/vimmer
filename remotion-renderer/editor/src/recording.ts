// Recording (录音): what is pure about it, so it is unit-tested with Node
// (test/editor-recording.test.ts). The microphone side is in useRecorder.ts.
//
// Recordings are WAV, 48 kHz, mono, 16-bit: what the render mixes without conversion, and what
// whisper reads for subtitles. The browser's capture rate is resampled here when it differs.

import type { Scene, VideoTimeline } from "../../src/types/timeline";

export const RECORD_RATE = 48_000;
export const COUNTDOWN_S = 3;
/** Longest recording (the render's whole-video limit). */
export const MAX_RECORD_MS = 200_000;

/** Linear resampling of mono samples from `from` Hz to `to` Hz. */
export const resample = (input: Float32Array, from: number, to: number): Float32Array => {
  if (from === to || input.length === 0) return input;
  const n = Math.max(1, Math.round((input.length * to) / from));
  const out = new Float32Array(n);
  const step = from / to;
  for (let i = 0; i < n; i++) {
    const x = i * step;
    const k = Math.floor(x);
    const f = x - k;
    const a = input[Math.min(k, input.length - 1)];
    const b = input[Math.min(k + 1, input.length - 1)];
    out[i] = a + (b - a) * f;
  }
  return out;
};

/** Joins captured blocks into one buffer. */
export const joinBlocks = (blocks: Float32Array[]): Float32Array => {
  const out = new Float32Array(blocks.reduce((n, b) => n + b.length, 0));
  let at = 0;
  for (const b of blocks) {
    out.set(b, at);
    at += b.length;
  }
  return out;
};

/** A 16-bit PCM mono WAV file of `samples` (-1..1, clamped) at `rate`. */
export const encodeWav = (samples: Float32Array, rate = RECORD_RATE): Uint8Array => {
  const bytes = new Uint8Array(44 + samples.length * 2);
  const v = new DataView(bytes.buffer);
  const ascii = (at: number, s: string) => [...s].forEach((c, i) => v.setUint8(at + i, c.charCodeAt(0)));
  ascii(0, "RIFF");
  v.setUint32(4, 36 + samples.length * 2, true);
  ascii(8, "WAVE");
  ascii(12, "fmt ");
  v.setUint32(16, 16, true); // PCM chunk size
  v.setUint16(20, 1, true); // PCM
  v.setUint16(22, 1, true); // mono
  v.setUint32(24, rate, true);
  v.setUint32(28, rate * 2, true); // byte rate
  v.setUint16(32, 2, true); // block align
  v.setUint16(34, 16, true); // bits
  ascii(36, "data");
  v.setUint32(40, samples.length * 2, true);
  for (let i = 0; i < samples.length; i++) {
    const s = Math.max(-1, Math.min(1, samples[i]));
    v.setInt16(44 + i * 2, s < 0 ? Math.round(s * 0x8000) : Math.round(s * 0x7fff), true);
  }
  return bytes;
};

/** Format and length of a PCM WAV, or null when it is not one. */
export const wavInfo = (bytes: Uint8Array): { rate: number; channels: number; bits: number; ms: number } | null => {
  if (bytes.length < 44) return null;
  const v = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength);
  const tag = (at: number) => String.fromCharCode(...bytes.subarray(at, at + 4));
  if (tag(0) !== "RIFF" || tag(8) !== "WAVE" || tag(12) !== "fmt ") return null;
  const channels = v.getUint16(22, true);
  const rate = v.getUint32(24, true);
  const bits = v.getUint16(34, true);
  const data = v.getUint32(40, true);
  if (!channels || !rate || !bits) return null;
  return { rate, channels, bits, ms: Math.round((data / (channels * (bits / 8)) / rate) * 1000) };
};

/** Peak and RMS of a block in dBFS (-Infinity for silence), for the level meter. */
export const levels = (block: Float32Array): { peakDb: number; rmsDb: number } => {
  let peak = 0;
  let sum = 0;
  for (let i = 0; i < block.length; i++) {
    const a = Math.abs(block[i]);
    if (a > peak) peak = a;
    sum += block[i] * block[i];
  }
  const rms = block.length ? Math.sqrt(sum / block.length) : 0;
  const db = (x: number) => (x > 0 ? 20 * Math.log10(x) : Number.NEGATIVE_INFINITY);
  return { peakDb: db(peak), rmsDb: db(rms) };
};

/** Meter fill 0..1 for a level in dBFS: -60 dB empty, 0 dB full. */
export const meterFill = (db: number): number => (Number.isFinite(db) ? Math.max(0, Math.min(1, (db + 60) / 60)) : 0);

/** File stem of a recording: the scene id (or `voiceover`), made safe for a file name. */
export const recordingStem = (name: string): string =>
  name
    .normalize("NFKD")
    .replace(/[^A-Za-z0-9_-]+/g, "-")
    .replace(/^-+|-+$/g, "")
    .slice(0, 40) || "take";

/** What a scene's speaker reads: its spoken line (台词) when set, else its text. */
export const scriptOf = (scene: Scene): string => {
  const line = scene.narration?.text?.trim();
  return line || scene.text?.trim() || "";
};

export type PrompterLine = { index: number; id: string; text: string; startMs: number; endMs: number };

/** The teleprompter's lines: each scene's script from `fromMs` on, with its time on the video. */
export const prompterLines = (t: VideoTimeline, fromMs = 0): PrompterLine[] => {
  const out: PrompterLine[] = [];
  let at = 0;
  t.scenes.forEach((s, index) => {
    const end = at + s.duration_ms;
    if (end > fromMs) out.push({ index, id: s.id, text: scriptOf(s), startMs: at, endMs: end });
    at = end;
  });
  return out;
};

/** The line being read `elapsedMs` after a recording that started at `fromMs`. */
export const currentLine = (lines: PrompterLine[], fromMs: number, elapsedMs: number): number => {
  const now = fromMs + elapsedMs;
  const k = lines.findIndex((l) => now < l.endMs);
  return k === -1 ? lines.length - 1 : k;
};

/** Base64 of bytes (for sending a WAV to the desktop command or the dev server). */
export const toBase64 = (bytes: Uint8Array): string => {
  let s = "";
  for (let i = 0; i < bytes.length; i += 0x8000) s += String.fromCharCode(...bytes.subarray(i, i + 0x8000));
  return btoa(s);
};
