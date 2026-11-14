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
// back_out overshoots to about 1.1, elastic_out springs to about 1.37 and rings out, bounce_out
// bounces up to 1 (never above).
export type Easing = "linear" | "ease_in" | "ease_out" | "ease_in_out" | "back_out" | "elastic_out" | "bounce_out";

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

// ---- Video clips. Mirrors `VideoClip` in rust-core/crates/vimmer-dsl/src/layer.rs (frame math in
// motion/video.ts). Optional fields fall back to the Rust defaults.

// Shorter than its scene or layer: hold the last frame, start again, or change speed to fit.
export type FitTime = "freeze" | "loop" | "stretch";

export type VideoClip = {
  src: string; // .mp4 / .mov / .m4v / .webm under public/
  trim_start_ms?: number; // default 0
  trim_end_ms?: number | null; // default the end of the file
  speed?: number; // 0.25..4, default 1; ignored with fit_time stretch
  fit_time?: FitTime; // default freeze
  fit?: "cover" | "contain"; // default cover
  volume?: number; // 0..1, default 0 (muted); mixed into the soundtrack by the Rust core
};

export type Scene = {
  id: string;
  duration_ms: number;
  text: string | null;
  highlights: string[];
  image: string | null;
  motion: MotionType;
  // A clip as the background, in the image's place (motion / keyframes move it). Not with `image`.
  video?: VideoClip | null;
  audio: AudioMapping | null;
  narration?: Narration | null;
  transition_in?: Transition | null;
  // When non-empty, replaces `motion`.
  keyframes?: Keyframe[];
  // Extra graphics, in list order. Absent or empty: the scene renders exactly as before.
  layers?: Layer[];
  // A built-in template, expanded into layers by the Rust core (vimmer-dsl/src/template.rs)
  // before rendering. The renderer never sees one; the editor previews it through `vimmer expand`.
  template?: SceneTemplate | null;
  // Short sounds mixed in at moments of this scene (at most 16). Absent or empty: none.
  sfx?: SoundEffect[];
};

// Mirrors `SoundEffect` in vimmer-dsl/src/model.rs. Exactly one of `at_ms` / `on`. Mixed into the
// soundtrack by the Rust core on the frame of its moment; plays to its end (past the scene end).
export type LayerEvent = "enter" | "exit";
export type SoundEffect = {
  src: string; // audio file under public/, e.g. assets/sfx/whoosh.wav
  at_ms?: number | null; // from the scene start
  on?: { layer: string; event: LayerEvent } | null; // when a layer enters, or starts its exit
  volume?: number; // 0..1, default 0.8
  duck_music?: boolean; // default false: plays over the music without lowering it
};

export type SceneTemplate = {
  name: string; // intro | outro | title_card | quote | list (editor/src/template-catalog.json)
  params?: Record<string, string | number | boolean | string[]>;
};

// ---- Layers. Mirrors rust-core/crates/vimmer-dsl/src/layer.rs (units and stacking are documented
// there and in motion/layers.ts). Optional fields fall back to the Rust defaults.

export type Anchor = "top_left" | "center";

export type LayerAnimationKind = "fade" | "slide_up" | "slide_down" | "slide_left" | "slide_right" | "pop";

export type LayerAnimation = {
  kind: LayerAnimationKind;
  duration_ms?: number; // default 400
  ease?: Easing; // default: the kind's own curve (ease-out cubic in, ease-in cubic out, back-out for pop in)
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
  color?: string; // default: brand text, else #fafafa
  font?: FontRole; // brand font, default body
  weight?: number; // default 600
  align?: "left" | "center" | "right"; // default center
  line_height?: number; // default 1.2
  background?: TextBackground | null;
  highlights?: string[];
  // typewriter: unit by unit (motion/typewriter.ts) at typing_cps from the layer start
  reveal?: "none" | "words" | "typewriter"; // default none
  typing_cps?: number; // characters per second, 1..200, default 20
  cursor?: boolean; // blinking cursor, typewriter only, default false
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

// Name/title strip. Without enter/exit it animates itself (motion/lowerThird.ts).
export type LowerThirdLayer = LayerCommon & {
  kind: "lower_third";
  title: string; // at most 120 characters
  subtitle?: string | null; // at most 160 characters
  font_size?: number; // title, fraction of the frame height, default 0.045; subtitle 0.62 of it
  color?: string; // title, default: brand text, else #fafafa
  subtitle_color?: string; // default: brand muted_text, else #d4d4d8
  accent_color?: string; // default: brand primary, else #6366f1
  background_color?: string | null; // default #09090bd9; null = no box
};

// A number counting from `from` to `to` (motion/data.ts). One line; height may be left out.
export type CounterLayer = LayerCommon & {
  kind: "counter";
  from?: number; // default 0
  to: number;
  duration_ms?: number | null; // default: until the exit begins
  ease?: Easing; // default ease_out
  decimals?: number; // 0..6, default 0
  prefix?: string;
  suffix?: string;
  separator?: string; // thousands, default ","; "" for none
  decimal_mark?: string; // default "."
  font_size?: number; // fraction of the frame height, default 0.12
  color?: string; // default: brand text, else #fafafa
  weight?: number; // default 700
  align?: "left" | "center" | "right"; // default center
};

export type ChartBar = { label: string; value: number; color?: string | null };

// Bars growing in one after another (motion/data.ts for timing and layout fractions).
export type BarChartLayer = LayerCommon & {
  kind: "bar_chart";
  bars: ChartBar[]; // 1..12, values >= 0
  orientation?: "vertical" | "horizontal"; // default vertical
  max?: number | null; // default the largest value
  color?: string; // bars, default: brand primary, else #6366f1
  label_color?: string; // default: brand muted_text, else #e4e4e7
  font_size?: number; // labels and values, fraction of the frame height, default 0.032
  show_values?: boolean; // default true
  decimals?: number; // value labels, 0..6
  prefix?: string;
  suffix?: string;
  grow_ms?: number; // default 800
  stagger_ms?: number; // default 120
  ease?: Easing; // default ease_out
};

// A ring filling clockwise from the top, from `from` to `value` (fractions of a full ring).
export type ProgressRingLayer = LayerCommon & {
  kind: "progress_ring";
  value: number; // 0..1
  from?: number; // 0..1, default 0
  duration_ms?: number | null; // default: until the exit begins
  ease?: Easing; // default ease_out
  color?: string; // default: brand primary, else #6366f1
  track_color?: string; // default #ffffff26
  thickness?: number; // fraction of the ring's diameter, 0.02..0.5, default 0.12
  show_label?: boolean; // default true
  label?: string | null; // custom center text (at most 24 characters); default the percentage
  decimals?: number; // of the percentage, default 0
  label_color?: string; // default: brand text, else #fafafa
};

// A video clip in a box (at most 4 per scene). Drawn muted; its sound is mixed by the Rust core.
export type VideoLayer = LayerCommon & VideoClip & { kind: "video" };

export type Layer =
  | VideoLayer
  | ImageLayer
  | TextLayer
  | ShapeLayer
  | IconLayer
  | LowerThirdLayer
  | CounterLayer
  | BarChartLayer
  | ProgressRingLayer;
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
// Exactly one of `source` (a file) and `pick` (a library track chosen by mood; the Rust core
// turns it into `source` before rendering, see public/assets/music/library) is given.
export type Music = {
  source?: string;
  pick?: MusicPick | null;
  volume: number;
  fade_in_ms: number;
  fade_out_ms: number;
  duck?: Duck;
  // A beat grid given by hand (skips detection): a beat every 60000/bpm ms from offset_ms.
  beats?: BeatGrid | null;
  // Snap scene cuts to the music's beats at render time (after narration). The preview shows
  // the cuts where the durations put them.
  beat_sync?: BeatSync | null;
};

// Choose a track from the music library: one whose moods include `mood`, the closest to
// `energy` (0 calm .. 1 intense) when given, never one of `exclude` (track ids).
export type MusicPick = {
  mood: string;
  energy?: number;
  exclude?: string[];
};

export type BeatGrid = {
  bpm: number; // 30..300
  offset_ms?: number; // first beat, from the start of the file; default 0
};

export type BeatSync = {
  enabled?: boolean; // default true
  every?: 1 | 2 | 4; // beats per step (4 = bar starts); default 1
  max_shift_ms?: number; // 1..1000, default 250
  mode?: "nearest" | "later"; // default nearest
};

// ---- Brand kit. Mirrors rust-core/crates/vimmer-dsl/src/brand.rs (which element uses which color
// is documented there; the built-in values are in src/brand.ts). Every field is optional.

export type FontRole = "heading" | "body";

export type BrandColors = {
  primary?: string; // lower-third bars, chart bars, rings, template shapes
  accent?: string; // highlighted words
  text?: string;
  muted_text?: string; // lower-third subtitles, chart labels, template sub-lines
  background?: string; // behind scenes without an image or video
  subtitle_background?: string;
};

export type BrandFont = {
  family: string;
  src?: string | null; // .ttf/.otf/.woff/.woff2 under public/, loaded before the first frame
};

export type LogoCorner = "top_left" | "top_right" | "bottom_left" | "bottom_right";

export type Logo = {
  src: string; // .png/.jpg/.jpeg under public/
  corner?: LogoCorner; // default top_right
  size?: number; // height, fraction of the frame height, default 0.08
  margin?: number; // fraction of the frame width (x) and height (y), default 0.05
  opacity?: number; // default 1
  hide_on?: string[]; // scene ids
};

export type Brand = {
  colors?: BrandColors;
  fonts?: { heading?: BrandFont | null; body?: BrandFont | null };
  logo?: Logo | null;
};

export type VideoTimeline = {
  fps: number;
  resolution: Resolution;
  scenes: Scene[];
  subtitles: SubtitleCue[];
  music?: Music | null;
  // Absent: the built-in look, exactly as before brands existed.
  brand?: Brand | null;
};
