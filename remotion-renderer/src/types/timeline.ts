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

// Easing of the segment that leaves a keyframe. Mirrors `Easing` in model.rs.
export type Easing = "linear" | "ease_in" | "ease_out" | "ease_in_out";

// One pose of the scene's image layer. Mirrors `Keyframe` in model.rs.
// Optional fields fall back to the identity pose when absent from the JSON.
export type Keyframe = {
  at_ms: number;
  scale?: number;
  x?: number;
  y?: number;
  opacity?: number;
  rotation?: number;
  ease?: Easing;
};

export type TransitionKind =
  | "fade"
  | "slide_left"
  | "slide_right"
  | "slide_up"
  | "slide_down"
  | "zoom_in"
  | "wipe_left";

// Mirrors `Transition` in model.rs. The overlap rule is documented in motion/transitions.ts.
export type Transition = {
  kind: TransitionKind;
  duration_ms: number;
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
  transition_in?: Transition | null;
  // When non-empty, replaces `motion`.
  keyframes?: Keyframe[];
};

export type Resolution = {
  width: number;
  height: number;
};

// Mirrors `WordTiming` in model.rs: when one word (one character for Chinese and Japanese)
// is spoken, in absolute milliseconds like the cue. `text` keeps attached punctuation and spacing.
export type WordTiming = {
  text: string;
  start_ms: number;
  end_ms: number;
};

export type SubtitleCue = {
  start_ms: number;
  end_ms: number;
  text: string;
  // Absent or empty: the cue is shown as one line, as before. Present: shown word by word,
  // with the word being spoken highlighted.
  words?: WordTiming[];
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
