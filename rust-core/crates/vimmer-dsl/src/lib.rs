//! JSON DSL for describing a short video timeline.
//!
//! The same JSON document is consumed by the Remotion renderer, so field names
//! here are part of a cross-language contract (`snake_case`, no renames).

pub mod beatsync;
pub mod brand;
pub mod layer;
pub mod library;
pub mod model;
pub mod srt;
pub mod template;
pub mod validation;

pub use beatsync::{
    beat_grid, manual_beats, scene_floor, snap_cuts, CutSnap, SnapReport, MIN_SCENE_MS,
};
pub use brand::{Brand, BrandColors, BrandFont, BrandFonts, Corner, FontRole, Logo};
pub use layer::{
    clip_frame_at, clip_frames, clip_problems, count_frames, format_number, frame_progress,
    is_hex_color, typing_units, Anchor, BarChartLayer, ChartBar, ChartOrientation, CounterLayer,
    FitTime, IconLayer, ImageFit, ImageLayer, Layer, LayerAnimation, LayerAnimationKind,
    LayerContent, LowerThirdLayer, ProgressRingLayer, ShapeKind, ShapeLayer, TextAlign,
    TextBackground, TextLayer, TextReveal, VideoClip, CLIP_EXTENSIONS, DEFAULT_TYPING_CPS,
    MAX_BARS, MAX_BAR_LABEL_CHARS, MAX_CLIP_SPEED, MAX_LAYERS, MAX_LAYER_KEYFRAMES,
    MAX_LAYER_TEXT_CHARS, MAX_RING_LABEL_CHARS, MAX_TYPING_CPS, MAX_VIDEO_LAYERS, MIN_CLIP_SPEED,
};
pub use library::{
    pick_seed, resolve_music_pick, LibraryError, LibraryTrack, MusicLibrary, PickChoice,
    FAR_ENERGY, LIBRARY_DIR, LIBRARY_INDEX,
};
pub use model::{
    AudioMapping, BeatGrid, BeatSync, Duck, Easing, Keyframe, LayerEvent, MotionType, Music,
    MusicPick, Narration, Resolution, Scene, SfxTrigger, SnapMode, SoundEffect, SubtitleCue,
    Transition, TransitionKind, VideoTimeline, WordTiming, MAX_DURATION_MS, MAX_FRAMES, TARGET_FPS,
};
pub use srt::{parse_srt, SrtError};
pub use template::{
    expand_scene, expand_templates, SceneTemplate, TemplateError, TemplateInfo, CATALOG,
};
pub use validation::{sfx_problems, validate, ValidationError, MAX_SFX_PER_SCENE, SFX_EXTENSIONS};
