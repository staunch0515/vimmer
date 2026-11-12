// TypeScript mirror of the Rust `vimmer-dsl` model (rust-core/crates/vimmer-dsl/src/model.rs).
// Keep field names in sync: the JSON document is the contract between the two layers.

export type MotionType = "static" | "zoom_in" | "zoom_out" | "pan_left" | "pan_right";

export type AudioMapping = {
  source: string;
  offset_ms: number;
};

// Narration is synthesized from `text` at render time (local speech engine).
export type Narration = {
  voice: string | null;
};

export type Scene = {
  id: string;
  duration_ms: number;
  text: string | null;
  highlights: string[];
  image: string | null;
  motion: MotionType;
  audio: AudioMapping | null;
  narration?: Narration | null;
};

export type Resolution = {
  width: number;
  height: number;
};

export type SubtitleCue = {
  start_ms: number;
  end_ms: number;
  text: string;
};

// Background music: loops under the whole video. Mixed in by the Rust core after rendering.
export type Music = {
  source: string;
  volume: number;
  fade_in_ms: number;
  fade_out_ms: number;
};

export type VideoTimeline = {
  fps: number;
  resolution: Resolution;
  scenes: Scene[];
  subtitles: SubtitleCue[];
  music?: Music | null;
};
