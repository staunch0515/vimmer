//! The narrator avatar (数字人讲解人): a talking head made from one face photo, shown as an inset
//! in a corner while someone speaks.
//!
//! - `brand.avatar`: the brand's default narrator.
//! - `avatar` (timeline): this video's narrator; replaces the brand's as a whole.
//! - `scenes[i].avatar`: per-scene override: `enabled` on/off, another `photo`, another `corner`.
//!
//! The clip itself is generated from the video's speech by `vimmer avatar generate` (slow, cached
//! under `generated/avatar/`). The render never generates it: scenes whose clip is not ready are
//! drawn without the inset (or the render stops, with `--avatar require`). Scenes with no speech
//! never show it. The scheduler writes the clip to the render-only [`AvatarClip`] of each scene.
//!
//! Every field at its default is left out of the JSON, and timelines without an avatar are
//! written exactly as before.

use serde::{Deserialize, Serialize};

use crate::brand::Corner;

pub const DEFAULT_AVATAR_SIZE: f32 = 0.3;
pub const DEFAULT_AVATAR_MARGIN: f32 = 0.04;
pub const DEFAULT_BORDER_WIDTH: f32 = 0.025;
pub const DEFAULT_BORDER_COLOR: &str = "#FFFFFF";
pub const MIN_AVATAR_SIZE: f32 = 0.1;
pub const MAX_AVATAR_SIZE: f32 = 0.6;
pub const MAX_AVATAR_MARGIN: f32 = 0.3;
pub const MAX_BORDER_WIDTH: f32 = 0.1;
/// Photo files the avatar engine reads.
pub const AVATAR_PHOTO_EXTENSIONS: &[&str] = &["png", "jpg", "jpeg", "webp"];

/// Shape of the inset.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AvatarShape {
    #[default]
    Circle,
    /// A square with rounded corners.
    Rounded,
}

/// A ring around the inset.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AvatarBorder {
    /// `#RRGGBB`. Absent: white.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
    /// Ring width as a fraction of the inset size. Absent: 0.025.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub width: Option<f32>,
}

/// The narrator of a brand or a timeline.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Avatar {
    /// A face photo (.png/.jpg/.jpeg/.webp) relative to the public dir or the project folder.
    /// One clear, front-facing face; head and shoulders work best.
    pub photo: String,
    /// Absent: bottom right.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub corner: Option<Corner>,
    /// Inset height as a fraction of the frame height (0.1–0.6). Absent: 0.3.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size: Option<f32>,
    /// Distance from the two nearest edges, as for the logo (fraction of the frame width for
    /// left/right, of the height for top/bottom). Absent: 0.04.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub margin: Option<f32>,
    /// Absent: circle.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shape: Option<AvatarShape>,
    /// Absent: no ring.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub border: Option<AvatarBorder>,
    /// `false` keeps the settings but shows no avatar. Absent: on.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,
}

/// A scene's own avatar settings, over the timeline's (or brand's).
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct SceneAvatar {
    /// `false`: no avatar in this scene. `true`: show it even when the default is off.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,
    /// Another narrator photo for this scene.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub photo: Option<String>,
    /// Another corner for this scene (for example when its text sits in the default corner).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub corner: Option<Corner>,
}

/// Render-only: the generated clip a scene shows, written by the scheduler (never by hand).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AvatarClip {
    /// The muted talking-head MP4, relative to the public dir.
    pub src: String,
    /// Clip frame shown on the scene's first frame (the clip covers a run of scenes).
    pub offset_frames: u32,
    pub corner: Corner,
    pub size: f32,
    pub margin: f32,
    pub shape: AvatarShape,
    /// Ring width as a fraction of the inset size; 0 = none.
    pub border_width: f32,
    pub border_color: String,
}

/// The settings a scene's avatar is drawn and generated with.
#[derive(Debug, Clone, PartialEq)]
pub struct ResolvedAvatar {
    pub photo: String,
    pub corner: Corner,
    pub size: f32,
    pub margin: f32,
    pub shape: AvatarShape,
    pub border_width: f32,
    pub border_color: String,
}

impl Avatar {
    pub fn with_photo(photo: &str) -> Self {
        Avatar {
            photo: photo.to_string(),
            corner: None,
            size: None,
            margin: None,
            shape: None,
            border: None,
            enabled: None,
        }
    }

    fn resolved(&self, photo: &str, corner: Option<Corner>) -> ResolvedAvatar {
        ResolvedAvatar {
            photo: photo.to_string(),
            corner: corner.or(self.corner).unwrap_or(Corner::BottomRight),
            size: self.size.unwrap_or(DEFAULT_AVATAR_SIZE),
            margin: self.margin.unwrap_or(DEFAULT_AVATAR_MARGIN),
            shape: self.shape.unwrap_or_default(),
            border_width: self
                .border
                .as_ref()
                .map_or(0.0, |b| b.width.unwrap_or(DEFAULT_BORDER_WIDTH)),
            border_color: self
                .border
                .as_ref()
                .and_then(|b| b.color.clone())
                .unwrap_or_else(|| DEFAULT_BORDER_COLOR.to_string()),
        }
    }

    /// Every problem of the settings, as messages.
    pub fn problems(&self) -> Vec<String> {
        let mut out = photo_problems(&self.photo);
        if let Some(s) = self.size {
            if !(MIN_AVATAR_SIZE..=MAX_AVATAR_SIZE).contains(&s) {
                out.push(format!(
                    "size {s} must be between {MIN_AVATAR_SIZE} and {MAX_AVATAR_SIZE} (a fraction of the frame height)"
                ));
            }
        }
        if let Some(m) = self.margin {
            if !(0.0..=MAX_AVATAR_MARGIN).contains(&m) {
                out.push(format!(
                    "margin {m} must be between 0 and {MAX_AVATAR_MARGIN}"
                ));
            }
        }
        if let Some(b) = &self.border {
            if let Some(w) = b.width {
                if !(0.0..=MAX_BORDER_WIDTH).contains(&w) {
                    out.push(format!(
                        "border width {w} must be between 0 and {MAX_BORDER_WIDTH}"
                    ));
                }
            }
            if let Some(c) = &b.color {
                if !crate::layer::is_hex_color(c) {
                    out.push(format!("border color `{c}` is not a #RRGGBB color"));
                }
            }
        }
        out
    }
}

impl SceneAvatar {
    pub fn problems(&self) -> Vec<String> {
        self.photo
            .as_deref()
            .map(photo_problems)
            .unwrap_or_default()
    }
}

fn photo_problems(photo: &str) -> Vec<String> {
    let p = photo.trim();
    if p.is_empty() {
        return vec!["photo is empty".into()];
    }
    if let Some(problem) = crate::validation::asset_path_problem(p) {
        return vec![format!("photo `{p}`: {problem}")];
    }
    let ext = p.rsplit_once('.').map(|(_, e)| e.to_ascii_lowercase());
    match ext {
        Some(e) if AVATAR_PHOTO_EXTENSIONS.contains(&e.as_str()) => vec![],
        _ => vec![format!(
            "photo `{p}` must be a {} file",
            AVATAR_PHOTO_EXTENSIONS.join("/")
        )],
    }
}

/// The avatar a scene shows, or `None` (off). `default` is the timeline's avatar, else the
/// brand's.
pub fn resolve(default: Option<&Avatar>, own: Option<&SceneAvatar>) -> Option<ResolvedAvatar> {
    let own_enabled = own.and_then(|o| o.enabled);
    let own_photo = own.and_then(|o| o.photo.as_deref());
    let own_corner = own.and_then(|o| o.corner);
    match default {
        Some(d) => {
            let on = own_enabled.unwrap_or(d.enabled.unwrap_or(true));
            on.then(|| d.resolved(own_photo.unwrap_or(&d.photo), own_corner))
        }
        // No default: a scene can still bring its own narrator.
        None => match (own_enabled, own_photo) {
            (Some(false), _) | (_, None) => None,
            (_, Some(p)) => Some(Avatar::with_photo(p).resolved(p, own_corner)),
        },
    }
}

/// A fingerprint of everything the avatar clips of `t` depend on that is known before the
/// render (scene lengths, spoken text, voices, audio files, voice-over, avatar settings).
/// `vimmer avatar` stores clip status under it, so `vimmer check` can say whether the clips of
/// this timeline are ready without narrating. `None`: no scene can show an avatar.
pub fn plan_signature(t: &crate::model::VideoTimeline) -> Option<String> {
    let scenes: Vec<serde_json::Value> = t
        .scenes
        .iter()
        .enumerate()
        .map(|(i, s)| {
            serde_json::json!([
                s.id,
                s.duration_ms,
                s.spoken_text(),
                s.narration,
                s.audio,
                t.scene_avatar(i).map(|a| a.photo),
            ])
        })
        .collect();
    if t.scenes.iter().enumerate().all(|(i, _)| t.scene_avatar(i).is_none()) {
        return None;
    }
    let doc = serde_json::json!([t.fps, scenes, t.voiceover]).to_string();
    // FNV-1a 64: stable across runs and machines.
    let mut h: u64 = 0xcbf29ce484222325;
    for b in doc.as_bytes() {
        h ^= u64::from(*b);
        h = h.wrapping_mul(0x100000001b3);
    }
    Some(format!("{h:016x}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scene_overrides_resolve() {
        let d = Avatar::with_photo("me.png");
        let r = resolve(Some(&d), None).unwrap();
        assert_eq!(r.corner, Corner::BottomRight);
        assert_eq!(r.size, DEFAULT_AVATAR_SIZE);
        assert_eq!(r.border_width, 0.0);
        let off = SceneAvatar {
            enabled: Some(false),
            ..Default::default()
        };
        assert!(resolve(Some(&d), Some(&off)).is_none());
        let other = SceneAvatar {
            photo: Some("b.jpg".into()),
            corner: Some(Corner::TopLeft),
            ..Default::default()
        };
        let r = resolve(Some(&d), Some(&other)).unwrap();
        assert_eq!((r.photo.as_str(), r.corner), ("b.jpg", Corner::TopLeft));
        let mut disabled = d.clone();
        disabled.enabled = Some(false);
        assert!(resolve(Some(&disabled), None).is_none());
        let on = SceneAvatar {
            enabled: Some(true),
            ..Default::default()
        };
        assert_eq!(resolve(Some(&disabled), Some(&on)).unwrap().photo, "me.png");
        assert!(resolve(None, Some(&on)).is_none());
        assert_eq!(resolve(None, Some(&other)).unwrap().photo, "b.jpg");
    }

    #[test]
    fn plan_signature_follows_speech_and_photo_only() {
        let raw = r#"{"fps":30,"resolution":{"width":1920,"height":1080},"scenes":[{"id":"a","duration_ms":1000,"text":"Hi","narration":{"voice":null}}],"subtitles":[],"music":null}"#;
        let mut t: crate::model::VideoTimeline = serde_json::from_str(raw).unwrap();
        assert_eq!(plan_signature(&t), None);
        t.avatar = Some(Avatar::with_photo("me.png"));
        let a = plan_signature(&t).unwrap();
        t.avatar.as_mut().unwrap().corner = Some(Corner::TopLeft); // placement only
        assert_eq!(plan_signature(&t).unwrap(), a);
        t.scenes[0].text = Some("Hello".into());
        assert_ne!(plan_signature(&t).unwrap(), a);
    }

    #[test]
    fn defaults_are_not_written() {
        let a: Avatar = serde_json::from_str(r#"{"photo":"me.png"}"#).unwrap();
        assert_eq!(serde_json::to_string(&a).unwrap(), r#"{"photo":"me.png"}"#);
        assert_eq!(
            serde_json::to_string(&SceneAvatar::default()).unwrap(),
            "{}"
        );
    }

    #[test]
    fn problems_are_reported() {
        let mut a = Avatar::with_photo("me.gif");
        a.size = Some(0.9);
        a.margin = Some(-0.1);
        a.border = Some(AvatarBorder {
            color: Some("white".into()),
            width: Some(0.5),
        });
        assert_eq!(a.problems().len(), 5);
        assert!(Avatar::with_photo("ok.JPG").problems().is_empty());
        assert_eq!(Avatar::with_photo(" ").problems(), vec!["photo is empty"]);
    }
}
