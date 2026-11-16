//! The look of a scene (画面氛围): a color grade over the scene picture and animated atmosphere
//! overlays (snow, dust, bokeh, film grain) drawn above it.
//!
//! Where it is set:
//! - `timeline.look`: the default of every scene.
//! - `scene.look`: this scene's own. Each part replaces the default's part when present: a
//!   scene `grade` replaces the default grade; a scene `overlays` list (even an empty one, which
//!   means "no overlays here") replaces the default list. A part left out is inherited.
//!
//! Stacking (renderer: `remotion-renderer/src/components/Atmosphere.tsx`): the grade applies to the
//! scene picture (background image or video, layers, scene text); the overlays sit above it; the
//! brand logo and the subtitles stay on top, ungraded and unobscured.
//!
//! `preset: "auto"` takes the preset from the music's mood (`music.pick.mood`, see
//! [`preset_for_mood`]); without a mood it is `none`. Because a render replaces the pick with the
//! chosen file before the renderer sees it, [`crate::resolve_music_pick`] first turns every
//! `auto` into the concrete preset (see [`resolve_auto_grades`]).
//!
//! Every field is optional and left out of the JSON when absent, so timelines without a look
//! round-trip byte-identical and render exactly as before.

use serde::{Deserialize, Serialize};

use crate::layer::is_hex_color;
use crate::model::VideoTimeline;

/// Grade presets. `none` switches grading off (e.g. one scene under a graded default), `auto`
/// follows the music mood.
pub const GRADE_PRESETS: [&str; 8] = [
    "none",
    "auto",
    "warm",
    "tech_blue",
    "noir",
    "moody",
    "vintage",
    "vivid",
];

/// Overlay kinds.
pub const OVERLAY_KINDS: [&str; 4] = ["snow", "dust", "bokeh", "film_grain"];

/// Overlay kinds that take a `color` (the tint of the motes or light spots).
pub const TINTED_OVERLAYS: [&str; 2] = ["dust", "bokeh"];

/// Most overlays one look may stack.
pub const MAX_OVERLAYS: usize = 4;

/// Default overlay intensity: noticeable but subtle.
pub const DEFAULT_INTENSITY: f32 = 0.5;

fn default_intensity() -> f32 {
    DEFAULT_INTENSITY
}

fn default_strength() -> f32 {
    1.0
}

/// A color grade of the scene picture: a preset at a strength (0 = ungraded, 1 = the full preset).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Grade {
    /// One of [`GRADE_PRESETS`].
    pub preset: String,
    #[serde(default = "default_strength")]
    pub strength: f32,
}

/// One animated overlay above the scene picture.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Overlay {
    /// One of [`OVERLAY_KINDS`].
    pub kind: String,
    /// 0..1: how much of it (particle count, size and opacity; grain strength).
    #[serde(default = "default_intensity")]
    pub intensity: f32,
    /// Tint of `dust` and `bokeh` (`#rgb`, `#rrggbb` or `#rrggbbaa`). Absent: a warm white / gold.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
}

/// A look: grade and overlays, each optional (see the module docs for inheritance).
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct Look {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub grade: Option<Grade>,
    /// Present (even empty): these overlays, replacing the default's. Absent: inherited.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub overlays: Option<Vec<Overlay>>,
}

/// The look a scene is drawn with, after inheritance.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct EffectiveLook {
    pub grade: Option<Grade>,
    pub overlays: Vec<Overlay>,
    /// Whether the overlays are the timeline default's (they then continue across scene cuts).
    pub overlays_inherited: bool,
}

impl Look {
    /// The look of a scene with `own` look under the timeline default `default`.
    pub fn effective(default: Option<&Look>, own: Option<&Look>) -> EffectiveLook {
        let grade = own
            .and_then(|l| l.grade.clone())
            .or_else(|| default.and_then(|l| l.grade.clone()));
        let (overlays, overlays_inherited) = match own.and_then(|l| l.overlays.clone()) {
            Some(list) => (list, false),
            None => (
                default.and_then(|l| l.overlays.clone()).unwrap_or_default(),
                true,
            ),
        };
        EffectiveLook {
            grade,
            overlays,
            overlays_inherited,
        }
    }

    /// Every problem of this look, as messages (unknown preset or kind, out-of-range numbers,
    /// colors, too many or repeated overlays).
    pub fn problems(&self) -> Vec<String> {
        let mut out = Vec::new();
        if let Some(g) = &self.grade {
            if !GRADE_PRESETS.contains(&g.preset.as_str()) {
                out.push(format!(
                    "grade preset `{}` is unknown; presets: {}",
                    g.preset,
                    GRADE_PRESETS.join(", ")
                ));
            }
            if !(g.strength.is_finite() && (0.0..=1.0).contains(&g.strength)) {
                out.push(format!(
                    "grade strength {} must be between 0 and 1",
                    g.strength
                ));
            }
        }
        if let Some(list) = &self.overlays {
            if list.len() > MAX_OVERLAYS {
                out.push(format!(
                    "{} overlays; the limit is {MAX_OVERLAYS}",
                    list.len()
                ));
            }
            for (i, o) in list.iter().enumerate() {
                if !OVERLAY_KINDS.contains(&o.kind.as_str()) {
                    out.push(format!(
                        "overlay {i} kind `{}` is unknown; kinds: {}",
                        o.kind,
                        OVERLAY_KINDS.join(", ")
                    ));
                } else if list[..i].iter().any(|p| p.kind == o.kind) {
                    out.push(format!(
                        "overlay {i}: `{}` is listed twice; raise its intensity instead",
                        o.kind
                    ));
                }
                if !(o.intensity.is_finite() && (0.0..=1.0).contains(&o.intensity)) {
                    out.push(format!(
                        "overlay {i} intensity {} must be between 0 and 1",
                        o.intensity
                    ));
                }
                if let Some(c) = &o.color {
                    if !is_hex_color(c) {
                        out.push(format!(
                            "overlay {i} color `{c}` must be a color like #rgb, #rrggbb or #rrggbbaa"
                        ));
                    } else if OVERLAY_KINDS.contains(&o.kind.as_str())
                        && !TINTED_OVERLAYS.contains(&o.kind.as_str())
                    {
                        out.push(format!(
                            "overlay {i}: `{}` has no color (only {} take one)",
                            o.kind,
                            TINTED_OVERLAYS.join(" and ")
                        ));
                    }
                }
            }
        }
        out
    }
}

/// The preset `auto` picks for a music mood (case and spaces ignored). Covers every mood of the
/// built-in music library; any other mood gets `none`.
///
/// | moods | preset |
/// |---|---|
/// | warm, hopeful, romantic, nostalgic | warm |
/// | calm, reflective, ambient | vintage |
/// | tense, suspense, dark, mysterious, sad | moody |
/// | happy, playful, upbeat, energetic, fun | vivid |
/// | epic, dramatic, inspiring, tech, corporate, futuristic | tech_blue |
pub fn preset_for_mood(mood: &str) -> &'static str {
    match mood.trim().to_lowercase().as_str() {
        "warm" | "hopeful" | "romantic" | "nostalgic" => "warm",
        "calm" | "reflective" | "ambient" => "vintage",
        "tense" | "suspense" | "dark" | "mysterious" | "sad" => "moody",
        "happy" | "playful" | "upbeat" | "energetic" | "fun" => "vivid",
        "epic" | "dramatic" | "inspiring" | "tech" | "corporate" | "futuristic" => "tech_blue",
        _ => "none",
    }
}

/// Replaces every `auto` grade (timeline default and scenes) with the preset of the music's mood
/// (`none` without one). Called before a music pick is replaced by its file, which drops the mood.
/// Timelines without an `auto` grade are left untouched.
pub fn resolve_auto_grades(t: &mut VideoTimeline) {
    let preset = auto_preset(t);
    replace_auto_grades(t, preset);
}

/// The preset `auto` stands for in `t`: the one of its music pick's mood, else `none`.
pub fn auto_preset(t: &VideoTimeline) -> &'static str {
    preset_for_mood(
        t.music
            .as_ref()
            .and_then(|m| m.pick.as_ref())
            .map_or("", |p| p.mood.as_str()),
    )
}

/// Replaces every `auto` grade of `t` (timeline default and scenes) with `preset`. Used for a
/// render chunk, which carries no music: `preset` is then [`auto_preset`] of the full timeline.
pub fn replace_auto_grades(t: &mut VideoTimeline, preset: &str) {
    let looks = t
        .look
        .iter_mut()
        .chain(t.scenes.iter_mut().filter_map(|s| s.look.as_mut()));
    for look in looks {
        if let Some(g) = look.grade.as_mut().filter(|g| g.preset == "auto") {
            g.preset = preset.to_string();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn look(json: &str) -> Look {
        serde_json::from_str(json).unwrap()
    }

    #[test]
    fn a_look_round_trips_and_fills_defaults() {
        let l = look(r#"{"grade":{"preset":"warm"},"overlays":[{"kind":"snow"}]}"#);
        assert_eq!(l.grade.as_ref().unwrap().strength, 1.0);
        assert_eq!(l.overlays.as_ref().unwrap()[0].intensity, 0.5);
        assert_eq!(
            serde_json::to_string(&l).unwrap(),
            r#"{"grade":{"preset":"warm","strength":1.0},"overlays":[{"kind":"snow","intensity":0.5}]}"#
        );
        // Absent parts are not written; an explicit empty list is.
        assert_eq!(serde_json::to_string(&look("{}")).unwrap(), "{}");
        assert_eq!(
            serde_json::to_string(&look(r#"{"overlays":[]}"#)).unwrap(),
            r#"{"overlays":[]}"#
        );
        let tinted = r##"{"kind":"bokeh","intensity":0.3,"color":"#ffcc88"}"##;
        let o: Overlay = serde_json::from_str(tinted).unwrap();
        assert_eq!(serde_json::to_string(&o).unwrap(), tinted);
    }

    #[test]
    fn a_scene_part_replaces_the_default_part_and_a_missing_part_is_inherited() {
        let default = look(
            r#"{"grade":{"preset":"warm","strength":0.6},"overlays":[{"kind":"dust"},{"kind":"film_grain"}]}"#,
        );
        // No scene look: everything inherited.
        let e = Look::effective(Some(&default), None);
        assert_eq!(e.grade.as_ref().unwrap().preset, "warm");
        assert_eq!(e.overlays.len(), 2);
        assert!(e.overlays_inherited);
        // Scene grade only: overlays still inherited.
        let e = Look::effective(
            Some(&default),
            Some(&look(r#"{"grade":{"preset":"noir"}}"#)),
        );
        assert_eq!(e.grade.as_ref().unwrap().preset, "noir");
        assert_eq!(e.overlays.len(), 2);
        // Scene overlays replace the whole list; the grade is inherited.
        let e = Look::effective(
            Some(&default),
            Some(&look(r#"{"overlays":[{"kind":"snow"}]}"#)),
        );
        assert_eq!(e.grade.as_ref().unwrap().preset, "warm");
        assert_eq!(e.overlays.len(), 1);
        assert_eq!(e.overlays[0].kind, "snow");
        assert!(!e.overlays_inherited);
        // An explicit empty list: no overlays in this scene.
        let e = Look::effective(Some(&default), Some(&look(r#"{"overlays":[]}"#)));
        assert!(e.overlays.is_empty());
        // `none` is a real grade: it switches the default off for this scene.
        let e = Look::effective(
            Some(&default),
            Some(&look(r#"{"grade":{"preset":"none"}}"#)),
        );
        assert_eq!(e.grade.as_ref().unwrap().preset, "none");
        // Nothing anywhere.
        assert_eq!(Look::effective(None, None), EffectiveLook {
            grade: None,
            overlays: vec![],
            overlays_inherited: true
        });
    }

    #[test]
    fn bad_values_are_reported() {
        assert!(look(
            r##"{"grade":{"preset":"auto","strength":0},"overlays":[{"kind":"snow","intensity":1},{"kind":"dust","color":"#fff"}]}"##
        )
        .problems()
        .is_empty());
        let p = look(r#"{"grade":{"preset":"sepia","strength":1.5}}"#).problems();
        assert_eq!(p.len(), 2, "{p:?}");
        assert!(p[0].contains("`sepia` is unknown"));
        assert!(p[1].contains("strength 1.5"));
        let p = look(
            r##"{"overlays":[{"kind":"rain"},{"kind":"snow","intensity":-0.1},{"kind":"snow"},{"kind":"film_grain","color":"#fff"},{"kind":"dust","color":"red"}]}"##,
        )
        .problems();
        let all = p.join("\n");
        assert!(all.contains("5 overlays; the limit is 4"), "{all}");
        assert!(all.contains("kind `rain` is unknown"), "{all}");
        assert!(all.contains("intensity -0.1"), "{all}");
        assert!(all.contains("`snow` is listed twice"), "{all}");
        assert!(all.contains("`film_grain` has no color"), "{all}");
        assert!(all.contains("color `red`"), "{all}");
    }

    #[test]
    fn every_library_mood_maps_to_a_preset() {
        let raw = std::fs::read_to_string(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../../remotion-renderer/public/assets/music/library/library.json"
        ))
        .unwrap();
        let lib: serde_json::Value = serde_json::from_str(&raw).unwrap();
        for track in lib["tracks"].as_array().unwrap() {
            for mood in track["moods"].as_array().unwrap() {
                let mood = mood.as_str().unwrap();
                let preset = preset_for_mood(mood);
                assert_ne!(preset, "none", "library mood `{mood}` has no preset");
                assert!(GRADE_PRESETS.contains(&preset));
            }
        }
        assert_eq!(preset_for_mood(" Warm "), "warm");
        assert_eq!(preset_for_mood("calm"), "vintage");
        assert_eq!(preset_for_mood("tense"), "moody");
        assert_eq!(preset_for_mood("upbeat"), "vivid");
        assert_eq!(preset_for_mood("epic"), "tech_blue");
        assert_eq!(preset_for_mood("polka"), "none");
        assert_eq!(preset_for_mood(""), "none");
    }

    #[test]
    fn auto_grades_resolve_from_the_pick_mood_and_nothing_else_changes() {
        let mut t: VideoTimeline = serde_json::from_value(serde_json::json!({
            "fps": 30, "resolution": {"width": 1920, "height": 1080},
            "scenes": [
                {"id": "a", "duration_ms": 1000, "text": "a"},
                {"id": "b", "duration_ms": 1000, "text": "b", "look": {"grade": {"preset": "auto", "strength": 0.5}}},
                {"id": "c", "duration_ms": 1000, "text": "c", "look": {"grade": {"preset": "noir"}}}
            ],
            "music": {"pick": {"mood": "tense"}, "volume": 0.3, "fade_in_ms": 0, "fade_out_ms": 0},
            "look": {"grade": {"preset": "auto"}}
        }))
        .unwrap();
        resolve_auto_grades(&mut t);
        assert_eq!(t.look.as_ref().unwrap().grade.as_ref().unwrap().preset, "moody");
        let b = t.scenes[1].look.as_ref().unwrap().grade.as_ref().unwrap();
        assert_eq!((b.preset.as_str(), b.strength), ("moody", 0.5));
        assert_eq!(t.scenes[2].look.as_ref().unwrap().grade.as_ref().unwrap().preset, "noir");
        assert!(t.scenes[0].look.is_none());

        // No mood (a music file, or no music): none.
        t.scenes[1].look.as_mut().unwrap().grade.as_mut().unwrap().preset = "auto".into();
        t.music.as_mut().unwrap().pick = None;
        t.music.as_mut().unwrap().source = Some("m.mp3".into());
        resolve_auto_grades(&mut t);
        assert_eq!(t.scenes[1].look.as_ref().unwrap().grade.as_ref().unwrap().preset, "none");
    }
}
