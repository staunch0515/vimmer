// TypeScript mirror of the Rust `vimmer-dsl` model (rust-core/crates/vimmer-dsl/src/model.rs).
// Keep field names in sync: the JSON document is the contract between the two layers.

export type MotionType = "static" | "zoom_in" | "zoom_out" | "pan_left" | "pan_right";

export type AudioMapping = {
  source: string;
  offset_ms: number;
};

export type Scene = {
  id: string;
  duration_ms: number;
  text: string | null;
  highlights: string[];
  image: string | null;
  motion: MotionType;
  audio: AudioMapping | null;
};

export type Resolution = {
  width: number;
  height: number;
};

export type VideoTimeline = {
  fps: number;
  resolution: Resolution;
  scenes: Scene[];
};
