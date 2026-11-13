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
  // Extra graphics, in list order. Absent or empty: the scene renders exactly as before.
  layers?: Layer[];
};

// ---- Layers. Mirrors rust-core/crates/vimmer-dsl/src/layer.rs (units and stacking are documented
// there and in motion/layers.ts). Optional fields fall back to the Rust defaults.

export type Anchor = "top_left" | "center";

export type LayerAnimationKind = "fade" | "slide_up" | "slide_down" | "slide_left" | "slide_right" | "pop";

export type LayerAnimation = {
  kind: LayerAnimationKind;
  duration_ms?: number; // default 400
};

export type LayerCommon = {
  id: string;
  x: number; // fraction of the frame width, from the left
  y: number; // fraction of the frame height, from the top
  width: number; // fraction of the frame width
  height?: number | null; // fraction of the frame height; text may omit it
  anchor?: Anchor; // default top_left
  z?: number; // default 0; < 0 under the scene text, >= 0 above it
  start_ms?: number; // from the scene start, default 0
  end_ms?: number | null; // from the scene start, default the scene end
  opacity?: number; // default 1
  rotation?: number; // degrees, default 0
  keyframes?: Keyframe[]; // at_ms from the scene start
  enter?: LayerAnimation | null;
  exit?: LayerAnimation | null;
};

export type ImageLayer = LayerCommon & {
  kind: "image";
  src: string;
  fit?: "cover" | "contain"; // default cover
  corner_radius?: number; // fraction of the frame height
};

export type TextBackground = {
  color: string;
  padding?: number; // font sizes, default 0.4
  radius?: number; // font sizes, default 0.3
};

export type TextLayer = LayerCommon & {
  kind: "text";
  text: string;
  font_size?: number; // fraction of the frame height, default 0.06
  color?: string; // default #fafafa
  weight?: number; // default 600
  align?: "left" | "center" | "right"; // default center
  line_height?: number; // default 1.2
  background?: TextBackground | null;
  highlights?: string[];
  reveal?: "none" | "words"; // default none
};

export type ShapeKind = "rect" | "ellipse" | "line";

export type ShapeLayer = LayerCommon & {
  kind: "shape";
  shape?: ShapeKind; // default rect
  fill?: string | null;
  stroke?: string | null;
  stroke_width?: number; // fraction of the frame height; 0 with a stroke = 0.004
  corner_radius?: number; // rect only, fraction of the frame height
};

export type IconLayer = LayerCommon & {
  kind: "icon";
  src: string; // .svg under public/
  color?: string; // default #fafafa
};

export type Layer = ImageLayer | TextLayer | ShapeLayer | IconLayer;
export type LayerKind = Layer["kind"];

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

// Ducking: the music drops under each scene's audio (narration or `audio`) and comes back up
// between them. Omitted = on with the defaults below. Applied by the Rust core when it mixes the
// soundtrack; the preview plays the music without it.
export type Duck = {
  enabled?: boolean; // default true
  depth_db?: number; // -40..0, default -12 (relative to `volume`)
  attack_ms?: number; // 10..5000, default 200: fully down when the speech starts
  release_ms?: number; // 10..5000, default 400: back up after the speech ends
};

// Background music: loops under the whole video. Mixed in by the Rust core after rendering.
export type Music = {
  source: string;
  volume: number;
  fade_in_ms: number;
  fade_out_ms: number;
  duck?: Duck;
};

export type VideoTimeline = {
  fps: number;
  resolution: Resolution;
  scenes: Scene[];
  subtitles: SubtitleCue[];
  music?: Music | null;
};
