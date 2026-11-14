//! Word-by-word subtitles: recognize each synthesized narration clip again with a local
//! speech recognizer, align the result to the scene text, and attach per-word timing to the
//! subtitle cues that cover the clip (or create cues when there are none).

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use vimmer_dsl::{LayerContent, Scene, SubtitleCue, VideoTimeline, WordTiming};
use vimmer_media::{
    align_words, guess_language, split_units, AlignedWord, AudioProbe, Limits, MediaError,
    RecognizedToken, TranscribeRequest, Transcriber, WhisperCli,
};

use crate::control::{Progress, RenderContext};
use crate::run::{SchedulerConfig, SchedulerError};

/// How long a generated line stays up after its last word, unless the next line comes sooner.
const LINE_HOLD_MS: u64 = 600;
/// Gaps shorter than this between two generated lines are closed, so the band does not blink.
const LINE_JOIN_MS: u64 = 1_000;

/// Settings for word-by-word subtitles.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WordSubtitleConfig {
    /// On by default. It only touches scenes whose narration is synthesized, and it falls back to
    /// plain subtitles with a warning when the recognizer is not installed.
    pub enabled: bool,
    /// `whisper-cli` binary. `None`: `$VIMMER_WHISPER_CLI`, then `whisper-cli` on PATH.
    pub whisper_cli: Option<PathBuf>,
    /// Model file. `None`: `$VIMMER_WHISPER_MODEL`, then the default model under the home dir.
    pub model: Option<PathBuf>,
    /// Recognition language: a whisper code (`zh`, `en`, `ja`, ...) or `auto` to let whisper
    /// detect it. `None` guesses it from each scene's text.
    pub language: Option<String>,
    /// Whether to create subtitle cues from the narration text where no cue covers a narrated
    /// scene. Off by default, so a timeline renders the same subtitles it always did: word
    /// timing is only added to cues the timeline already has.
    pub auto_subtitles: AutoSubtitles,
}

/// Which narrated scenes get generated subtitle cues (only where no cue covers the narration;
/// cues already in the timeline are never removed and always get word timing).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AutoSubtitles {
    /// None.
    #[default]
    Off,
    /// Only narration that is not already on screen as text: a scene whose scene text or text
    /// layers show the whole narration gets no cue, so the sentence is not shown twice.
    On,
    /// Every narrated scene, even when the narration is on screen (captions for accessibility).
    All,
}

impl AutoSubtitles {
    /// Whether cues may be generated for `clip`.
    pub(crate) fn generates(self, clip: &NarrationClip) -> bool {
        match self {
            AutoSubtitles::Off => false,
            AutoSubtitles::On => !clip.on_screen,
            AutoSubtitles::All => true,
        }
    }
}

impl Default for WordSubtitleConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            whisper_cli: None,
            model: None,
            language: None,
            auto_subtitles: AutoSubtitles::Off,
        }
    }
}

/// A synthesized narration clip on the global timeline.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct NarrationClip {
    pub scene_index: usize,
    pub text: String,
    pub wav: PathBuf,
    /// Where the clip starts, in ms from the video start.
    pub start_ms: u64,
    /// The latest a cue for this clip may end: the end of its scene.
    pub scene_end_ms: u64,
    /// The whole narration is shown as text while it plays (see [`narration_on_screen`]).
    pub on_screen: bool,
}

/// Synthesized narration clips: scenes with `narration` whose audio is the generated file.
pub(crate) fn narration_clips(
    timeline: &VideoTimeline,
    cfg: &SchedulerConfig,
) -> Vec<NarrationClip> {
    let fps = u64::from(timeline.fps);
    let frame_ms = |frame: u32| (u64::from(frame) * 1000 + fps / 2) / fps;
    let starts = timeline.scene_start_frames();
    timeline
        .scenes
        .iter()
        .enumerate()
        .filter_map(|(i, scene)| {
            scene.narration.as_ref()?;
            let audio = scene.audio.as_ref()?;
            if !audio.source.starts_with("generated/tts/") {
                return None;
            }
            let text = scene.text.clone().filter(|t| !t.trim().is_empty())?;
            let scene_start = frame_ms(starts[i]);
            // Validation measures the timeline as the sum of `duration_ms`, which can be a few
            // ms shorter than the frame-rounded end, so cues must stay inside both.
            let ms_end: u64 = timeline.scenes[..=i].iter().map(|s| s.duration_ms).sum();
            let scene_end = frame_ms(starts[i] + scene.duration_frames(timeline.fps)).min(ms_end);
            Some(NarrationClip {
                scene_index: i,
                on_screen: narration_on_screen(scene, &text, audio.offset_ms),
                text,
                wav: cfg.public_dir.join(&audio.source),
                start_ms: scene_start + audio.offset_ms,
                scene_end_ms: scene_end,
            })
        })
        .collect()
}

/// Text for comparing what is said with what is shown: letters and digits only, lowercased,
/// full-width forms folded to ASCII (`ＡＢＣ１` → `abc1`). Case, punctuation (ASCII or CJK),
/// whitespace and character width therefore never decide whether two texts match.
pub(crate) fn visible_key(text: &str) -> String {
    text.chars()
        .map(|c| match c {
            '\u{FF01}'..='\u{FF5E}' => char::from_u32(c as u32 - 0xFEE0).unwrap_or(c),
            _ => c,
        })
        .filter(|c| c.is_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

/// The text a scene shows while its narration plays, in drawing order: text layers under the
/// scene text, the scene text, text layers over it. A layer counts when it is on screen at any
/// time from `from_ms` (scene-relative, where the narration starts) to the scene end.
fn visible_texts(scene: &Scene, from_ms: u64) -> Vec<&str> {
    let layer_texts = |above: bool| {
        scene
            .layers
            .iter()
            .filter(move |l| {
                (l.z >= 0) == above
                    && l.opacity > 0.0
                    && l.start_ms < scene.duration_ms
                    && l.end_ms_in(scene.duration_ms) > from_ms
            })
            .flat_map(|l| match &l.content {
                LayerContent::Text(t) => vec![t.text.as_str()],
                LayerContent::LowerThird(lt) => std::iter::once(lt.title.as_str())
                    .chain(lt.subtitle.as_deref())
                    .collect(),
                _ => vec![],
            })
    };
    let mut texts: Vec<&str> = layer_texts(false).collect();
    texts.extend(scene.text.as_deref());
    texts.extend(layer_texts(true));
    texts
}

/// Whether the whole narration `spoken` (starting `offset_ms` into the scene) is already on
/// screen as text: its [`visible_key`] is part of the scene's visible text, read in drawing
/// order. Narration that only partly appears (a longer sentence under a short headline) is not.
pub(crate) fn narration_on_screen(scene: &Scene, spoken: &str, offset_ms: u64) -> bool {
    let shown: String = visible_texts(scene, offset_ms)
        .into_iter()
        .map(visible_key)
        .collect();
    shown.contains(&visible_key(spoken))
}

/// Runs recognition and alignment for every synthesized narration clip. Returns the number of
/// clips that got word timing. A missing engine or a failing clip only adds a warning: the
/// affected cues keep their plain behaviour.
pub async fn add_word_subtitles(
    timeline: &mut VideoTimeline,
    cfg: &SchedulerConfig,
    probe: Arc<dyn AudioProbe>,
    ctx: &RenderContext,
    warnings: &mut Vec<String>,
    notes: &mut Vec<String>,
) -> Result<usize, SchedulerError> {
    if !cfg.word_subtitles.enabled {
        return Ok(0);
    }
    drop_stale_words(&mut timeline.subtitles, warnings);
    let all = narration_clips(timeline, cfg);
    if cfg.word_subtitles.auto_subtitles == AutoSubtitles::On {
        let shown: Vec<&str> = all
            .iter()
            .filter(|c| c.on_screen && !has_cue_to_fill(&timeline.subtitles, c))
            .map(|c| timeline.scenes[c.scene_index].id.as_str())
            .collect();
        if !shown.is_empty() {
            notes.push(format!(
                "auto-subtitles: no cues for {} narrated scene(s) whose narration is already on screen as text ({}); `--auto-subtitles all` subtitles them too",
                shown.len(),
                shown.join(", ")
            ));
        }
    }
    let clips = clips_to_time(&timeline.subtitles, all, cfg.word_subtitles.auto_subtitles);
    if clips.is_empty() {
        return Ok(0);
    }
    let ws = &cfg.word_subtitles;
    let engine = WhisperCli::locate(
        ws.whisper_cli.as_deref(),
        ws.model.as_deref(),
        cfg.child_path.as_deref(),
    )
    .map(|mut engine| {
        engine.ffmpeg = cfg.ffmpeg.clone();
        engine
    });
    let engine: Arc<dyn Transcriber> = match engine {
        Ok(engine) => Arc::new(engine),
        Err(e) => {
            warnings.push(format!(
                "word-by-word subtitles skipped, subtitles stay as they are: {e}"
            ));
            return Ok(0);
        }
    };
    apply_word_subtitles(timeline, &clips, engine, probe, ws, ctx, warnings).await
}

/// Time budget for one recognition call: a fixed allowance for loading the model plus a
/// multiple of the audio length. Measured on a loaded 4-core Intel CPU: about 40 s for 4.6 s.
const TIMEOUT_BASE: Duration = Duration::from_secs(180);
const TIMEOUT_PER_AUDIO: u32 = 30;

/// Time budget for recognizing `audio_ms` of audio.
pub(crate) fn recognition_timeout(audio_ms: u64) -> Duration {
    TIMEOUT_BASE + Duration::from_millis(audio_ms) * TIMEOUT_PER_AUDIO
}

/// Batches never exceed one whisper window (30 s), since every call encodes a full window
/// however short the audio is.
const BATCH_MAX_MS: u64 = 28_000;
/// Silence between clips joined into one batch, so words cannot run from one clip into the next.
const BATCH_GAP_MS: u64 = 1_000;

/// Clips recognized in one whisper call: same language, joined with silence in between.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Batch {
    pub language: Option<String>,
    /// `(clip index, offset of the clip in the joined audio, clip length)`, in ms.
    pub members: Vec<(usize, u64, u64)>,
}

/// Groups clips by language, in timeline order, into batches of at most `BATCH_MAX_MS` of audio
/// (gaps included). A clip longer than that gets a batch of its own.
pub(crate) fn plan_batches(languages: &[Option<String>], durations: &[u64]) -> Vec<Batch> {
    let mut batches: Vec<Batch> = Vec::new();
    let mut open: Vec<(Option<String>, usize, u64)> = Vec::new(); // (language, batch, end ms)
    for (clip, (language, &ms)) in languages.iter().zip(durations).enumerate() {
        let slot = open.iter_mut().find(|(l, _, _)| l == language);
        match slot {
            Some((_, batch, end)) if *end + BATCH_GAP_MS + ms <= BATCH_MAX_MS => {
                let offset = *end + BATCH_GAP_MS;
                batches[*batch].members.push((clip, offset, ms));
                *end = offset + ms;
            }
            _ => {
                open.retain(|(l, _, _)| l != language);
                open.push((language.clone(), batches.len(), ms));
                batches.push(Batch {
                    language: language.clone(),
                    members: vec![(clip, 0, ms)],
                });
            }
        }
    }
    batches
}

/// Hands each recognized token of a joined batch back to the clip whose span (plus half the
/// surrounding gaps) holds the token's midpoint, with times made relative to that clip.
pub(crate) fn split_tokens(
    tokens: &[RecognizedToken],
    members: &[(usize, u64, u64)],
) -> Vec<Vec<RecognizedToken>> {
    let mut out = vec![Vec::new(); members.len()];
    for token in tokens {
        let mid = (token.start_ms + token.end_ms.max(token.start_ms)) / 2;
        let k = members
            .iter()
            .rposition(|&(_, offset, _)| offset <= mid + BATCH_GAP_MS / 2)
            .unwrap_or(0);
        let (_, offset, ms) = members[k];
        let rebase = |t: u64| t.saturating_sub(offset).min(ms);
        out[k].push(RecognizedToken::new(
            token.text.clone(),
            rebase(token.start_ms),
            rebase(token.end_ms.max(token.start_ms)),
        ));
    }
    out
}

/// Recognizes one batch: a lone clip directly, several clips joined into one file first.
fn recognize_batch(
    engine: &dyn Transcriber,
    batch: &Batch,
    clips: &[NarrationClip],
    limits: &Limits,
) -> Result<Vec<Vec<RecognizedToken>>, vimmer_media::MediaError> {
    let texts: Vec<&str> = batch
        .members
        .iter()
        .map(|&(c, _, _)| clips[c].text.as_str())
        .collect();
    let separator = if batch
        .language
        .as_deref()
        .is_some_and(|l| matches!(l, "zh" | "ja"))
    {
        ""
    } else {
        " "
    };
    let request = TranscribeRequest {
        language: batch.language.clone(),
        prompt: Some(texts.join(separator)),
    };
    if let [(clip, _, _)] = batch.members[..] {
        return Ok(vec![engine.transcribe(
            &clips[clip].wav,
            &request,
            limits,
        )?]);
    }
    let wavs: Vec<PathBuf> = batch
        .members
        .iter()
        .map(|&(c, _, _)| clips[c].wav.clone())
        .collect();
    let names: Vec<String> = wavs
        .iter()
        .map(|w| {
            w.file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default()
        })
        .collect();
    let dir = wavs[0].parent().map(PathBuf::from).unwrap_or_default();
    let joined = dir.join(format!(
        "batch-{:016x}.wav",
        fnv1a(&format!("{BATCH_GAP_MS}\n{}", names.join("\n")))
    ));
    // The joined file is only an intermediate: the recognition result is cached under its
    // name, and joining again is quick, so it is removed whatever happens.
    let result = engine
        .join_clips(&wavs, BATCH_GAP_MS, &joined, limits)
        .and_then(|()| engine.transcribe(&joined, &request, limits));
    let _ = std::fs::remove_file(&joined);
    Ok(split_tokens(&result?, &batch.members))
}

fn fnv1a(s: &str) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in s.bytes() {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x0100_0000_01b3);
    }
    hash
}

/// Whether a cue's words still spell its text, ignoring whitespace. Same rule as `wordsMatchText`
/// in remotion-renderer/src/subtitles/words.ts, which draws a failing cue as one plain line.
pub(crate) fn words_match_text(text: &str, words: &[WordTiming]) -> bool {
    let squash = |s: &str| s.chars().filter(|c| !c.is_whitespace()).collect::<String>();
    squash(&words.iter().map(|w| w.text.as_str()).collect::<String>()) == squash(text)
}

/// Clears the words of cues whose text was changed after the words were timed (an edited JSON),
/// so they are timed again like a cue that never had words.
pub(crate) fn drop_stale_words(subtitles: &mut [SubtitleCue], warnings: &mut Vec<String>) {
    for (i, cue) in subtitles.iter_mut().enumerate() {
        if !cue.words.is_empty() && !words_match_text(&cue.text, &cue.words) {
            cue.words.clear();
            warnings.push(format!(
                "subtitle {}: its words no longer match its text; timing it again",
                i + 1
            ));
        }
    }
}

/// Whether a clip's scene has a subtitle cue without words that overlaps it.
fn has_cue_to_fill(subtitles: &[SubtitleCue], clip: &NarrationClip) -> bool {
    subtitles
        .iter()
        .any(|c| c.words.is_empty() && c.start_ms < clip.scene_end_ms && c.end_ms > clip.start_ms)
}

/// The clips worth recognizing: those cues may be generated for, and those with a cue to fill.
/// Skipping the others saves a whisper call each.
pub(crate) fn clips_to_time(
    subtitles: &[SubtitleCue],
    clips: Vec<NarrationClip>,
    auto_subtitles: AutoSubtitles,
) -> Vec<NarrationClip> {
    clips
        .into_iter()
        .filter(|clip| auto_subtitles.generates(clip) || has_cue_to_fill(subtitles, clip))
        .collect()
}

pub(crate) async fn apply_word_subtitles(
    timeline: &mut VideoTimeline,
    clips: &[NarrationClip],
    engine: Arc<dyn Transcriber>,
    probe: Arc<dyn AudioProbe>,
    ws: &WordSubtitleConfig,
    ctx: &RenderContext,
    warnings: &mut Vec<String>,
) -> Result<usize, SchedulerError> {
    let scene_id = |timeline: &VideoTimeline, clip: &NarrationClip| {
        timeline.scenes[clip.scene_index].id.clone()
    };
    let languages: Vec<Option<String>> = clips
        .iter()
        .map(|clip| match ws.language.as_deref() {
            Some("auto") => None,
            Some(code) => Some(code.to_string()),
            None => guess_language(&clip.text).map(String::from),
        })
        .collect();
    let wavs: Vec<PathBuf> = clips.iter().map(|c| c.wav.clone()).collect();
    let probe2 = probe.clone();
    let durations = tokio::task::spawn_blocking(move || {
        wavs.iter()
            .map(|w| probe2.duration_ms(w))
            .collect::<Result<Vec<u64>, _>>()
    })
    .await
    .map_err(|e| SchedulerError::Join(e.to_string()))??;

    // One whisper call per batch of same-language clips; results land per clip.
    let batches = plan_batches(&languages, &durations);
    let mut recognized: Vec<Option<Vec<RecognizedToken>>> = vec![None; clips.len()];
    // Child processes poll the render's cancel flag, so a cancel stops whisper mid-run.
    let cancel = ctx.cancel.clone();
    let base_limits = Limits {
        timeout: None,
        cancelled: Arc::new(move || cancel.is_cancelled()),
    };
    let mut timed_out = false;
    for (n, batch) in batches.iter().enumerate() {
        if timed_out {
            // One call already ran out of time: the engine is too slow here, so do not spend the
            // same budget again on every remaining batch.
            for &(clip, _, _) in &batch.members {
                warnings.push(format!(
                    "scene `{}`: word-by-word subtitles skipped after an earlier recognition timed out",
                    scene_id(timeline, &clips[clip])
                ));
            }
            continue;
        }
        ctx.emit(Progress::Stage {
            name: format!("Timing subtitles ({}/{})", n + 1, batches.len()),
        });
        let audio_ms = batch
            .members
            .last()
            .map_or(0, |&(_, offset, ms)| offset + ms);
        let limits = base_limits.with_timeout(recognition_timeout(audio_ms));
        let (engine, batch_owned, clips_owned) = (engine.clone(), batch.clone(), clips.to_vec());
        let result = tokio::task::spawn_blocking(move || {
            recognize_batch(engine.as_ref(), &batch_owned, &clips_owned, &limits)
        })
        .await
        .map_err(|e| SchedulerError::Join(e.to_string()))?;
        ctx.check()?;
        match result {
            Ok(per_clip) => {
                for (&(clip, _, _), tokens) in batch.members.iter().zip(per_clip) {
                    recognized[clip] = Some(tokens);
                }
            }
            Err(MediaError::Cancelled) => return Err(SchedulerError::Cancelled),
            Err(e) => {
                timed_out |= matches!(e, MediaError::TimedOut { .. });
                for &(clip, _, _) in &batch.members {
                    warnings.push(format!(
                        "scene `{}`: word-by-word subtitles skipped: {e}",
                        scene_id(timeline, &clips[clip])
                    ));
                }
            }
        }
    }

    let max_cols = if timeline.resolution.width >= timeline.resolution.height {
        42
    } else {
        28
    };
    let clip_words: Vec<Option<Vec<AlignedWord>>> = clips
        .iter()
        .zip(&recognized)
        .zip(&durations)
        .map(|((clip, tokens), &ms)| {
            let tokens = tokens.as_ref()?;
            Some(
                align_words(&clip.text, tokens, ms)
                    .into_iter()
                    .map(|w| AlignedWord {
                        start_ms: w.start_ms + clip.start_ms,
                        end_ms: w.end_ms + clip.start_ms,
                        text: w.text,
                    })
                    .collect(),
            )
        })
        .collect();
    // Each cue is timed from the one clip whose narration its text matches best.
    let owners = assign_cues(&timeline.subtitles, clips, &clip_words);
    let mut timed = 0;
    for (k, clip) in clips.iter().enumerate() {
        let Some(words) = &clip_words[k] else {
            continue;
        };
        let id = scene_id(timeline, clip);
        if recognized[k].as_ref().is_some_and(Vec::is_empty) {
            warnings.push(format!(
                "scene `{id}`: speech recognition heard nothing; word timing is estimated from text length"
            ));
        }
        if merge_clip_words(
            &mut timeline.subtitles,
            clip,
            words,
            max_cols,
            ws.auto_subtitles.generates(clip),
            &|i| owners[i] == Some(k),
        ) {
            timed += 1;
        } else {
            warnings.push(format!(
                "scene `{id}`: existing subtitle cues do not match the narration text; they keep plain timing"
            ));
        }
    }
    timeline.subtitles.sort_by_key(|c| c.start_ms);
    Ok(timed)
}

/// The span of the timeline a clip's words may be shown in: from the clip start to the end of
/// its scene, or of its last word when that is later. `None` without words.
fn clip_window(clip: &NarrationClip, words: &[AlignedWord]) -> Option<(u64, u64)> {
    let clip_end = words.last()?.end_ms.max(clip.start_ms);
    Some((clip.start_ms, clip.scene_end_ms.max(clip_end)))
}

/// Picks, for every cue without words, the clip to time it from: among the clips whose window it
/// overlaps, the one whose narration contains the largest share of the cue's text (in order),
/// then the one it overlaps longest, then the earliest. A cue imported from an SRT that starts a
/// little before a scene ends but carries the next scene's text therefore goes to the next
/// scene's clip instead of being squeezed into the first one. `None` for cues with words or
/// outside every clip.
pub(crate) fn assign_cues(
    subtitles: &[SubtitleCue],
    clips: &[NarrationClip],
    clip_words: &[Option<Vec<AlignedWord>>],
) -> Vec<Option<usize>> {
    let script_keys: Vec<Option<Vec<String>>> = clip_words
        .iter()
        .map(|w| {
            w.as_ref()
                .map(|w| w.iter().map(|x| unit_key(&x.text)).collect())
        })
        .collect();
    subtitles
        .iter()
        .map(|cue| {
            if !cue.words.is_empty() {
                return None;
            }
            let candidates: Vec<(usize, u64)> = clips
                .iter()
                .enumerate()
                .filter_map(|(k, clip)| {
                    let (start, end) = clip_window(clip, clip_words[k].as_deref()?)?;
                    let overlap = cue.end_ms.min(end).saturating_sub(cue.start_ms.max(start));
                    (overlap > 0).then_some((k, overlap))
                })
                .collect();
            if candidates.len() < 2 {
                return candidates.first().map(|&(k, _)| k);
            }
            let cue_keys: Vec<String> = split_units(&cue.text)
                .iter()
                .map(|u| unit_key(u))
                .filter(|k| !k.is_empty())
                .collect();
            candidates
                .into_iter()
                .max_by_key(|&(k, overlap)| {
                    let script = script_keys[k].as_deref().unwrap_or_default();
                    (
                        common_units(&cue_keys, script),
                        overlap,
                        std::cmp::Reverse(k),
                    )
                })
                .map(|(k, _)| k)
        })
        .collect()
}

/// How many of `a` appear in `b` in the same order (longest common subsequence).
fn common_units(a: &[String], b: &[String]) -> usize {
    let mut row = vec![0usize; b.len() + 1];
    for x in a {
        let mut diag = 0;
        for (j, y) in b.iter().enumerate() {
            let up = row[j + 1];
            row[j + 1] = if x == y { diag + 1 } else { up.max(row[j]) };
            diag = up;
        }
    }
    row[b.len()]
}

/// Puts a clip's aligned words (absolute times) into the subtitle track. Cues overlapping the
/// clip that have no words and that `claims` (see `assign_cues`) get the words of their own
/// text; with no overlapping cues at all, new cues are made from the words. Cues that already
/// carry words are left alone, and so is a cue whose words would mostly be zero-length (most of
/// its text is not in this narration): it stays plain rather than showing a line that never
/// lights up. Returns false when existing cues could not be matched to the narration.
pub(crate) fn merge_clip_words(
    subtitles: &mut Vec<SubtitleCue>,
    clip: &NarrationClip,
    words: &[AlignedWord],
    max_cols: usize,
    generate: bool,
    claims: &dyn Fn(usize) -> bool,
) -> bool {
    let Some((window_start, window_end)) = clip_window(clip, words) else {
        return true;
    };
    let overlapping: Vec<usize> = subtitles
        .iter()
        .enumerate()
        .filter(|(_, c)| c.start_ms < window_end && c.end_ms > window_start)
        .map(|(i, _)| i)
        .collect();

    if overlapping.is_empty() {
        if !generate {
            return true;
        }
        // The next cue after this clip bounds how long generated lines may stay up.
        let limit = subtitles
            .iter()
            .map(|c| c.start_ms)
            .filter(|&s| s >= clip.start_ms)
            .min()
            .unwrap_or(u64::MAX)
            .min(window_end);
        subtitles.extend(cues_from_words(words, max_cols, limit));
        return true;
    }

    let targets: Vec<usize> = overlapping
        .into_iter()
        .filter(|&i| subtitles[i].words.is_empty() && claims(i))
        .collect();
    if targets.is_empty() {
        return true;
    }
    // The cues split the narration into lines: match their units, in order, to the clip's.
    let script_keys: Vec<String> = words.iter().map(|w| unit_key(&w.text)).collect();
    let mut cue_units: Vec<Vec<String>> = Vec::new();
    for &i in &targets {
        cue_units.push(split_units(&subtitles[i].text));
    }
    let cue_keys: Vec<String> = cue_units.iter().flatten().map(|u| unit_key(u)).collect();
    let Some(mapping) = map_units(&cue_keys, &script_keys) else {
        return false;
    };

    let mut next = 0;
    for (k, &i) in targets.iter().enumerate() {
        let cue = &mut subtitles[i];
        let mut floor = cue.start_ms;
        let mut timed = Vec::with_capacity(cue_units[k].len());
        for unit in &cue_units[k] {
            let (s, e) = match mapping[next] {
                Some(j) => (words[j].start_ms, words[j].end_ms),
                None => (floor, floor),
            };
            next += 1;
            let start = s.clamp(floor, cue.end_ms);
            let end = e.clamp(start, cue.end_ms);
            floor = end;
            timed.push(WordTiming {
                text: unit.clone(),
                start_ms: start,
                end_ms: end,
            });
        }
        let zero = timed.iter().filter(|w| w.start_ms == w.end_ms).count();
        if zero * 2 > timed.len() {
            continue;
        }
        cue.words = timed;
    }
    true
}

fn unit_key(unit: &str) -> String {
    unit.chars()
        .filter(|c| c.is_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

/// Maps cue units onto narration units by exact key, in order (a monotone greedy walk with a
/// small lookahead). `None` when fewer than half of the cue units are found.
fn map_units(cue_keys: &[String], script_keys: &[String]) -> Option<Vec<Option<usize>>> {
    const LOOKAHEAD: usize = 8;
    let mut out = Vec::with_capacity(cue_keys.len());
    let mut j = 0;
    let mut found = 0;
    for key in cue_keys {
        let hit = (j..script_keys.len().min(j + LOOKAHEAD)).find(|&k| &script_keys[k] == key);
        match hit {
            Some(k) => {
                out.push(Some(k));
                found += 1;
                j = k + 1;
            }
            None => out.push(None),
        }
    }
    (found * 2 >= cue_keys.len()).then_some(out)
}

/// Display width: CJK characters count double.
fn width(text: &str) -> usize {
    text.trim()
        .chars()
        .map(|c| {
            if vimmer_media::is_cjk(c)
                || ('\u{3000}'..='\u{303F}').contains(&c)
                || ('\u{FF00}'..='\u{FFEF}').contains(&c)
            {
                2
            } else {
                1
            }
        })
        .sum()
}

fn ends_sentence(text: &str) -> bool {
    text.trim_end()
        .ends_with(['。', '！', '？', '.', '!', '?', '；', ';', '…'])
}

fn ends_clause(text: &str) -> bool {
    text.trim_end().ends_with([',', '，', '、', ':', '：'])
}

/// Groups words into subtitle lines: a sentence end always closes a line, and a line longer than
/// `max_cols` is broken at its last comma when that is past a third of the line, else before the
/// word that overflows.
pub(crate) fn line_breaks(words: &[AlignedWord], max_cols: usize) -> Vec<std::ops::Range<usize>> {
    let mut lines = Vec::new();
    let mut start = 0;
    let mut cols = 0;
    let mut last_comma: Option<usize> = None;
    let mut i = 0;
    while i < words.len() {
        let w = width(&words[i].text);
        if i > start && cols + w > max_cols {
            let cut = match last_comma {
                Some(c) if width_of(&words[start..=c]) * 3 >= max_cols => c + 1,
                _ => i,
            };
            lines.push(start..cut);
            start = cut;
            cols = width_of(&words[start..i]);
            last_comma = None;
            continue;
        }
        cols += w;
        if ends_sentence(&words[i].text) {
            lines.push(start..i + 1);
            start = i + 1;
            cols = 0;
            last_comma = None;
        } else if ends_clause(&words[i].text) {
            last_comma = Some(i);
        }
        i += 1;
    }
    if start < words.len() {
        lines.push(start..words.len());
    }
    lines
}

fn width_of(words: &[AlignedWord]) -> usize {
    words.iter().map(|w| width(&w.text)).sum()
}

/// Builds cues from aligned words (absolute times). A line shows from its first word until the
/// next line starts (when that is soon) or briefly after its last word, never past `limit`.
pub(crate) fn cues_from_words(
    words: &[AlignedWord],
    max_cols: usize,
    limit: u64,
) -> Vec<SubtitleCue> {
    let lines = line_breaks(words, max_cols);
    let mut cues = Vec::new();
    for (n, range) in lines.iter().enumerate() {
        let line = &words[range.clone()];
        let start = line[0].start_ms;
        let last_end = line.last().expect("lines are non-empty").end_ms;
        let next_start = lines.get(n + 1).map(|r| words[r.start].start_ms);
        let end = match next_start {
            Some(next) if next <= last_end + LINE_JOIN_MS => next,
            _ => last_end + LINE_HOLD_MS,
        }
        .min(limit)
        .max(last_end.min(limit));
        if end <= start {
            continue;
        }
        let count = line.len();
        let words: Vec<WordTiming> = line
            .iter()
            .enumerate()
            .map(|(k, w)| {
                let mut text = w.text.clone();
                if k == 0 {
                    text = text.trim_start().to_string();
                }
                if k + 1 == count {
                    text = text.trim_end().to_string();
                }
                WordTiming {
                    text,
                    start_ms: w.start_ms.clamp(start, end),
                    end_ms: w.end_ms.clamp(start, end),
                }
            })
            .collect();
        cues.push(SubtitleCue {
            start_ms: start,
            end_ms: end,
            text: words.iter().map(|w| w.text.as_str()).collect(),
            words,
        });
    }
    cues
}

#[cfg(test)]
mod tests {
    use std::path::Path;
    use std::sync::Mutex;

    use vimmer_dsl::{validate, AudioMapping, MotionType, Narration, Resolution, Scene};
    use vimmer_media::MediaError;

    use super::*;

    fn w(text: &str, start_ms: u64, end_ms: u64) -> AlignedWord {
        AlignedWord {
            text: text.into(),
            start_ms,
            end_ms,
        }
    }

    fn clip(start_ms: u64, scene_end_ms: u64) -> NarrationClip {
        NarrationClip {
            scene_index: 0,
            text: String::new(),
            wav: PathBuf::from("x.wav"),
            start_ms,
            scene_end_ms,
            on_screen: false,
        }
    }

    /// The old `--auto-subtitles on`: cues for every narrated scene.
    fn auto() -> WordSubtitleConfig {
        WordSubtitleConfig {
            auto_subtitles: AutoSubtitles::All,
            ..WordSubtitleConfig::default()
        }
    }

    #[test]
    fn sentences_and_long_lines_are_broken() {
        let words = vec![
            w("Hello ", 0, 100),
            w("there. ", 100, 200),
            w("This ", 300, 400),
            w("line, ", 400, 500),
            w("is ", 500, 600),
            w("long", 600, 700),
        ];
        assert_eq!(line_breaks(&words, 80), vec![0..2, 2..6]);
        // 11 columns: "This line, is" fits, adding "long" overflows, so the line is cut at
        // the comma and "is long" moves on.
        assert_eq!(line_breaks(&words, 11), vec![0..2, 2..4, 4..6]);
    }

    #[test]
    fn cjk_counts_double_and_breaks_at_commas() {
        let text = "我们在西安，看钟楼和鼓楼。";
        let units = split_units(text);
        let words: Vec<AlignedWord> = units
            .iter()
            .enumerate()
            .map(|(i, u)| w(u, i as u64 * 100, i as u64 * 100 + 100))
            .collect();
        // 11 characters = 22 columns. With 16 columns the line breaks after the comma.
        let lines = line_breaks(&words, 16);
        assert_eq!(lines, vec![0..5, 5..11]);
    }

    #[test]
    fn generated_cues_hold_join_and_respect_the_limit() {
        let words = vec![
            w(" One. ", 0, 400),
            w("Two. ", 900, 1300),
            w("Three.", 4000, 4500),
        ];
        let cues = cues_from_words(&words, 42, 4800);
        assert_eq!(cues.len(), 3);
        assert_eq!(
            (cues[0].start_ms, cues[0].end_ms),
            (0, 900),
            "joined to the next line"
        );
        assert_eq!(
            (cues[1].start_ms, cues[1].end_ms),
            (900, 1900),
            "held 600 ms"
        );
        assert_eq!(
            (cues[2].start_ms, cues[2].end_ms),
            (4000, 4800),
            "cut at the limit"
        );
        assert_eq!(cues[0].text, "One.");
        assert_eq!(cues[0].words[0].text, "One.");
        assert_eq!(cues[2].words[0].end_ms, 4500);
    }

    #[test]
    fn fills_words_into_existing_cues_and_clamps_to_them() {
        let mut subs = vec![
            SubtitleCue::new(0, 1000, "Hello world"),
            SubtitleCue::new(1000, 3000, "again"),
            SubtitleCue::new(5000, 6000, "elsewhere"),
        ];
        let words = vec![
            w("Hello ", 100, 400),
            w("world ", 400, 1100),
            w("again", 1200, 1500),
        ];
        assert!(merge_clip_words(
            &mut subs,
            &clip(100, 3000),
            &words,
            42,
            true,
            &|_| true
        ));
        assert_eq!(subs.len(), 3);
        let spans: Vec<(u64, u64)> = subs[0]
            .words
            .iter()
            .map(|x| (x.start_ms, x.end_ms))
            .collect();
        assert_eq!(
            spans,
            vec![(100, 400), (400, 1000)],
            "clamped to the cue end"
        );
        assert_eq!(subs[1].words[0].start_ms, 1200);
        assert!(subs[2].words.is_empty(), "cue outside the clip untouched");
    }

    /// Two narrated scenes, 0-3 s and 3-6 s, and an imported cue that starts 300 ms before the
    /// first scene ends but carries the second scene's text.
    fn cross_scene_setup() -> (Vec<SubtitleCue>, Vec<NarrationClip>, Vec<Vec<AlignedWord>>) {
        let subs = vec![
            SubtitleCue::new(0, 2600, "Hello big world."),
            SubtitleCue::new(2700, 5500, "Bye now friends."),
        ];
        let mut b = clip(3000, 6000);
        b.scene_index = 1;
        let clips = vec![clip(0, 3000), b];
        let words = vec![
            vec![
                w("Hello ", 100, 500),
                w("big ", 500, 900),
                w("world.", 900, 2000),
            ],
            vec![
                w("Bye ", 3100, 3500),
                w("now ", 3500, 4000),
                w("friends.", 4000, 5000),
            ],
        ];
        (subs, clips, words)
    }

    fn spans(cue: &SubtitleCue) -> Vec<(u64, u64)> {
        cue.words.iter().map(|x| (x.start_ms, x.end_ms)).collect()
    }

    #[test]
    fn a_cue_crossing_into_the_next_scene_is_timed_from_the_narration_it_matches() {
        let (mut subs, clips, words) = cross_scene_setup();
        let clip_words: Vec<Option<Vec<AlignedWord>>> = words.iter().cloned().map(Some).collect();
        let owners = assign_cues(&subs, &clips, &clip_words);
        assert_eq!(
            owners,
            vec![Some(0), Some(1)],
            "assigned by text, not by first overlap"
        );
        for (k, clip) in clips.iter().enumerate() {
            assert!(merge_clip_words(
                &mut subs,
                clip,
                &words[k],
                42,
                false,
                &|i| owners[i] == Some(k)
            ));
        }
        assert_eq!(spans(&subs[0]), vec![(100, 500), (500, 900), (900, 2000)]);
        assert_eq!(
            spans(&subs[1]),
            vec![(3100, 3500), (3500, 4000), (4000, 5000)],
            "real timing from the second scene, not zero-length words at the cue start"
        );
    }

    #[test]
    fn mostly_zero_length_words_are_not_written_and_a_later_clip_fills_them() {
        // Without the assignment (every overlapping cue claimed), the first clip still matches
        // more than half of all cue text, which used to stamp the second cue with zero-length
        // words at 2700 ms and stop the second clip from fixing it.
        let (mut subs, clips, words) = cross_scene_setup();
        assert!(merge_clip_words(
            &mut subs,
            &clips[0],
            &words[0],
            42,
            false,
            &|_| true
        ));
        assert_eq!(spans(&subs[0]), vec![(100, 500), (500, 900), (900, 2000)]);
        assert!(subs[1].words.is_empty(), "left for a later clip");
        assert!(merge_clip_words(
            &mut subs,
            &clips[1],
            &words[1],
            42,
            false,
            &|_| true
        ));
        assert_eq!(
            spans(&subs[1]),
            vec![(3100, 3500), (3500, 4000), (4000, 5000)]
        );
    }

    #[test]
    fn a_cue_inside_one_clip_still_goes_to_that_clip() {
        let (subs, clips, words) = cross_scene_setup();
        // Only the first clip was recognized: the second cue has no other candidate.
        let clip_words = vec![Some(words[0].clone()), None];
        assert_eq!(
            assign_cues(&subs, &clips, &clip_words),
            vec![Some(0), Some(0)]
        );
        // A cue that already has words is never reassigned.
        let mut filled = subs.clone();
        filled[0].words = vec![WordTiming {
            text: "Hello big world.".into(),
            start_ms: 0,
            end_ms: 2000,
        }];
        let clip_words: Vec<Option<Vec<AlignedWord>>> = words.into_iter().map(Some).collect();
        assert_eq!(
            assign_cues(&filled, &clips, &clip_words),
            vec![None, Some(1)]
        );
    }

    #[test]
    fn words_that_no_longer_spell_the_text_are_dropped() {
        let word = |text: &str, start_ms: u64, end_ms: u64| WordTiming {
            text: text.into(),
            start_ms,
            end_ms,
        };
        let mut a = SubtitleCue::new(0, 1000, "Hello there");
        a.words = vec![word("Hello ", 0, 400), word("world", 400, 900)];
        let mut b = SubtitleCue::new(1000, 2000, " Hello  world ");
        b.words = vec![word("Hello ", 1000, 1400), word("world", 1400, 1900)];
        let mut subs = vec![a, b.clone()];
        let mut warnings = vec![];
        drop_stale_words(&mut subs, &mut warnings);
        assert!(subs[0].words.is_empty(), "edited text: timed again");
        assert_eq!(subs[1], b, "whitespace-only difference keeps the words");
        assert_eq!(warnings.len(), 1);
        assert!(warnings[0].starts_with("subtitle 1:"), "{warnings:?}");
        assert!(words_match_text(
            "你好",
            &[word("你", 0, 1), word("好", 1, 2)]
        ));
        assert!(!words_match_text(
            "你好！",
            &[word("你", 0, 1), word("好", 1, 2)]
        ));
    }

    #[test]
    fn unrelated_existing_cues_are_reported_and_left_alone() {
        let mut subs = vec![SubtitleCue::new(0, 1000, "完全不同")];
        let words = vec![w("Hello ", 100, 400), w("world", 400, 900)];
        assert!(!merge_clip_words(
            &mut subs,
            &clip(0, 1000),
            &words,
            42,
            true,
            &|_| true
        ));
        assert!(subs[0].words.is_empty());
    }

    #[test]
    fn generates_cues_when_none_cover_the_clip() {
        let mut subs = vec![SubtitleCue::new(9000, 9500, "later")];
        let words = vec![w("你", 1000, 1200), w("好。", 1200, 1500)];
        assert!(merge_clip_words(
            &mut subs,
            &clip(1000, 3000),
            &words,
            42,
            true,
            &|_| true
        ));
        assert_eq!(subs.len(), 2);
        assert_eq!(subs[1].text, "你好。");
        assert_eq!((subs[1].start_ms, subs[1].end_ms), (1000, 2100));
    }

    #[test]
    fn batches_group_by_language_up_to_one_window() {
        let l = |s: &str| Some(s.to_string());
        let langs = vec![l("zh"), l("en"), l("zh"), l("zh"), None, l("zh")];
        let ms = vec![5_000, 4_000, 6_000, 20_000, 3_000, 30_000];
        let batches = plan_batches(&langs, &ms);
        assert_eq!(
            batches,
            vec![
                Batch {
                    language: l("zh"),
                    members: vec![(0, 0, 5_000), (2, 6_000, 6_000)]
                },
                Batch {
                    language: l("en"),
                    members: vec![(1, 0, 4_000)]
                },
                Batch {
                    language: l("zh"),
                    members: vec![(3, 0, 20_000)]
                },
                Batch {
                    language: None,
                    members: vec![(4, 0, 3_000)]
                },
                Batch {
                    language: l("zh"),
                    members: vec![(5, 0, 30_000)]
                },
            ]
        );
    }

    #[test]
    fn tokens_of_a_joined_batch_go_back_to_their_clips() {
        let members = vec![(0, 0, 2_000), (1, 3_000, 1_500), (2, 5_500, 1_000)];
        let tokens = vec![
            RecognizedToken::new("a", 100, 900),
            RecognizedToken::new("b", 1_800, 2_300), // runs a little into the gap
            RecognizedToken::new("c", 2_900, 3_400), // starts a little early
            RecognizedToken::new("d", 5_600, 6_400),
        ];
        let split = split_tokens(&tokens, &members);
        assert_eq!(
            split[0],
            vec![
                RecognizedToken::new("a", 100, 900),
                RecognizedToken::new("b", 1_800, 2_000)
            ]
        );
        assert_eq!(split[1], vec![RecognizedToken::new("c", 0, 400)]);
        assert_eq!(split[2], vec![RecognizedToken::new("d", 100, 900)]);
    }

    struct FixedTranscriber(Vec<RecognizedToken>, Mutex<Vec<TranscribeRequest>>);
    impl Transcriber for FixedTranscriber {
        fn transcribe(
            &self,
            _wav: &Path,
            request: &TranscribeRequest,
            _limits: &Limits,
        ) -> Result<Vec<RecognizedToken>, MediaError> {
            self.1.lock().unwrap().push(request.clone());
            Ok(self.0.clone())
        }
    }

    struct FixedProbe(u64);
    impl AudioProbe for FixedProbe {
        fn duration_ms(&self, _path: &Path) -> Result<u64, MediaError> {
            Ok(self.0)
        }
    }

    fn narrated(id: &str, ms: u64, text: &str) -> Scene {
        Scene {
            sfx: vec![],
            template: None,
            id: id.into(),
            duration_ms: ms,
            text: Some(text.into()),
            highlights: vec![],
            image: None,
            motion: MotionType::Static,
            video: None,
            audio: Some(AudioMapping {
                source: format!("generated/tts/{id}.wav"),
                offset_ms: 0,
            }),
            narration: Some(Narration::default()),
            transition_in: None,
            keyframes: vec![],
            layers: vec![],
        }
    }

    #[tokio::test]
    async fn end_to_end_with_a_fixed_recognizer() {
        let mut t = VideoTimeline {
            brand: None,
            fps: 30,
            resolution: Resolution {
                width: 1920,
                height: 1080,
            },
            scenes: vec![
                narrated("intro", 2000, "Hello world."),
                narrated("zh", 2000, "你好世界。"),
            ],
            subtitles: vec![],
            music: None,
        };
        t.scenes[0].audio = None; // not narrated by us: ignored
        let cfg = SchedulerConfig::new(
            PathBuf::from("w"),
            PathBuf::from("o.mp4"),
            PathBuf::from("r"),
        );
        let clips = narration_clips(&t, &cfg);
        assert_eq!(clips.len(), 1);
        assert_eq!(clips[0].start_ms, 2000);

        let engine = Arc::new(FixedTranscriber(
            vec![
                RecognizedToken::new("你好", 100, 500),
                RecognizedToken::new("世界", 500, 1200),
            ],
            Mutex::new(vec![]),
        ));
        let mut warnings = vec![];
        let n = apply_word_subtitles(
            &mut t,
            &clips,
            engine.clone(),
            Arc::new(FixedProbe(1500)),
            &auto(),
            &RenderContext::silent(),
            &mut warnings,
        )
        .await
        .unwrap();
        assert_eq!(n, 1);
        assert!(warnings.is_empty());
        let requests = engine.1.lock().unwrap().clone();
        assert_eq!(requests[0].language.as_deref(), Some("zh"));
        assert_eq!(t.subtitles.len(), 1);
        let cue = &t.subtitles[0];
        assert_eq!(cue.text, "你好世界。");
        assert_eq!(cue.start_ms, 2100);
        let spans: Vec<(u64, u64)> = cue.words.iter().map(|x| (x.start_ms, x.end_ms)).collect();
        assert_eq!(
            spans,
            vec![(2100, 2300), (2300, 2500), (2500, 2850), (2850, 3200)]
        );
        assert_eq!(validate(&t), Ok(()));
    }

    #[tokio::test]
    async fn a_cue_edited_after_timing_is_timed_again() {
        let mut t = VideoTimeline {
            brand: None,
            fps: 30,
            resolution: Resolution {
                width: 1920,
                height: 1080,
            },
            scenes: vec![narrated("zh", 2000, "你好世界。")],
            subtitles: vec![SubtitleCue::new(0, 2000, "你好世界。")],
            music: None,
        };
        // Words from an earlier text, and times that no longer fit the edited cue.
        t.subtitles[0].words = ["再", "见", "。"]
            .iter()
            .enumerate()
            .map(|(i, c)| WordTiming {
                text: (*c).into(),
                start_ms: i as u64 * 100,
                end_ms: i as u64 * 100 + 100,
            })
            .collect();
        let cfg = SchedulerConfig::new(
            PathBuf::from("w"),
            PathBuf::from("o.mp4"),
            PathBuf::from("r"),
        );
        let mut warnings = vec![];
        drop_stale_words(&mut t.subtitles, &mut warnings);
        let clips = clips_to_time(&t.subtitles, narration_clips(&t, &cfg), AutoSubtitles::Off);
        assert_eq!(clips.len(), 1, "the edited cue needs timing again");
        let engine = Arc::new(FixedTranscriber(
            vec![
                RecognizedToken::new("你好", 100, 500),
                RecognizedToken::new("世界", 500, 1200),
            ],
            Mutex::new(vec![]),
        ));
        let n = apply_word_subtitles(
            &mut t,
            &clips,
            engine,
            Arc::new(FixedProbe(1500)),
            &WordSubtitleConfig::default(),
            &RenderContext::silent(),
            &mut warnings,
        )
        .await
        .unwrap();
        assert_eq!(n, 1);
        let cue = &t.subtitles[0];
        assert!(words_match_text(&cue.text, &cue.words));
        assert_eq!(
            spans(cue),
            vec![(100, 300), (300, 500), (500, 850), (850, 1200)]
        );
        assert_eq!(validate(&t), Ok(()));
    }

    #[tokio::test]
    async fn missing_engine_warns_and_changes_nothing() {
        let mut t = VideoTimeline {
            brand: None,
            fps: 30,
            resolution: Resolution {
                width: 1920,
                height: 1080,
            },
            scenes: vec![narrated("a", 2000, "Hello")],
            subtitles: vec![SubtitleCue::new(0, 1000, "Hello")],
            music: None,
        };
        let before = t.clone();
        let mut cfg = SchedulerConfig::new(
            PathBuf::from("w"),
            PathBuf::from("o.mp4"),
            PathBuf::from("r"),
        );
        cfg.child_path = Some("/nonexistent-dir".into());
        cfg.word_subtitles.whisper_cli = Some(PathBuf::from("whisper-cli-missing"));
        let mut warnings = vec![];
        let n = add_word_subtitles(
            &mut t,
            &cfg,
            Arc::new(FixedProbe(1000)),
            &RenderContext::silent(),
            &mut warnings,
            &mut vec![],
        )
        .await
        .unwrap();
        assert_eq!(n, 0);
        assert_eq!(t, before);
        assert_eq!(warnings.len(), 1);
        assert!(
            warnings[0].contains("word-by-word subtitles skipped"),
            "{warnings:?}"
        );

        cfg.word_subtitles.enabled = false;
        let mut warnings = vec![];
        add_word_subtitles(
            &mut t,
            &cfg,
            Arc::new(FixedProbe(1000)),
            &RenderContext::silent(),
            &mut warnings,
            &mut vec![],
        )
        .await
        .unwrap();
        assert!(warnings.is_empty(), "disabled: not even a warning");
    }

    #[test]
    fn clip_end_never_passes_the_validated_timeline_length() {
        // 4920 ms and 4650 ms round up to 148 and 140 frames: frame time 9600 ms, sum 9570 ms.
        let t = VideoTimeline {
            brand: None,
            fps: 30,
            resolution: Resolution {
                width: 1920,
                height: 1080,
            },
            scenes: vec![
                narrated("zh", 4920, "你好。"),
                narrated("ja", 4650, "こんにちは。"),
            ],
            subtitles: vec![],
            music: None,
        };
        let cfg = SchedulerConfig::new(
            PathBuf::from("w"),
            PathBuf::from("o.mp4"),
            PathBuf::from("r"),
        );
        let clips = narration_clips(&t, &cfg);
        assert_eq!(clips[1].scene_end_ms, 9_570);
        assert_eq!(t.total_duration_ms(), 9_570);
        let mut subs = vec![];
        let words = vec![w("こん", 4_950, 6_000), w("にちは。", 6_000, 9_500)];
        assert!(merge_clip_words(
            &mut subs,
            &clips[1],
            &words,
            42,
            true,
            &|_| true
        ));
        assert_eq!(
            subs[0].end_ms, 9_570,
            "held line is cut at the validated end"
        );
    }

    struct JoiningTranscriber {
        joined: Mutex<Vec<(Vec<PathBuf>, u64)>>,
        calls: Mutex<Vec<PathBuf>>,
    }
    impl Transcriber for JoiningTranscriber {
        fn transcribe(
            &self,
            wav: &Path,
            _request: &TranscribeRequest,
            _limits: &Limits,
        ) -> Result<Vec<RecognizedToken>, MediaError> {
            self.calls.lock().unwrap().push(wav.to_path_buf());
            // Joined audio: clip 1 (1000 ms) at 0, gap 1000 ms, clip 2 at 2000.
            Ok(vec![
                RecognizedToken::new("你好", 0, 1000),
                RecognizedToken::new("再见", 2000, 2800),
            ])
        }
        fn join_clips(
            &self,
            inputs: &[PathBuf],
            gap_ms: u64,
            out: &Path,
            _limits: &Limits,
        ) -> Result<(), MediaError> {
            self.joined.lock().unwrap().push((inputs.to_vec(), gap_ms));
            std::fs::write(out, b"joined").map_err(MediaError::from)
        }
    }

    #[tokio::test]
    async fn same_language_clips_are_recognized_in_one_call() {
        let dir = std::env::temp_dir().join(format!("vimmer-batch-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let mut t = VideoTimeline {
            brand: None,
            fps: 30,
            resolution: Resolution {
                width: 1920,
                height: 1080,
            },
            scenes: vec![narrated("a", 2000, "你好"), narrated("b", 2000, "再见")],
            subtitles: vec![],
            music: None,
        };
        let mut clips = narration_clips(
            &t,
            &SchedulerConfig::new(dir.clone(), dir.join("o.mp4"), dir.clone()),
        );
        for clip in &mut clips {
            clip.wav = dir.join(clip.wav.file_name().unwrap());
        }
        let engine = Arc::new(JoiningTranscriber {
            joined: Mutex::new(vec![]),
            calls: Mutex::new(vec![]),
        });
        let mut warnings = vec![];
        let n = apply_word_subtitles(
            &mut t,
            &clips,
            engine.clone(),
            Arc::new(FixedProbe(1000)),
            &auto(),
            &RenderContext::silent(),
            &mut warnings,
        )
        .await
        .unwrap();
        assert_eq!(n, 2, "{warnings:?}");
        assert_eq!(
            engine.calls.lock().unwrap().len(),
            1,
            "one whisper call for both clips"
        );
        assert_eq!(engine.joined.lock().unwrap()[0].1, BATCH_GAP_MS);
        let joined_wav = engine.calls.lock().unwrap()[0].clone();
        assert!(
            !joined_wav.exists(),
            "the joined batch file is removed after recognition"
        );
        let spans: Vec<Vec<(u64, u64)>> = t
            .subtitles
            .iter()
            .map(|c| c.words.iter().map(|w| (w.start_ms, w.end_ms)).collect())
            .collect();
        assert_eq!(
            spans,
            vec![
                vec![(0, 500), (500, 1000)],
                vec![(2000, 2400), (2400, 2800)]
            ]
        );
        assert_eq!(validate(&t), Ok(()));
        let _ = std::fs::remove_dir_all(dir);
    }

    /// Fails every call with the given error and counts the calls.
    struct FailingTranscriber(fn() -> MediaError, Mutex<usize>);
    impl Transcriber for FailingTranscriber {
        fn transcribe(
            &self,
            _wav: &Path,
            _request: &TranscribeRequest,
            limits: &Limits,
        ) -> Result<Vec<RecognizedToken>, MediaError> {
            *self.1.lock().unwrap() += 1;
            assert!(limits.timeout.is_some(), "every call has a time budget");
            Err((self.0)())
        }
    }

    fn three_languages() -> (VideoTimeline, Vec<NarrationClip>) {
        let t = VideoTimeline {
            brand: None,
            fps: 30,
            resolution: Resolution {
                width: 1920,
                height: 1080,
            },
            scenes: vec![
                narrated("en", 2000, "Hello."),
                narrated("zh", 2000, "你好。"),
                narrated("ja", 2000, "こんにちは。"),
            ],
            subtitles: vec![SubtitleCue::new(0, 1000, "Hello.")],
            music: None,
        };
        let cfg = SchedulerConfig::new(
            PathBuf::from("w"),
            PathBuf::from("o.mp4"),
            PathBuf::from("r"),
        );
        let clips = narration_clips(&t, &cfg);
        (t, clips)
    }

    #[tokio::test]
    async fn a_timeout_falls_back_and_skips_the_remaining_batches() {
        let (mut t, clips) = three_languages();
        let before = t.subtitles.clone();
        let engine = Arc::new(FailingTranscriber(
            || MediaError::TimedOut { secs: 1 },
            Mutex::new(0),
        ));
        let mut warnings = vec![];
        let n = apply_word_subtitles(
            &mut t,
            &clips,
            engine.clone(),
            Arc::new(FixedProbe(1000)),
            &auto(),
            &RenderContext::silent(),
            &mut warnings,
        )
        .await
        .unwrap();
        assert_eq!(n, 0);
        assert_eq!(
            *engine.1.lock().unwrap(),
            1,
            "no second call after a timeout"
        );
        assert_eq!(warnings.len(), 3, "{warnings:?}");
        assert!(warnings[0].contains("timed out"), "{warnings:?}");
        assert_eq!(t.subtitles, before, "plain subtitles kept");
    }

    #[tokio::test]
    async fn a_cancel_during_recognition_cancels_the_render() {
        let (mut t, clips) = three_languages();
        let engine = Arc::new(FailingTranscriber(|| MediaError::Cancelled, Mutex::new(0)));
        let ctx = RenderContext::silent();
        let result = apply_word_subtitles(
            &mut t,
            &clips,
            engine.clone(),
            Arc::new(FixedProbe(1000)),
            &auto(),
            &ctx,
            &mut vec![],
        )
        .await;
        assert!(matches!(result, Err(SchedulerError::Cancelled)));
        assert_eq!(*engine.1.lock().unwrap(), 1);
    }

    #[test]
    fn timeout_grows_with_the_audio() {
        assert_eq!(recognition_timeout(0), Duration::from_secs(180));
        assert_eq!(recognition_timeout(28_000), Duration::from_secs(180 + 840));
    }

    #[test]
    fn by_default_only_clips_with_cues_are_timed() {
        let (t, clips) = three_languages();
        // Only the English scene has a cue.
        let kept = clips_to_time(&t.subtitles, clips.clone(), AutoSubtitles::Off);
        assert_eq!(kept.len(), 1);
        assert_eq!(t.scenes[kept[0].scene_index].id, "en");
        assert_eq!(
            clips_to_time(&t.subtitles, clips.clone(), AutoSubtitles::All).len(),
            3
        );
        // A cue that already has words needs nothing.
        let mut filled = t.subtitles.clone();
        filled[0].words = vec![WordTiming {
            text: "Hello.".into(),
            start_ms: 0,
            end_ms: 500,
        }];
        assert!(clips_to_time(&filled, clips, AutoSubtitles::Off).is_empty());
    }

    #[test]
    fn by_default_no_cue_is_generated_for_narration_without_cues() {
        let mut subs = vec![SubtitleCue::new(9000, 9500, "later")];
        let words = vec![w("你", 1000, 1200), w("好。", 1200, 1500)];
        assert!(merge_clip_words(
            &mut subs,
            &clip(1000, 3000),
            &words,
            42,
            false,
            &|_| true
        ));
        assert_eq!(subs, vec![SubtitleCue::new(9000, 9500, "later")]);
        assert!(merge_clip_words(
            &mut subs,
            &clip(1000, 3000),
            &words,
            42,
            true,
            &|_| true
        ));
        assert_eq!(subs.len(), 2, "generated when switched on");
    }

    #[tokio::test]
    async fn default_config_leaves_a_timeline_without_cues_untouched() {
        let mut t = VideoTimeline {
            brand: None,
            fps: 30,
            resolution: Resolution {
                width: 1920,
                height: 1080,
            },
            scenes: vec![narrated("zh", 2000, "你好。")],
            subtitles: vec![],
            music: None,
        };
        let before = t.clone();
        let mut cfg = SchedulerConfig::new(
            PathBuf::from("w"),
            PathBuf::from("o.mp4"),
            PathBuf::from("r"),
        );
        // Point at a missing engine: with nothing to time, it must not even be looked up.
        cfg.word_subtitles.whisper_cli = Some(PathBuf::from("/nonexistent/whisper-cli"));
        let mut warnings = vec![];
        let n = add_word_subtitles(
            &mut t,
            &cfg,
            Arc::new(FixedProbe(1000)),
            &RenderContext::silent(),
            &mut warnings,
            &mut vec![],
        )
        .await
        .unwrap();
        assert_eq!(n, 0);
        assert_eq!(t, before);
        assert!(warnings.is_empty(), "{warnings:?}");
        // Switched on, the same timeline would need the engine (and warns that it is missing).
        cfg.word_subtitles.auto_subtitles = AutoSubtitles::All;
        add_word_subtitles(
            &mut t,
            &cfg,
            Arc::new(FixedProbe(1000)),
            &RenderContext::silent(),
            &mut warnings,
            &mut vec![],
        )
        .await
        .unwrap();
        assert_eq!(warnings.len(), 1, "{warnings:?}");
    }

    fn layer(json: &str) -> vimmer_dsl::Layer {
        serde_json::from_str(json).unwrap()
    }

    fn shown(text: &str) -> Scene {
        narrated("s", 4000, text)
    }

    #[test]
    fn visible_key_ignores_case_punctuation_spacing_and_width() {
        assert_eq!(visible_key("Hello, World!"), "helloworld");
        assert_eq!(visible_key("Ｈｅｌｌｏ，　Ｗｏｒｌｄ！"), "helloworld");
        assert_eq!(visible_key("  hello\nworld... "), "helloworld");
        assert_eq!(visible_key("你好，世界。"), visible_key("你好 世界!"));
        assert_eq!(visible_key("「ＡＩ」で変わる、未来。"), "aiで変わる未来");
        assert_eq!(visible_key("Tip 1: don't"), "tip1dont");
    }

    #[test]
    fn narration_matching_the_scene_text_is_on_screen() {
        // English: punctuation, case and spacing differ.
        let s = shown("Tip one: grind your beans right before brewing.");
        assert!(narration_on_screen(
            &s,
            "tip one — Grind your beans  right before brewing",
            0
        ));
        // Chinese: full-width punctuation on screen, half-width in the narration.
        let s = shown("在西安的钟楼下，每一个字都会亮起来。");
        assert!(narration_on_screen(
            &s,
            "在西安的钟楼下,每一个字都会亮起来.",
            0
        ));
        // Japanese with full-width Latin letters on screen.
        let s = shown("ＡＩで、未来が変わる！");
        assert!(narration_on_screen(&s, "AIで未来が変わる。", 0));
        // The narration is a part of a longer on-screen text: every spoken word is shown.
        let s = shown("Small changes, big flavor. Try it tomorrow.");
        assert!(narration_on_screen(&s, "Small changes, big flavor.", 0));
    }

    #[test]
    fn narration_longer_than_the_on_screen_text_is_not() {
        let s = shown("Grind fresh");
        assert!(!narration_on_screen(
            &s,
            "Tip one: grind your beans right before brewing.",
            0
        ));
        let s = shown("西安钟楼");
        assert!(!narration_on_screen(&s, "西安钟楼，古城的中心。", 0));
        let s = shown("桜");
        assert!(!narration_on_screen(&s, "桜の季節がやってきました。", 0));
        // Same words in another order are not the same sentence.
        let s = shown("big flavor, small changes");
        assert!(!narration_on_screen(&s, "Small changes, big flavor.", 0));
        // A different word.
        let s = shown("Use water just off the boil.");
        assert!(!narration_on_screen(
            &s,
            "Use water just below the boil.",
            0
        ));
    }

    #[test]
    fn text_layers_on_screen_during_the_narration_count() {
        let mut s = shown("Coffee");
        s.layers = vec![layer(
            r#"{"id":"t","kind":"text","x":0,"y":0,"width":1,"text":"Grind your beans right before brewing!"}"#,
        )];
        assert!(narration_on_screen(
            &s,
            "Grind your beans right before brewing.",
            0
        ));
        // Split between a layer under the scene text and the scene text itself, in drawing
        // order.
        let mut s = shown("right before brewing.");
        s.layers = vec![layer(
            r#"{"id":"t","kind":"text","x":0,"y":0,"width":1,"z":-1,"text":"Grind your beans"}"#,
        )];
        assert!(narration_on_screen(
            &s,
            "Grind your beans right before brewing.",
            0
        ));
        // A lower third's title and subtitle.
        let mut s = shown("Coffee");
        s.layers = vec![layer(
            r#"{"id":"l","kind":"lower_third","x":0,"y":0.7,"width":0.6,"height":0.15,"title":"Ada Lovelace","subtitle":"First programmer"}"#,
        )];
        assert!(narration_on_screen(
            &s,
            "Ada Lovelace, first programmer.",
            0
        ));
        // A layer that is gone before the narration starts, or invisible, does not count.
        let mut s = shown("Coffee");
        s.layers = vec![
            layer(
                r#"{"id":"a","kind":"text","x":0,"y":0,"width":1,"end_ms":500,"text":"Grind your beans."}"#,
            ),
            layer(
                r#"{"id":"b","kind":"text","x":0,"y":0,"width":1,"opacity":0,"text":"Grind your beans."}"#,
            ),
        ];
        assert!(!narration_on_screen(&s, "Grind your beans.", 800));
        assert!(narration_on_screen(&s, "Grind your beans.", 200));
    }

    #[test]
    fn auto_subtitles_on_skips_only_narration_already_on_screen() {
        let (t, mut clips) = three_languages();
        // Every narration in these scenes is their scene text.
        assert!(clips.iter().all(|c| c.on_screen));
        // `on`: the English clip still has a cue to fill; the others are on screen.
        let kept = clips_to_time(&t.subtitles, clips.clone(), AutoSubtitles::On);
        assert_eq!(
            kept,
            clips_to_time(&t.subtitles, clips.clone(), AutoSubtitles::Off)
        );
        // Narration that is not on screen is still subtitled.
        clips[2].on_screen = false;
        let kept = clips_to_time(&t.subtitles, clips.clone(), AutoSubtitles::On);
        let ids: Vec<&str> = kept
            .iter()
            .map(|c| t.scenes[c.scene_index].id.as_str())
            .collect();
        assert_eq!(ids, ["en", "ja"]);
        assert!(!AutoSubtitles::On.generates(&clips[1]));
        assert!(AutoSubtitles::On.generates(&clips[2]));
        assert!(AutoSubtitles::All.generates(&clips[1]));
        assert!(!AutoSubtitles::Off.generates(&clips[2]));
    }

    async fn run_auto(
        mode: AutoSubtitles,
        t: &mut VideoTimeline,
        tokens: Vec<RecognizedToken>,
    ) -> Vec<String> {
        let cfg = SchedulerConfig::new(
            PathBuf::from("w"),
            PathBuf::from("o.mp4"),
            PathBuf::from("r"),
        );
        let clips = clips_to_time(&t.subtitles, narration_clips(t, &cfg), mode);
        let engine = Arc::new(FixedTranscriber(tokens, Mutex::new(vec![])));
        let ws = WordSubtitleConfig {
            auto_subtitles: mode,
            ..WordSubtitleConfig::default()
        };
        let mut warnings = vec![];
        apply_word_subtitles(
            t,
            &clips,
            engine,
            Arc::new(FixedProbe(1500)),
            &ws,
            &RenderContext::silent(),
            &mut warnings,
        )
        .await
        .unwrap();
        warnings
    }

    /// Two narrated scenes in different languages (one recognition call each).
    fn two_scenes() -> VideoTimeline {
        VideoTimeline {
            brand: None,
            fps: 30,
            resolution: Resolution {
                width: 1920,
                height: 1080,
            },
            scenes: vec![
                narrated("one", 2000, "Hello world."),
                narrated("two", 2000, "你好世界。"),
            ],
            subtitles: vec![],
            music: None,
        }
    }

    #[tokio::test]
    async fn on_screen_narration_gets_no_cue_but_hand_written_cues_keep_their_words() {
        let tokens = vec![
            RecognizedToken::new("Hello", 100, 500),
            RecognizedToken::new("world.", 500, 1200),
        ];
        // `all` makes a cue for each narrated scene (the old `on`).
        let mut all = two_scenes();
        run_auto(AutoSubtitles::All, &mut all, tokens.clone()).await;
        assert_eq!(all.subtitles.len(), 2);

        // `on`: both narrations are on screen, so nothing is generated.
        let mut on = two_scenes();
        let warnings = run_auto(AutoSubtitles::On, &mut on, tokens.clone()).await;
        assert!(on.subtitles.is_empty(), "{:?}", on.subtitles);
        assert!(warnings.is_empty(), "{warnings:?}");

        // A hand-written cue over an on-screen narration stays and is word-timed exactly as with
        // `all` or `off`.
        let with_cue = |mut t: VideoTimeline| {
            t.subtitles = vec![SubtitleCue::new(2000, 3900, "你好世界。")];
            t
        };
        let mut timed = Vec::new();
        for mode in [AutoSubtitles::Off, AutoSubtitles::On, AutoSubtitles::All] {
            let mut t = with_cue(two_scenes());
            run_auto(mode, &mut t, tokens.clone()).await;
            timed.push(t.subtitles);
        }
        assert_eq!(timed[0].len(), 1);
        assert!(!timed[0][0].words.is_empty());
        assert_eq!(
            timed[0], timed[1],
            "`on` times hand-written cues like `off`"
        );
        // `all` also generates the first scene's cue; the hand-written one is identical.
        assert_eq!(timed[2].len(), 2);
        assert_eq!(timed[2][1], timed[0][0]);
    }

    #[tokio::test]
    async fn narration_not_on_screen_is_still_subtitled_with_on() {
        let tokens = vec![
            RecognizedToken::new("Hello", 100, 500),
            RecognizedToken::new("world.", 500, 1200),
        ];
        let mut t = two_scenes();
        let cfg = SchedulerConfig::new(
            PathBuf::from("w"),
            PathBuf::from("o.mp4"),
            PathBuf::from("r"),
        );
        let mut clips = narration_clips(&t, &cfg);
        // As if the second scene's narration were not shown as text.
        clips[1].on_screen = false;
        let engine = Arc::new(FixedTranscriber(tokens, Mutex::new(vec![])));
        let ws = WordSubtitleConfig {
            auto_subtitles: AutoSubtitles::On,
            ..WordSubtitleConfig::default()
        };
        let clips = clips_to_time(&t.subtitles, clips, AutoSubtitles::On);
        assert_eq!(clips.len(), 1);
        apply_word_subtitles(
            &mut t,
            &clips,
            engine,
            Arc::new(FixedProbe(1500)),
            &ws,
            &RenderContext::silent(),
            &mut vec![],
        )
        .await
        .unwrap();
        assert_eq!(t.subtitles.len(), 1);
        assert_eq!(t.subtitles[0].text, "你好世界。");
        assert!(t.subtitles[0].start_ms >= 2000);
        assert!(!t.subtitles[0].words.is_empty());
        assert_eq!(validate(&t), Ok(()));
    }

    #[tokio::test]
    async fn on_with_everything_on_screen_needs_no_engine_and_says_why() {
        let mut t = two_scenes();
        let before = t.clone();
        let mut cfg = SchedulerConfig::new(
            PathBuf::from("w"),
            PathBuf::from("o.mp4"),
            PathBuf::from("r"),
        );
        cfg.word_subtitles.whisper_cli = Some(PathBuf::from("/nonexistent/whisper-cli"));
        cfg.word_subtitles.auto_subtitles = AutoSubtitles::On;
        let (mut warnings, mut notes) = (vec![], vec![]);
        let n = add_word_subtitles(
            &mut t,
            &cfg,
            Arc::new(FixedProbe(1000)),
            &RenderContext::silent(),
            &mut warnings,
            &mut notes,
        )
        .await
        .unwrap();
        assert_eq!(n, 0);
        assert_eq!(t, before);
        assert!(warnings.is_empty(), "{warnings:?}");
        assert_eq!(notes.len(), 1, "{notes:?}");
        assert!(notes[0].contains("(one, two)"), "{notes:?}");
    }
}
