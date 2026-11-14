//! Durations: DSL validation, audio offsets, subtitle cue timing, transitions, and narration
//! that will not fit its scene.
//!
//! * Cue shorter than [`MIN_CUE_MS`] (833 ms, Netflix's 5/6 s minimum): warning. Shorter than
//!   [`CUE_FADE_MS`] (400 ms): error, since the renderer's 6-frame fade-in and fade-out never let
//!   the cue reach full opacity.
//! * Overlapping cues: the renderer shows only the first matching cue, so the overlapped part of
//!   the later cue is hidden. Warning; error when a cue is hidden completely.
//! * A transition longer than the previous scene: warning (the held last frame shows longer than
//!   that scene did). Longer than its own scene is a validation error.
//! * Narration estimate: the shared estimator ([`vimmer_media::estimate_speech_ms`]), fitted on
//!   the default `say` and Kokoro voices per language (`vimmer-media/calibration/`), for the engine
//!   the render will use ([`crate::CheckOptions::tts`]), plus the 300 ms tail the narration step
//!   adds. On the calibration corpus it is within about 20% per sentence, and other voices differ
//!   more, so a scene is flagged (warning) only when even a 20% faster voice would not fit, and
//!   as info when only the plain estimate does not. `--synth` measures instead and reports exact
//!   numbers.

use std::path::{Path, PathBuf};

use serde_json::json;
use vimmer_dsl::{validate, ValidationError, VideoTimeline};
use vimmer_media::{
    default_voice_for, looks_like_kokoro_voice, AudioProbe, FfmpegProbe, SaySpeech, Speech,
    SpeechLang, TtsEngine,
};

use crate::audio::TAIL_MS;
use crate::text::Lang;
use crate::{cue_path, scene_path, scene_starts_ms, CheckOptions, Finding, Severity, Suggestion};

pub const MIN_CUE_MS: u64 = 833;
pub const CUE_FADE_MS: u64 = 400;
/// Narration speed margin (see the module docs).
pub const NARRATION_MARGIN: f64 = 0.80;

/// DSL validation errors, except those the checks below report in more detail.
pub fn validation(timeline: &VideoTimeline) -> Vec<Finding> {
    let Err(errors) = validate(timeline) else {
        return Vec::new();
    };
    errors
        .into_iter()
        .filter(|e| {
            !matches!(
                e,
                ValidationError::SubtitleOutOfRange { .. }
                    | ValidationError::WordOutOfCue { .. }
                    | ValidationError::WordsNotMonotonic { .. }
            )
        })
        .map(|e| {
            Finding::new(
                Severity::Error,
                "dsl.invalid",
                format!("the timeline is invalid and will not render: {e}"),
                Suggestion::text("fix the field named in the message"),
            )
        })
        .collect()
}

pub fn check(timeline: &VideoTimeline) -> Vec<Finding> {
    let mut out = Vec::new();
    let starts = scene_starts_ms(timeline);
    let total = timeline.total_duration_ms();

    for (i, scene) in timeline.scenes.iter().enumerate() {
        if let Some(audio) = &scene.audio {
            if audio.offset_ms >= scene.duration_ms && scene.duration_ms > 0 {
                out.push(
                    Finding::new(
                        Severity::Error,
                        "audio.offset_past_end",
                        format!(
                            "audio starts {} ms into a {} ms scene, so it never plays",
                            audio.offset_ms, scene.duration_ms
                        ),
                        Suggestion::set(
                            "start the audio with the scene",
                            scene_path(i, "audio/offset_ms"),
                            json!(0),
                        ),
                    )
                    .scene(&scene.id)
                    .at(starts[i]),
                );
            }
            if scene.narration.is_some() {
                out.push(
                    Finding::new(
                        Severity::Info,
                        "audio.narration_ignored",
                        "the scene has both audio and narration; the audio plays and no narration is synthesized",
                        Suggestion::set("drop the narration setting", scene_path(i, "narration"), serde_json::Value::Null),
                    )
                    .scene(&scene.id)
                    .at(starts[i]),
                );
            }
        }
        if i > 0 {
            if let Some(t) = scene.transition_in {
                let prev = &timeline.scenes[i - 1];
                if t.duration_ms > prev.duration_ms {
                    let fixed = (prev.duration_ms.min(scene.duration_ms) / 2).max(1);
                    out.push(
                        Finding::new(
                            Severity::Warning,
                            "transition.too_long",
                            format!(
                                "transition is {} ms, longer than the previous scene `{}` ({} ms)",
                                t.duration_ms, prev.id, prev.duration_ms
                            ),
                            Suggestion::set(
                                format!("shorten the transition to {fixed} ms"),
                                scene_path(i, "transition_in/duration_ms"),
                                json!(fixed),
                            ),
                        )
                        .scene(&scene.id)
                        .at(starts[i]),
                    );
                }
            }
        }
    }

    for (i, cue) in timeline.subtitles.iter().enumerate() {
        if cue.start_ms >= cue.end_ms {
            continue; // validation error
        }
        if cue.start_ms >= total {
            out.push(
                Finding::new(
                    Severity::Error,
                    "subtitle.outside_timeline",
                    format!(
                        "cue starts at {} ms, after the video ends at {total} ms; it never shows",
                        cue.start_ms
                    ),
                    Suggestion::text(format!(
                        "remove subtitles[{i}] or move it before {total} ms"
                    )),
                )
                .cue(i)
                .at(cue.start_ms),
            );
            continue;
        } else if cue.end_ms > total {
            out.push(
                Finding::new(
                    Severity::Error,
                    "subtitle.outside_timeline",
                    format!("cue ends at {} ms, after the video ends at {total} ms (the timeline does not validate)", cue.end_ms),
                    Suggestion::set(format!("end the cue at {total} ms"), cue_path(i, "end_ms"), json!(total)),
                )
                .cue(i)
                .at(cue.start_ms),
            );
        }

        let dur = cue.end_ms - cue.start_ms;
        if dur < MIN_CUE_MS {
            let next_start = timeline.subtitles.get(i + 1).map_or(total, |n| n.start_ms);
            let end = (cue.start_ms + MIN_CUE_MS).min(total);
            let suggestion = if end <= next_start && end > cue.end_ms {
                Suggestion::set(
                    format!("end the cue at {end} ms"),
                    cue_path(i, "end_ms"),
                    json!(end),
                )
            } else {
                Suggestion::text("merge the cue with a neighbour")
            };
            out.push(
                Finding::new(
                    if dur < CUE_FADE_MS {
                        Severity::Error
                    } else {
                        Severity::Warning
                    },
                    "subtitle.too_short",
                    format!(
                        "cue shows for {dur} ms; under {MIN_CUE_MS} ms it is hard to read{}",
                        if dur < CUE_FADE_MS {
                            ", and under 400 ms it never fades fully in"
                        } else {
                            ""
                        }
                    ),
                    suggestion,
                )
                .cue(i)
                .at(cue.start_ms),
            );
        }

        if let Some(next) = timeline.subtitles.get(i + 1) {
            if next.start_ms < cue.end_ms && next.start_ms >= cue.start_ms {
                let hidden_completely = next.end_ms <= cue.end_ms;
                out.push(
                    Finding::new(
                        if hidden_completely { Severity::Error } else { Severity::Warning },
                        "subtitle.overlap",
                        format!(
                            "cue overlaps the next cue by {} ms; the renderer shows only one cue at a time, so the next one is hidden {}",
                            cue.end_ms.min(next.end_ms) - next.start_ms,
                            if hidden_completely { "completely" } else { "until this one ends" }
                        ),
                        Suggestion::set(
                            format!("end this cue at {} ms", next.start_ms),
                            cue_path(i, "end_ms"),
                            json!(next.start_ms),
                        ),
                    )
                    .cue(i)
                    .at(next.start_ms),
                );
            } else if next.start_ms < cue.start_ms {
                out.push(
                    Finding::new(
                        Severity::Warning,
                        "subtitle.out_of_order",
                        format!(
                            "the next cue starts earlier ({} ms) than this one",
                            next.start_ms
                        ),
                        Suggestion::text("sort the subtitles by start_ms"),
                    )
                    .cue(i)
                    .at(cue.start_ms),
                );
            }
        }

        let mut prev_end = cue.start_ms;
        for (w, word) in cue.words.iter().enumerate() {
            let outside = word.start_ms < cue.start_ms
                || word.end_ms > cue.end_ms
                || word.end_ms < word.start_ms;
            let unordered = word.start_ms < prev_end && w > 0;
            if outside || unordered {
                out.push(
                    Finding::new(
                        Severity::Error,
                        if outside {
                            "subtitle.word_outside_cue"
                        } else {
                            "subtitle.words_out_of_order"
                        },
                        format!(
                            "word {w} `{}` ({}..{} ms) {} (the timeline does not validate)",
                            word.text.trim(),
                            word.start_ms,
                            word.end_ms,
                            if outside {
                                format!(
                                    "lies outside its cue ({}..{} ms)",
                                    cue.start_ms, cue.end_ms
                                )
                            } else {
                                "starts before the previous word ends".to_string()
                            }
                        ),
                        Suggestion::set(
                            "drop the word timing; the next render times the cue again",
                            cue_path(i, "words"),
                            json!([]),
                        ),
                    )
                    .cue(i)
                    .at(word.start_ms),
                );
                break;
            }
            prev_end = word.end_ms;
        }
    }
    out
}

/// Estimated speech length in ms (without the tail) of `text` read by `engine`: the shared
/// estimator [`vimmer_media::estimate_speech_ms`], in the language the engine reads it.
pub fn estimate_speech_ms(text: &str, engine: TtsEngine) -> u64 {
    vimmer_media::estimate_speech_ms(text, SpeechLang::of(text), engine)
}

/// Scenes whose narration is synthesized at render time.
fn narrated(timeline: &VideoTimeline) -> impl Iterator<Item = (usize, &vimmer_dsl::Scene, &str)> {
    timeline.scenes.iter().enumerate().filter_map(|(i, s)| {
        let text = s.text.as_deref().filter(|t| !t.trim().is_empty())?;
        (s.narration.is_some() && s.audio.is_none()).then_some((i, s, text))
    })
}

/// Narration longer than its scene. `measured[i]` is the synthesized length of scene `i`, when
/// known.
pub fn narration(
    timeline: &VideoTimeline,
    measured: Option<&[Option<u64>]>,
    engine: TtsEngine,
) -> Vec<Finding> {
    let mut out = Vec::new();
    let starts = scene_starts_ms(timeline);
    let has_cues = !timeline.subtitles.is_empty();
    for (i, scene, text) in narrated(timeline) {
        let shift_note = |extra: u64| {
            if has_cues && i + 1 < timeline.scenes.len() {
                format!("; later scenes start {extra} ms later but subtitle cues keep their times, so cues after it drift")
            } else {
                String::new()
            }
        };
        if let Some(Some(speech)) = measured.map(|m| m.get(i).copied().flatten()) {
            let needed = speech + TAIL_MS;
            if needed > scene.duration_ms {
                out.push(
                    Finding::new(
                        Severity::Warning,
                        "narration.too_long",
                        format!(
                            "measured narration is {speech} ms (+{TAIL_MS} ms tail): the render stretches the scene from {} to {needed} ms{}",
                            scene.duration_ms,
                            shift_note(needed - scene.duration_ms)
                        ),
                        Suggestion::set(format!("set the scene to {needed} ms"), scene_path(i, "duration_ms"), json!(needed)),
                    )
                    .scene(&scene.id)
                    .at(starts[i]),
                );
            }
            continue;
        }
        let speech = estimate_speech_ms(text, engine);
        let nominal = speech + TAIL_MS;
        let optimistic = (speech as f64 * NARRATION_MARGIN) as u64 + TAIL_MS;
        if nominal <= scene.duration_ms {
            continue;
        }
        let needed = nominal.div_ceil(100) * 100;
        out.push(
            Finding::new(
                if optimistic > scene.duration_ms { Severity::Warning } else { Severity::Info },
                "narration.too_long",
                format!(
                    "narration is estimated at about {nominal} ms for a {} ms scene ({}, {}); the render would stretch the scene{}",
                    scene.duration_ms,
                    Lang::of(text).code(),
                    engine.as_str(),
                    shift_note(nominal - scene.duration_ms)
                ),
                Suggestion::set(
                    format!("set the scene to about {needed} ms (run with --synth to measure)"),
                    scene_path(i, "duration_ms"),
                    json!(needed),
                ),
            )
            .scene(&scene.id)
            .at(starts[i]),
        );
    }
    out
}

/// Synthesizes the narration of every narrated scene with `say`, through the render's cache
/// (`<public>/generated/tts`, same file names as the narration step), and measures it.
pub fn synthesize_narration(
    timeline: &VideoTimeline,
    opts: &CheckOptions,
) -> (Vec<Option<u64>>, Vec<Finding>) {
    let mut measured = vec![None; timeline.scenes.len()];
    let mut notes = Vec::new();
    let Some(public) = &opts.public_dir else {
        return (measured, notes);
    };
    if opts.tts == TtsEngine::Kokoro {
        notes.push(Finding::new(
            Severity::Info,
            "narration.engine",
            "the render narrates with Kokoro, but --synth measures `say`; Kokoro narration lengths differ".to_string(),
            Suggestion::text("treat the measured lengths as approximate"),
        ));
    }
    let speech = SaySpeech {
        ffmpeg: opts.ffmpeg.clone(),
        ..SaySpeech::default()
    };
    let probe = FfmpegProbe {
        binary: opts.ffmpeg.clone(),
    };
    let tts_dir = public.join("generated").join("tts");
    for (i, scene, text) in narrated(timeline) {
        let voice = match scene
            .narration
            .as_ref()
            .and_then(|n| n.voice.as_deref())
            .filter(|v| !v.trim().is_empty())
        {
            Some(v) if !looks_like_kokoro_voice(v.trim()) => v.to_string(),
            _ => default_voice_for(text).to_string(),
        };
        let dst = say_cache_path(&tts_dir, &scene.id, &voice, text);
        if !dst.exists() {
            if let Err(e) = speech.synthesize(text, &voice, &dst) {
                notes.push(
                    Finding::new(
                        Severity::Warning,
                        "narration.synth_failed",
                        format!("could not synthesize the narration with `say` ({e}); using the estimate"),
                        Suggestion::text("run on macOS with the voice installed, or drop --synth"),
                    )
                    .scene(&scene.id),
                );
                continue;
            }
        }
        match probe.duration_ms(&dst) {
            Ok(ms) => measured[i] = Some(ms),
            Err(e) => notes.push(
                Finding::new(
                    Severity::Warning,
                    "narration.synth_failed",
                    format!("could not measure the synthesized narration: {e}"),
                    Suggestion::text("check that ffmpeg is installed"),
                )
                .scene(&scene.id),
            ),
        }
    }
    (measured, notes)
}

/// The narration step's cache file for `say`: `<scene id>-<fnv1a(voice\ntext)>.wav`.
/// Kept identical to `vimmer-scheduler/src/narrate.rs` so the render reuses what --synth made.
pub fn say_cache_path(tts_dir: &Path, scene_id: &str, voice: &str, text: &str) -> PathBuf {
    let id: String = scene_id
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect::<String>()
        .trim_matches('-')
        .to_string();
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in format!("{voice}\n{text}").bytes() {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x0100_0000_01b3);
    }
    tts_dir.join(format!("{id}-{hash:016x}.wav"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn estimates_use_the_shared_estimator_in_the_language_of_the_text() {
        for (text, lang) in [
            ("西安钟楼，始建于明代。", SpeechLang::Zh),
            ("こんにちは、世界。", SpeechLang::Ja),
            ("The quick brown fox", SpeechLang::En),
        ] {
            for engine in [TtsEngine::Say, TtsEngine::Kokoro] {
                assert_eq!(
                    estimate_speech_ms(text, engine),
                    vimmer_media::estimate_speech_ms(text, lang, engine)
                );
            }
        }
        // Kokoro reads short text more slowly than `say` (its lead-in is longer).
        assert!(
            estimate_speech_ms("你好。", TtsEngine::Kokoro)
                > estimate_speech_ms("你好。", TtsEngine::Say)
        );
    }

    #[test]
    fn cache_name_matches_the_narration_step() {
        // fnv1a("Samantha\nhi") computed independently.
        let p = say_cache_path(Path::new("/t"), "scene 01", "Samantha", "hi");
        let name = p.file_name().unwrap().to_str().unwrap();
        assert!(
            name.starts_with("scene-01-") && name.ends_with(".wav"),
            "{name}"
        );
        assert_eq!(name.len(), "scene-01-".len() + 16 + 4);
    }
}
