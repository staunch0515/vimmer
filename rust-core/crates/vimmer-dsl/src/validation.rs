use std::collections::HashSet;

use thiserror::Error;

use crate::model::{VideoTimeline, MAX_DURATION_MS, MAX_FRAMES, TARGET_FPS};

const ALLOWED_RESOLUTIONS: [(u32, u32); 2] = [(1920, 1080), (1080, 1920)];

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ValidationError {
    #[error("timeline has no scenes")]
    NoScenes,
    #[error("unsupported fps {0}; expected {TARGET_FPS}")]
    UnsupportedFps(u32),
    #[error("unsupported resolution {0}x{1}; expected 1920x1080 or 1080x1920")]
    UnsupportedResolution(u32, u32),
    #[error("duplicate scene id `{0}`")]
    DuplicateSceneId(String),
    #[error("scene `{0}` has zero duration")]
    ZeroDuration(String),
    #[error("scene `{0}` has neither text nor image")]
    EmptyScene(String),
    #[error("scene `{0}` has an empty audio source")]
    EmptyAudioSource(String),
    #[error("timeline is {0} ms; maximum is {MAX_DURATION_MS} ms")]
    TooLong(u64),
    #[error("timeline is {0} frames; maximum is {MAX_FRAMES} frames")]
    TooManyFrames(u32),
}

/// Checks a timeline against the DSL rules. Returns every violation found,
/// not just the first, so callers can report them all at once.
pub fn validate(timeline: &VideoTimeline) -> Result<(), Vec<ValidationError>> {
    let mut errors = Vec::new();

    if timeline.scenes.is_empty() {
        errors.push(ValidationError::NoScenes);
    }
    if timeline.fps != TARGET_FPS {
        errors.push(ValidationError::UnsupportedFps(timeline.fps));
    }
    let res = (timeline.resolution.width, timeline.resolution.height);
    if !ALLOWED_RESOLUTIONS.contains(&res) {
        errors.push(ValidationError::UnsupportedResolution(res.0, res.1));
    }

    let mut seen = HashSet::new();
    for scene in &timeline.scenes {
        if !seen.insert(scene.id.as_str()) {
            errors.push(ValidationError::DuplicateSceneId(scene.id.clone()));
        }
        if scene.duration_ms == 0 {
            errors.push(ValidationError::ZeroDuration(scene.id.clone()));
        }
        let has_text = scene.text.as_deref().is_some_and(|t| !t.trim().is_empty());
        let has_image = scene.image.as_deref().is_some_and(|i| !i.trim().is_empty());
        if !has_text && !has_image {
            errors.push(ValidationError::EmptyScene(scene.id.clone()));
        }
        if let Some(audio) = &scene.audio {
            if audio.source.trim().is_empty() {
                errors.push(ValidationError::EmptyAudioSource(scene.id.clone()));
            }
        }
    }

    let total_ms = timeline.total_duration_ms();
    if total_ms > MAX_DURATION_MS {
        errors.push(ValidationError::TooLong(total_ms));
    }
    let total_frames = timeline.total_frames();
    if total_frames > MAX_FRAMES {
        errors.push(ValidationError::TooManyFrames(total_frames));
    }

    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{MotionType, Resolution, Scene};

    fn scene(id: &str, ms: u64) -> Scene {
        Scene {
            id: id.into(),
            duration_ms: ms,
            text: Some("hello".into()),
            highlights: vec![],
            image: None,
            motion: MotionType::Static,
            audio: None,
        }
    }

    fn timeline(scenes: Vec<Scene>) -> VideoTimeline {
        VideoTimeline {
            fps: 30,
            resolution: Resolution {
                width: 1920,
                height: 1080,
            },
            scenes,
        }
    }

    #[test]
    fn valid_timeline_passes() {
        assert!(validate(&timeline(vec![scene("a", 15_000)])).is_ok());
    }

    #[test]
    fn reports_all_errors_at_once() {
        let mut bad = scene("dup", 0);
        bad.text = None;
        let t = VideoTimeline {
            fps: 24,
            resolution: Resolution {
                width: 640,
                height: 480,
            },
            scenes: vec![bad, scene("dup", 1000)],
        };
        let errs = validate(&t).unwrap_err();
        assert!(errs.contains(&ValidationError::UnsupportedFps(24)));
        assert!(errs.contains(&ValidationError::UnsupportedResolution(640, 480)));
        assert!(errs.contains(&ValidationError::DuplicateSceneId("dup".into())));
        assert!(errs.contains(&ValidationError::ZeroDuration("dup".into())));
        assert!(errs.contains(&ValidationError::EmptyScene("dup".into())));
    }

    #[test]
    fn rejects_over_200_seconds() {
        let t = timeline(vec![scene("a", 120_000), scene("b", 100_000)]);
        let errs = validate(&t).unwrap_err();
        assert!(errs.contains(&ValidationError::TooLong(220_000)));
        assert!(errs.contains(&ValidationError::TooManyFrames(6_600)));
    }

    #[test]
    fn exactly_200_seconds_is_allowed() {
        let t = timeline(vec![scene("a", 200_000)]);
        assert!(validate(&t).is_ok());
    }

    #[test]
    fn rejects_empty_timeline() {
        let errs = validate(&timeline(vec![])).unwrap_err();
        assert_eq!(errs, vec![ValidationError::NoScenes]);
    }
}
