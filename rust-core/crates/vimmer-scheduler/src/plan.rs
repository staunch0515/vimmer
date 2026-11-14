use std::ops::Range;

use serde::Serialize;
use vimmer_dsl::{SubtitleCue, VideoTimeline, WordTiming};

/// Default target length of one render chunk. Chunks render in parallel, then get concatenated.
pub const DEFAULT_CHUNK_MS: u64 = 30_000;

/// A contiguous run of whole scenes rendered as one file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Chunk {
    pub index: usize,
    /// Indices into `timeline.scenes`.
    pub scenes: Range<usize>,
    /// Absolute frame of this chunk's first frame in the full timeline.
    pub start_frame: u32,
    pub frame_count: u32,
}

/// Greedily groups whole scenes into chunks of at most `max_chunk_ms`.
/// A scene longer than the limit becomes its own chunk. Scenes are never split,
/// so every scene's frames stay local to one render. A scene with a `transition_in`
/// always joins the chunk of the scene before it, because the transition needs that
/// scene's last frame. A chunk can therefore exceed `max_chunk_ms` for a long run of
/// transitions.
pub fn plan_chunks(timeline: &VideoTimeline, max_chunk_ms: u64) -> Vec<Chunk> {
    let max_chunk_ms = max_chunk_ms.max(1);
    let starts = timeline.scene_start_frames();
    let mut chunks = Vec::new();
    let mut first = 0usize;
    let mut acc_ms = 0u64;

    for (i, scene) in timeline.scenes.iter().enumerate() {
        let joins_previous = scene.transition_in.is_some();
        if i > first && !joins_previous && acc_ms + scene.duration_ms > max_chunk_ms {
            push_chunk(&mut chunks, timeline, &starts, first..i);
            first = i;
            acc_ms = 0;
        }
        acc_ms += scene.duration_ms;
    }
    if first < timeline.scenes.len() {
        push_chunk(&mut chunks, timeline, &starts, first..timeline.scenes.len());
    }
    chunks
}

/// Builds the self-contained timeline for one chunk. Subtitle cues are clipped to the
/// chunk's frame range and rebased so the chunk starts at 0 ms. Cues that straddle a
/// boundary appear in both chunks, each for its own part. Their words are rebased the same way
/// and clamped into the clipped cue, so every word stays (the line reads the same in both
/// chunks); words outside the chunk become zero-length at its edge, already said or not yet.
///
/// Words are rebased by frame, not by millisecond: each word time lands on the chunk frame that
/// is exactly its full-timeline frame minus the chunk start, so the renderer computes the same
/// word states in a chunk as in a full render (millisecond rebasing can round a word one frame
/// off when the chunk start is not a whole millisecond). The renderer stops highlighting a word
/// at the next scene cut, and the chunk start is always one, so words clamped to it read as
/// already spoken (see remotion-renderer/src/subtitles/words.ts).
///
/// The scheduler renders [`chunk_props`], which adds the unclipped cues; the renderer draws
/// those instead, so a clipped cue never fades at a seam. The clipped track stays as the
/// fallback for a renderer that ignores the extra key.
pub fn chunk_timeline(timeline: &VideoTimeline, chunk: &Chunk) -> VideoTimeline {
    let fps = timeline.fps;
    let chunk_start = chunk.start_frame;
    let chunk_end = chunk.start_frame + chunk.frame_count;
    let to_frame = |ms: u64| ((ms * u64::from(fps) + 500) / 1000) as u32;
    let to_ms = |frame: u32| (f64::from(frame) * 1000.0 / f64::from(fps)).round() as u64;

    let subtitles = timeline
        .subtitles
        .iter()
        .filter_map(|cue| {
            let start = to_frame(cue.start_ms).max(chunk_start);
            let end = to_frame(cue.end_ms).min(chunk_end);
            if start >= end {
                return None;
            }
            let start_ms = to_ms(start - chunk_start);
            let end_ms = to_ms(end - chunk_start);
            let rebase = |ms: u64| to_ms(to_frame(ms).clamp(start, end) - chunk_start);
            let words = cue
                .words
                .iter()
                .map(|w| WordTiming {
                    text: w.text.clone(),
                    start_ms: rebase(w.start_ms),
                    end_ms: rebase(w.end_ms),
                })
                .collect();
            Some(SubtitleCue {
                start_ms,
                end_ms,
                text: cue.text.clone(),
                words,
            })
        })
        .collect();

    VideoTimeline {
        // The brand draws the logo and sets fonts and default colors in every chunk.
        brand: timeline.brand.clone(),
        fps,
        resolution: timeline.resolution,
        scenes: timeline.scenes[chunk.scenes.clone()].to_vec(),
        subtitles,
        // Music is mixed once, over the joined video, so chunks never carry it.
        music: None,
    }
}

/// Render props for one chunk: the self-contained [`chunk_timeline`] plus a [`ChunkSubtitles`]
/// block under the `chunk` key. Only the scheduler writes this key; a user timeline never has it,
/// and without it the renderer draws `subtitles` as before.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ChunkProps {
    #[serde(flatten)]
    pub timeline: VideoTimeline,
    pub chunk: ChunkSubtitles,
}

/// The subtitle track of a chunk in full-timeline time, so the renderer draws every chunk frame
/// exactly like the same frame of a full render. A cue that straddles a seam keeps its real start
/// and end, so it fades in and out once, at its real edges, instead of fading out at the end of
/// one chunk and in again at the start of the next (a blink of about twice the fade length).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ChunkSubtitles {
    /// Full-timeline frame of the chunk's frame 0; the renderer adds it to the current frame.
    pub start_frame: u32,
    /// Unchanged cues (full-timeline milliseconds, words included) that overlap the chunk.
    pub subtitles: Vec<SubtitleCue>,
}

/// Builds the render props for one chunk (see [`ChunkProps`]).
pub fn chunk_props(timeline: &VideoTimeline, chunk: &Chunk) -> ChunkProps {
    let fps = timeline.fps;
    let to_frame = |ms: u64| ((ms * u64::from(fps) + 500) / 1000) as u32;
    let chunk_end = chunk.start_frame + chunk.frame_count;
    let subtitles = timeline
        .subtitles
        .iter()
        .filter(|cue| {
            to_frame(cue.start_ms).max(chunk.start_frame) < to_frame(cue.end_ms).min(chunk_end)
        })
        .cloned()
        .collect();
    ChunkProps {
        timeline: chunk_timeline(timeline, chunk),
        chunk: ChunkSubtitles {
            start_frame: chunk.start_frame,
            subtitles,
        },
    }
}

fn push_chunk(
    out: &mut Vec<Chunk>,
    timeline: &VideoTimeline,
    starts: &[u32],
    scenes: Range<usize>,
) {
    let start_frame = starts[scenes.start];
    let frame_count: u32 = timeline.scenes[scenes.clone()]
        .iter()
        .map(|s| s.duration_frames(timeline.fps))
        .sum();
    out.push(Chunk {
        index: out.len(),
        scenes,
        start_frame,
        frame_count,
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use vimmer_dsl::{MotionType, Resolution, Scene};

    fn scene(id: &str, ms: u64) -> Scene {
        Scene {
            sfx: vec![],
            template: None,
            id: id.into(),
            duration_ms: ms,
            text: Some("x".into()),
            highlights: vec![],
            image: None,
            motion: MotionType::Static,
            video: None,
            audio: None,
            narration: None,
            transition_in: None,
            keyframes: vec![],
            layers: vec![],
        }
    }

    fn timeline(durations: &[u64]) -> VideoTimeline {
        VideoTimeline {
            brand: None,
            fps: 30,
            resolution: Resolution {
                width: 1920,
                height: 1080,
            },
            scenes: durations
                .iter()
                .enumerate()
                .map(|(i, &ms)| scene(&format!("s{i}"), ms))
                .collect(),
            subtitles: vec![],
            music: None,
        }
    }

    #[test]
    fn groups_scenes_under_limit() {
        let t = timeline(&[10_000, 10_000, 10_000, 10_000]);
        let chunks = plan_chunks(&t, 30_000);
        assert_eq!(chunks.len(), 2);
        assert_eq!(chunks[0].scenes, 0..3);
        assert_eq!(chunks[1].scenes, 3..4);
        assert_eq!(chunks[1].start_frame, 900);
        assert_eq!(chunks[1].frame_count, 300);
    }

    #[test]
    fn oversized_scene_gets_its_own_chunk() {
        let t = timeline(&[5_000, 60_000, 5_000]);
        let chunks = plan_chunks(&t, 30_000);
        assert_eq!(
            chunks.iter().map(|c| c.scenes.clone()).collect::<Vec<_>>(),
            vec![0..1, 1..2, 2..3]
        );
    }

    #[test]
    fn frames_cover_whole_timeline_exactly() {
        let t = timeline(&[7_000, 9_000, 11_000, 13_000, 15_000, 4_000]);
        let chunks = plan_chunks(&t, DEFAULT_CHUNK_MS);
        let total: u32 = chunks.iter().map(|c| c.frame_count).sum();
        assert_eq!(total, t.total_frames());
        for pair in chunks.windows(2) {
            assert_eq!(
                pair[0].start_frame + pair[0].frame_count,
                pair[1].start_frame
            );
        }
    }

    #[test]
    fn chunk_timeline_clips_and_rebases_subtitles() {
        let mut t = timeline(&[10_000, 10_000]);
        t.subtitles = vec![
            SubtitleCue {
                start_ms: 1_000,
                end_ms: 2_000,
                text: "first".into(),
                words: Vec::new(),
            },
            SubtitleCue {
                start_ms: 9_000,
                end_ms: 11_000,
                text: "straddles".into(),
                words: Vec::new(),
            },
            SubtitleCue {
                start_ms: 15_000,
                end_ms: 16_000,
                text: "second".into(),
                words: Vec::new(),
            },
        ];
        let chunks = plan_chunks(&t, 10_000);
        let a = chunk_timeline(&t, &chunks[0]);
        let b = chunk_timeline(&t, &chunks[1]);
        assert_eq!(a.subtitles.len(), 2);
        assert_eq!(
            a.subtitles[1],
            SubtitleCue {
                start_ms: 9_000,
                end_ms: 10_000,
                text: "straddles".into(),
                words: Vec::new(),
            }
        );
        assert_eq!(b.subtitles.len(), 2);
        assert_eq!(
            b.subtitles[0],
            SubtitleCue {
                start_ms: 0,
                end_ms: 1_000,
                text: "straddles".into(),
                words: Vec::new(),
            }
        );
        assert_eq!(
            b.subtitles[1],
            SubtitleCue {
                start_ms: 5_000,
                end_ms: 6_000,
                text: "second".into(),
                words: Vec::new(),
            }
        );
        assert_eq!(b.total_duration_ms(), 10_000);
    }

    #[test]
    fn chunk_props_carry_seam_cues_in_global_time() {
        let mut t = timeline(&[10_000, 10_000]);
        let cue = |start_ms: u64, end_ms: u64, text: &str| SubtitleCue {
            start_ms,
            end_ms,
            text: text.into(),
            words: vec![
                WordTiming {
                    text: "a ".into(),
                    start_ms,
                    end_ms: start_ms + 100,
                },
                WordTiming {
                    text: text.into(),
                    start_ms: end_ms - 100,
                    end_ms,
                },
            ],
        };
        let first = cue(1_000, 2_000, "first");
        let straddles = cue(9_000, 11_000, "straddles");
        let second = cue(15_000, 16_000, "second");
        // Ends exactly on the seam: belongs to the first chunk only.
        let edge = cue(9_500, 10_000, "edge");
        t.subtitles = vec![
            first.clone(),
            straddles.clone(),
            edge.clone(),
            second.clone(),
        ];
        let chunks = plan_chunks(&t, 10_000);
        let a = chunk_props(&t, &chunks[0]);
        let b = chunk_props(&t, &chunks[1]);

        // The straddling cue keeps its real edges in both chunks, so neither chunk fades it at
        // the seam; the renderer offsets its frame by start_frame.
        assert_eq!(a.chunk.start_frame, 0);
        assert_eq!(a.chunk.subtitles, vec![first, straddles.clone(), edge]);
        assert_eq!(b.chunk.start_frame, 300);
        assert_eq!(b.chunk.subtitles, vec![straddles, second]);
        // The self-contained timeline is unchanged.
        assert_eq!(b.timeline, chunk_timeline(&t, &chunks[1]));

        let json = serde_json::to_value(&b).unwrap();
        assert_eq!(json["fps"], 30);
        assert_eq!(json["subtitles"][0]["start_ms"], 0);
        assert_eq!(json["chunk"]["start_frame"], 300);
        assert_eq!(json["chunk"]["subtitles"][0]["start_ms"], 9_000);
        assert_eq!(json["chunk"]["subtitles"][0]["words"][1]["end_ms"], 11_000);
    }

    #[test]
    fn chunk_timeline_rebases_and_clamps_words() {
        use vimmer_dsl::{validate, WordTiming};
        let word = |text: &str, start_ms: u64, end_ms: u64| WordTiming {
            text: text.into(),
            start_ms,
            end_ms,
        };
        let mut t = timeline(&[3_000, 3_000]);
        let mut cue = SubtitleCue::new(2_000, 4_500, "一二三四");
        cue.words = vec![
            word("一", 2_000, 2_600),
            word("二", 2_600, 3_200),
            word("三", 3_400, 4_000),
            word("四", 4_000, 4_500),
        ];
        t.subtitles = vec![cue];
        let chunks = plan_chunks(&t, 3_000);
        assert_eq!(chunks.len(), 2);
        let a = chunk_timeline(&t, &chunks[0]);
        let b = chunk_timeline(&t, &chunks[1]);
        let spans = |c: &SubtitleCue| -> Vec<(u64, u64)> {
            c.words.iter().map(|w| (w.start_ms, w.end_ms)).collect()
        };
        assert_eq!(
            (a.subtitles[0].start_ms, a.subtitles[0].end_ms),
            (2_000, 3_000)
        );
        assert_eq!(
            spans(&a.subtitles[0]),
            vec![
                (2_000, 2_600),
                (2_600, 3_000),
                (3_000, 3_000),
                (3_000, 3_000)
            ],
            "later words wait at the chunk end"
        );
        assert_eq!((b.subtitles[0].start_ms, b.subtitles[0].end_ms), (0, 1_500));
        assert_eq!(
            spans(&b.subtitles[0]),
            vec![(0, 0), (0, 200), (400, 1_000), (1_000, 1_500)],
            "earlier words are already said at the chunk start"
        );
        assert_eq!(b.subtitles[0].words.len(), 4, "the whole line is kept");
        assert_eq!(validate(&a), Ok(()));
        assert_eq!(validate(&b), Ok(()));
    }

    #[test]
    fn chunk_timeline_rebases_words_by_frame() {
        let word = |text: &str, start_ms: u64, end_ms: u64| WordTiming {
            text: text.into(),
            start_ms,
            end_ms,
        };
        // The seam example of remotion-renderer/test/word-subtitles.test.ts: "two " ends 15
        // frames before the seam at frame 90 and clamps to zero length at the second chunk's
        // start, where the renderer shows it as spoken (the chunk start is a scene cut).
        let mut t = timeline(&[3_000, 3_000]);
        let mut cue = SubtitleCue::new(1_000, 5_000, "one two three four");
        cue.words = vec![
            word("one ", 1_000, 1_400),
            word("two ", 1_400, 2_500),
            word("three ", 3_600, 4_200),
            word("four", 4_200, 4_800),
        ];
        t.subtitles = vec![cue];
        let chunks = plan_chunks(&t, 3_000);
        let b = chunk_timeline(&t, &chunks[1]);
        let spans: Vec<(u64, u64)> = b.subtitles[0]
            .words
            .iter()
            .map(|w| (w.start_ms, w.end_ms))
            .collect();
        assert_eq!(spans, vec![(0, 0), (0, 0), (600, 1_200), (1_200, 1_800)]);

        // A chunk start that is not a whole millisecond: 91 frames = 3033.3 ms. Every word must
        // land on its full-timeline frame minus 91, which millisecond rebasing misses for 3083 ms
        // (frame 92 -> chunk frame 1, but 3083 - 3033 = 50 ms rounds to frame 2).
        let mut t = timeline(&[3_033, 3_000]);
        let frame = |ms: u64| (ms * 30 + 500) / 1000;
        let mut cue = SubtitleCue::new(2_000, 5_000, "a b c d");
        cue.words = vec![
            word("a ", 2_000, 3_049),
            word("b ", 3_050, 3_083),
            word("c ", 3_084, 3_517),
            word("d", 3_517, 4_999),
        ];
        t.subtitles = vec![cue.clone()];
        let chunks = plan_chunks(&t, 3_033);
        assert_eq!(chunks[1].start_frame, 91);
        let b = chunk_timeline(&t, &chunks[1]);
        for (full, chunked) in cue.words.iter().zip(&b.subtitles[0].words) {
            for (f, c) in [
                (full.start_ms, chunked.start_ms),
                (full.end_ms, chunked.end_ms),
            ] {
                assert_eq!(frame(c), frame(f).max(91) - 91, "{} at {f} ms", full.text);
            }
        }
        assert_eq!(vimmer_dsl::validate(&b), Ok(()));
    }

    #[test]
    fn transition_keeps_scenes_in_one_chunk() {
        use vimmer_dsl::{Transition, TransitionKind};
        let mut t = timeline(&[10_000, 10_000, 10_000]);
        t.scenes[1].transition_in = Some(Transition {
            kind: TransitionKind::Fade,
            duration_ms: 1_000,
        });
        // Without the transition these would split at every scene (10 s limit).
        let chunks = plan_chunks(&t, 10_000);
        assert_eq!(
            chunks.iter().map(|c| c.scenes.clone()).collect::<Vec<_>>(),
            vec![0..2, 2..3]
        );
        assert_eq!(chunks[0].frame_count, 600);
        assert_eq!(chunks[1].start_frame, 600);
    }

    #[test]
    fn chain_of_transitions_stays_in_one_chunk() {
        use vimmer_dsl::{Transition, TransitionKind};
        let mut t = timeline(&[5_000, 5_000, 5_000, 5_000]);
        for scene in &mut t.scenes[1..] {
            scene.transition_in = Some(Transition {
                kind: TransitionKind::SlideLeft,
                duration_ms: 500,
            });
        }
        let chunks = plan_chunks(&t, 5_000);
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].scenes, 0..4);
        assert_eq!(chunks[0].frame_count, t.total_frames());
    }

    #[test]
    fn no_chunk_boundary_falls_inside_a_transition() {
        use vimmer_dsl::{Transition, TransitionKind};
        // Scene 2 and 4 carry transitions; the planner must never start a chunk on them.
        let mut t = timeline(&[7_000, 7_000, 7_000, 7_000, 7_000]);
        t.scenes[2].transition_in = Some(Transition {
            kind: TransitionKind::ZoomIn,
            duration_ms: 800,
        });
        t.scenes[4].transition_in = Some(Transition {
            kind: TransitionKind::Fade,
            duration_ms: 800,
        });
        for limit in [1, 7_000, 10_000, 14_000, 21_000, 30_000] {
            let chunks = plan_chunks(&t, limit);
            for c in &chunks {
                assert!(
                    t.scenes[c.scenes.start].transition_in.is_none() || c.scenes.start == 0,
                    "limit {limit}: chunk starts on a transition scene {:?}",
                    c.scenes
                );
            }
            let total: u32 = chunks.iter().map(|c| c.frame_count).sum();
            assert_eq!(total, t.total_frames());
        }
    }

    #[test]
    fn full_200_second_timeline_is_within_chunk_budget() {
        let durations = vec![10_000u64; 20];
        let t = timeline(&durations);
        let chunks = plan_chunks(&t, DEFAULT_CHUNK_MS);
        assert_eq!(chunks.len(), 7);
        assert_eq!(chunks.iter().map(|c| c.frame_count).sum::<u32>(), 6_000);
    }
}
