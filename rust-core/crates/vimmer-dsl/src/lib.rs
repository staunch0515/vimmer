//! JSON DSL for describing a short video timeline.
//!
//! The same JSON document is consumed by the Remotion renderer, so field names
//! here are part of a cross-language contract (`snake_case`, no renames).

pub mod beatsync;
pub mod brand;
pub mod layer;
pub mod library;
pub mod look;
pub mod model;
pub mod srt;
pub mod template;
pub mod text_layout;
pub mod text_style;
pub mod validation;
pub mod voiceover;

pub use beatsync::{
    beat_grid, manual_beats, scene_floor, snap_cuts, CutSnap, SnapReport, MIN_SCENE_MS,
};
pub use brand::{Brand, BrandColors, BrandFont, BrandFonts, Corner, FontRole, Logo};
pub use layer::{
    clip_frame_at, clip_frames, clip_problems, count_frames, format_number, frame_progress,
    is_hex_color, typing_units, Anchor, BarChartLayer, ChartBar, ChartOrientation, CounterLayer,
    FitTime, IconLayer, ImageFit, ImageLayer, Layer, LayerAnimation, LayerAnimationKind,
    LayerContent, LowerThirdLayer, ProgressRingLayer, ShapeKind, ShapeLayer, TextAlign,
    TextBackground, TextBorder, TextLayer, TextReveal, VideoClip, CLIP_EXTENSIONS, DEFAULT_TYPING_CPS,
    MAX_BARS, MAX_BAR_LABEL_CHARS, MAX_CLIP_SPEED, MAX_LAYERS, MAX_LAYER_KEYFRAMES,
    MAX_LAYER_TEXT_CHARS, MAX_RING_LABEL_CHARS, MAX_TYPING_CPS, MAX_VIDEO_LAYERS, MIN_CLIP_SPEED,
};
pub use library::{
    list_json, pick_json, pick_seed, resolve_music_pick, track_json, wanted_pick, LibraryError, LibraryTrack, MusicLibrary, PickChoice, TrackCredit,
    FAR_ENERGY, LIBRARY_DIR, LIBRARY_INDEX,
};
pub use look::{
    auto_preset, preset_for_mood, replace_auto_grades, resolve_auto_grades, EffectiveLook, Grade, Look, Overlay, GRADE_PRESETS,
    MAX_OVERLAYS, OVERLAY_KINDS, TINTED_OVERLAYS,
};
pub use model::{
    AudioMapping, BeatGrid, BeatSync, Duck, Easing, Keyframe, LayerEvent, MotionType, Music,
    MusicPick, Narration, Resolution, Scene, SfxTrigger, SnapMode, SoundEffect, SubtitleBox,
    SubtitleCue, Transition, TransitionKind, VideoTimeline, WordTiming, MAX_DURATION_MS,
    MAX_FRAMES, TARGET_FPS,
};
pub use srt::{parse_srt, SrtError};
pub use template::{
    expand_scene, expand_templates, SceneTemplate, TemplateError, TemplateInfo, CATALOG,
};
pub use text_layout::{TextPlace, TextPoint, 
    Paging, ResolvedTextLayout, TextLayout, TextPage, TextPosition, DEFAULT_TEXT_PX, MAX_TEXT_PX,
    MIN_TEXT_PX,
};
pub use text_style::{
    style_scale, CustomShadow, ShadowPreset, TextOutline, TextShadow, TextStyle, MAX_OUTLINE_PX,
};
pub use voiceover::{overlapping_clips, VoiceoverClip, MAX_VOICEOVER_CLIPS, MAX_VOICEOVER_GAIN};
pub use validation::{sfx_problems, validate, voiceover_problems, ValidationError, MAX_SFX_PER_SCENE, SFX_EXTENSIONS};
