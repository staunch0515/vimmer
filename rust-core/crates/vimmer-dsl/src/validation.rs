use std::collections::HashSet;

use thiserror::Error;

use crate::model::{Scene, SubtitleCue, VideoTimeline, MAX_DURATION_MS, MAX_FRAMES, TARGET_FPS};

const ALLOWED_RESOLUTIONS: [(u32, u32); 2] = [(1920, 1080), (1080, 1920)];

#[derive(Debug, Clone, PartialEq, Error)]
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
    #[error("subtitle {index} must have start < end (got {start_ms}..{end_ms} ms)")]
    BadSubtitleRange {
        index: usize,
        start_ms: u64,
        end_ms: u64,
    },
    #[error("subtitle {index} ends at {end_ms} ms, after the timeline ends at {total_ms} ms")]
    SubtitleOutOfRange {
        index: usize,
        end_ms: u64,
        total_ms: u64,
    },
    #[error("subtitle {0} has empty text")]
    EmptySubtitle(usize),
    #[error("subtitle {cue} word {word} ({start_ms}..{end_ms} ms) is outside the cue ({cue_start_ms}..{cue_end_ms} ms) or ends before it starts")]
    WordOutOfCue {
        cue: usize,
        word: usize,
        start_ms: u64,
        end_ms: u64,
        cue_start_ms: u64,
        cue_end_ms: u64,
    },
    #[error("subtitle {cue} word {word} starts before the previous word ends; words must be in time order without overlap")]
    WordsNotMonotonic { cue: usize, word: usize },
    #[error("subtitle {cue} word {word} has empty text")]
    EmptyWord { cue: usize, word: usize },
    #[error("music has an empty source")]
    EmptyMusicSource,
    #[error("music volume {0} must be between 0 and 1")]
    BadMusicVolume(f32),
    #[error("music fades ({fade_in_ms} ms in, {fade_out_ms} ms out) are longer than the timeline")]
    MusicFadesTooLong { fade_in_ms: u64, fade_out_ms: u64 },
    #[error("scene `{0}` is the first scene, so it cannot have a transition_in")]
    TransitionOnFirstScene(String),
    #[error("scene `{0}` has a zero-length transition")]
    ZeroTransition(String),
    #[error(
        "scene `{scene}` transition is {transition_ms} ms, longer than the scene ({scene_ms} ms)"
    )]
    TransitionTooLong {
        scene: String,
        transition_ms: u64,
        scene_ms: u64,
    },
    #[error(
        "scene `{scene}` keyframe {index} is at {at_ms} ms, after the scene ends at {scene_ms} ms"
    )]
    KeyframeOutOfScene {
        scene: String,
        index: usize,
        at_ms: u64,
        scene_ms: u64,
    },
    #[error("scene `{scene}` keyframes must be sorted by at_ms (keyframe {index} is earlier than the one before it)")]
    KeyframesNotSorted { scene: String, index: usize },
    #[error("scene `{scene}` has two keyframes at {at_ms} ms")]
    DuplicateKeyframe { scene: String, at_ms: u64 },
    #[error(
        "scene `{scene}` keyframe {index} has scale {value}; scale must be a finite number above 0"
    )]
    BadKeyframeScale {
        scene: String,
        index: usize,
        value: f32,
    },
    #[error(
        "scene `{scene}` keyframe {index} has opacity {value}; opacity must be between 0 and 1"
    )]
    BadKeyframeOpacity {
        scene: String,
        index: usize,
        value: f32,
    },
    #[error("scene `{scene}` keyframe {index} has a non-finite x, y or rotation")]
    NonFiniteKeyframe { scene: String, index: usize },
}

fn validate_transition(scene: &Scene, index: usize, errors: &mut Vec<ValidationError>) {
    let Some(transition) = &scene.transition_in else {
        return;
    };
    if index == 0 {
        errors.push(ValidationError::TransitionOnFirstScene(scene.id.clone()));
    }
    if transition.duration_ms == 0 {
        errors.push(ValidationError::ZeroTransition(scene.id.clone()));
    } else if transition.duration_ms > scene.duration_ms {
        errors.push(ValidationError::TransitionTooLong {
            scene: scene.id.clone(),
            transition_ms: transition.duration_ms,
            scene_ms: scene.duration_ms,
        });
    }
}

fn validate_keyframes(scene: &Scene, errors: &mut Vec<ValidationError>) {
    for (index, kf) in scene.keyframes.iter().enumerate() {
        if kf.at_ms > scene.duration_ms {
            errors.push(ValidationError::KeyframeOutOfScene {
                scene: scene.id.clone(),
                index,
                at_ms: kf.at_ms,
                scene_ms: scene.duration_ms,
            });
        }
        if !(kf.scale.is_finite() && kf.scale > 0.0) {
            errors.push(ValidationError::BadKeyframeScale {
                scene: scene.id.clone(),
                index,
                value: kf.scale,
            });
        }
        if !(0.0..=1.0).contains(&kf.opacity) {
            errors.push(ValidationError::BadKeyframeOpacity {
                scene: scene.id.clone(),
                index,
                value: kf.opacity,
            });
        }
        if !(kf.x.is_finite() && kf.y.is_finite() && kf.rotation.is_finite()) {
            errors.push(ValidationError::NonFiniteKeyframe {
                scene: scene.id.clone(),
                index,
            });
        }
        if index > 0 {
            let prev = &scene.keyframes[index - 1];
            if kf.at_ms < prev.at_ms {
                errors.push(ValidationError::KeyframesNotSorted {
                    scene: scene.id.clone(),
                    index,
                });
            } else if kf.at_ms == prev.at_ms {
                errors.push(ValidationError::DuplicateKeyframe {
                    scene: scene.id.clone(),
                    at_ms: kf.at_ms,
                });
            }
        }
    }
}

/// Word timings must sit inside their cue, in order, without overlapping. Zero-length words are
/// allowed: chunking clamps words outside a chunk to the chunk edge, which keeps them in order.
fn validate_words(cue: &SubtitleCue, index: usize, errors: &mut Vec<ValidationError>) {
    let mut prev_end = cue.start_ms;
    for (word, w) in cue.words.iter().enumerate() {
        if w.text.trim().is_empty() {
            errors.push(ValidationError::EmptyWord { cue: index, word });
        }
        if w.start_ms > w.end_ms || w.start_ms < cue.start_ms || w.end_ms > cue.end_ms {
            errors.push(ValidationError::WordOutOfCue {
                cue: index,
                word,
                start_ms: w.start_ms,
                end_ms: w.end_ms,
                cue_start_ms: cue.start_ms,
                cue_end_ms: cue.end_ms,
            });
        } else if word > 0 && w.start_ms < prev_end {
            errors.push(ValidationError::WordsNotMonotonic { cue: index, word });
        }
        prev_end = prev_end.max(w.end_ms);
    }
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
    for (i, scene) in timeline.scenes.iter().enumerate() {
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
        validate_transition(scene, i, &mut errors);
        validate_keyframes(scene, &mut errors);
    }

    let total_ms = timeline.total_duration_ms();
    if total_ms > MAX_DURATION_MS {
        errors.push(ValidationError::TooLong(total_ms));
    }
    let total_frames = timeline.total_frames();
    if total_frames > MAX_FRAMES {
        errors.push(ValidationError::TooManyFrames(total_frames));
    }

    for (index, cue) in timeline.subtitles.iter().enumerate() {
        if cue.start_ms >= cue.end_ms {
            errors.push(ValidationError::BadSubtitleRange {
                index,
                start_ms: cue.start_ms,
                end_ms: cue.end_ms,
            });
        } else if cue.end_ms > total_ms {
            errors.push(ValidationError::SubtitleOutOfRange {
                index,
                end_ms: cue.end_ms,
                total_ms,
            });
        }
        if cue.text.trim().is_empty() {
            errors.push(ValidationError::EmptySubtitle(index));
        }
        validate_words(cue, index, &mut errors);
    }

    if let Some(music) = &timeline.music {
        if music.source.trim().is_empty() {
            errors.push(ValidationError::EmptyMusicSource);
        }
        if !(0.0..=1.0).contains(&music.volume) {
            errors.push(ValidationError::BadMusicVolume(music.volume));
        }
        if music.fade_in_ms + music.fade_out_ms > total_ms {
            errors.push(ValidationError::MusicFadesTooLong {
                fade_in_ms: music.fade_in_ms,
                fade_out_ms: music.fade_out_ms,
            });
        }
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
    use crate::model::{MotionType, Resolution, Scene, SubtitleCue, WordTiming};

    fn scene(id: &str, ms: u64) -> Scene {
        Scene {
            id: id.into(),
            duration_ms: ms,
            text: Some("hello".into()),
            highlights: vec![],
            image: None,
            motion: MotionType::Static,
            audio: None,
            narration: None,
            transition_in: None,
            keyframes: vec![],
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
            subtitles: vec![],
            music: None,
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
            subtitles: vec![],
            music: None,
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
    fn checks_subtitle_cues() {
        let mut t = timeline(vec![scene("a", 5_000)]);
        t.subtitles = vec![
            SubtitleCue {
                start_ms: 2_000,
                end_ms: 1_000,
                text: "backwards".into(),
                words: Vec::new(),
            },
            SubtitleCue {
                start_ms: 4_000,
                end_ms: 6_000,
                text: "too late".into(),
                words: Vec::new(),
            },
            SubtitleCue {
                start_ms: 0,
                end_ms: 1_000,
                text: "  ".into(),
                words: Vec::new(),
            },
        ];
        let errs = validate(&t).unwrap_err();
        assert!(errs.contains(&ValidationError::BadSubtitleRange {
            index: 0,
            start_ms: 2_000,
            end_ms: 1_000
        }));
        assert!(errs.contains(&ValidationError::SubtitleOutOfRange {
            index: 1,
            end_ms: 6_000,
            total_ms: 5_000
        }));
        assert!(errs.contains(&ValidationError::EmptySubtitle(2)));
    }

    fn word(text: &str, start_ms: u64, end_ms: u64) -> WordTiming {
        WordTiming {
            text: text.into(),
            start_ms,
            end_ms,
        }
    }

    #[test]
    fn valid_words_pass_including_zero_length_edges() {
        let mut t = timeline(vec![scene("a", 5_000)]);
        let mut cue = SubtitleCue::new(1_000, 3_000, "你好 world");
        cue.words = vec![
            word("你", 1_000, 1_000),
            word("好 ", 1_000, 1_400),
            word("world", 1_500, 3_000),
        ];
        t.subtitles = vec![cue];
        assert_eq!(validate(&t), Ok(()));
    }

    #[test]
    fn words_must_sit_inside_their_cue_in_order() {
        let mut t = timeline(vec![scene("a", 5_000)]);
        let mut cue = SubtitleCue::new(1_000, 3_000, "a b c d e");
        cue.words = vec![
            word("a", 900, 1_200),
            word("b", 1_200, 1_600),
            word("c", 1_500, 1_800),
            word("d", 2_000, 1_900),
            word(" ", 2_900, 3_100),
        ];
        t.subtitles = vec![cue];
        let errs = validate(&t).unwrap_err();
        assert!(errs.iter().any(|e| matches!(
            e,
            ValidationError::WordOutOfCue {
                cue: 0,
                word: 0,
                ..
            }
        )));
        assert!(errs.contains(&ValidationError::WordsNotMonotonic { cue: 0, word: 2 }));
        assert!(errs
            .iter()
            .any(|e| matches!(e, ValidationError::WordOutOfCue { word: 3, .. })));
        assert!(errs
            .iter()
            .any(|e| matches!(e, ValidationError::WordOutOfCue { word: 4, .. })));
        assert!(errs.contains(&ValidationError::EmptyWord { cue: 0, word: 4 }));
        assert!(!errs.contains(&ValidationError::WordsNotMonotonic { cue: 0, word: 1 }));
    }

    #[test]
    fn checks_music() {
        use crate::model::Music;
        let mut t = timeline(vec![scene("a", 5_000)]);
        t.music = Some(Music {
            source: " ".into(),
            volume: 1.5,
            fade_in_ms: 4_000,
            fade_out_ms: 4_000,
        });
        let errs = validate(&t).unwrap_err();
        assert!(errs.contains(&ValidationError::EmptyMusicSource));
        assert!(errs.contains(&ValidationError::BadMusicVolume(1.5)));
        assert!(errs.contains(&ValidationError::MusicFadesTooLong {
            fade_in_ms: 4_000,
            fade_out_ms: 4_000
        }));
        t.music = Some(Music {
            source: "assets/m.mp3".into(),
            volume: 0.3,
            fade_in_ms: 500,
            fade_out_ms: 500,
        });
        assert!(validate(&t).is_ok());
    }

    #[test]
    fn rejects_empty_timeline() {
        let errs = validate(&timeline(vec![])).unwrap_err();
        assert_eq!(errs, vec![ValidationError::NoScenes]);
    }

    fn kf(at_ms: u64) -> crate::model::Keyframe {
        crate::model::Keyframe {
            at_ms,
            scale: 1.0,
            x: 0.0,
            y: 0.0,
            opacity: 1.0,
            rotation: 0.0,
            ease: crate::model::Easing::EaseInOut,
        }
    }

    fn fade(ms: u64) -> Option<crate::model::Transition> {
        Some(crate::model::Transition {
            kind: crate::model::TransitionKind::Fade,
            duration_ms: ms,
        })
    }

    #[test]
    fn transition_on_first_scene_is_rejected() {
        let mut first = scene("a", 5_000);
        first.transition_in = fade(500);
        let errs = validate(&timeline(vec![first])).unwrap_err();
        assert_eq!(
            errs,
            vec![ValidationError::TransitionOnFirstScene("a".into())]
        );
    }

    #[test]
    fn transition_on_later_scene_is_allowed() {
        let mut second = scene("b", 5_000);
        second.transition_in = fade(500);
        assert!(validate(&timeline(vec![scene("a", 5_000), second])).is_ok());
    }

    #[test]
    fn transition_longer_than_its_scene_is_rejected() {
        let mut second = scene("b", 1_000);
        second.transition_in = fade(1_500);
        let errs = validate(&timeline(vec![scene("a", 5_000), second])).unwrap_err();
        assert_eq!(
            errs,
            vec![ValidationError::TransitionTooLong {
                scene: "b".into(),
                transition_ms: 1_500,
                scene_ms: 1_000,
            }]
        );
    }

    #[test]
    fn transition_equal_to_scene_length_is_allowed() {
        let mut second = scene("b", 1_000);
        second.transition_in = fade(1_000);
        assert!(validate(&timeline(vec![scene("a", 5_000), second])).is_ok());
    }

    #[test]
    fn zero_length_transition_is_rejected() {
        let mut second = scene("b", 1_000);
        second.transition_in = fade(0);
        let errs = validate(&timeline(vec![scene("a", 5_000), second])).unwrap_err();
        assert_eq!(errs, vec![ValidationError::ZeroTransition("b".into())]);
    }

    #[test]
    fn keyframes_must_be_sorted_without_duplicates() {
        let mut a = scene("a", 4_000);
        a.keyframes = vec![kf(0), kf(3_000), kf(1_000)];
        let errs = validate(&timeline(vec![a])).unwrap_err();
        assert_eq!(
            errs,
            vec![ValidationError::KeyframesNotSorted {
                scene: "a".into(),
                index: 2,
            }]
        );

        let mut b = scene("b", 4_000);
        b.keyframes = vec![kf(1_000), kf(1_000)];
        let errs = validate(&timeline(vec![b])).unwrap_err();
        assert_eq!(
            errs,
            vec![ValidationError::DuplicateKeyframe {
                scene: "b".into(),
                at_ms: 1_000,
            }]
        );
    }

    #[test]
    fn keyframe_past_scene_end_is_rejected_and_end_is_allowed() {
        let mut a = scene("a", 4_000);
        a.keyframes = vec![kf(0), kf(4_000), kf(4_001)];
        let errs = validate(&timeline(vec![a])).unwrap_err();
        assert_eq!(
            errs,
            vec![ValidationError::KeyframeOutOfScene {
                scene: "a".into(),
                index: 2,
                at_ms: 4_001,
                scene_ms: 4_000,
            }]
        );
    }

    #[test]
    fn keyframe_values_are_range_checked() {
        let mut a = scene("a", 4_000);
        let mut bad = kf(0);
        bad.scale = 0.0;
        let mut bad2 = kf(1_000);
        bad2.opacity = 1.5;
        let mut bad3 = kf(2_000);
        bad3.x = f32::NAN;
        a.keyframes = vec![bad, bad2, bad3];
        let errs = validate(&timeline(vec![a])).unwrap_err();
        assert!(errs.contains(&ValidationError::BadKeyframeScale {
            scene: "a".into(),
            index: 0,
            value: 0.0,
        }));
        assert!(errs.contains(&ValidationError::BadKeyframeOpacity {
            scene: "a".into(),
            index: 1,
            value: 1.5,
        }));
        assert!(errs.contains(&ValidationError::NonFiniteKeyframe {
            scene: "a".into(),
            index: 2,
        }));
    }

    #[test]
    fn valid_keyframes_and_transition_pass() {
        let mut a = scene("a", 4_000);
        a.keyframes = vec![kf(0), kf(2_000), kf(4_000)];
        let mut b = scene("b", 4_000);
        b.transition_in = fade(1_000);
        assert!(validate(&timeline(vec![a, b])).is_ok());
    }
}
