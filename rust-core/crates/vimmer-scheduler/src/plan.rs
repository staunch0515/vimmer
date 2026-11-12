use std::ops::Range;

use vimmer_dsl::VideoTimeline;

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
/// so every scene's frames stay local to one render.
pub fn plan_chunks(timeline: &VideoTimeline, max_chunk_ms: u64) -> Vec<Chunk> {
    let max_chunk_ms = max_chunk_ms.max(1);
    let starts = timeline.scene_start_frames();
    let mut chunks = Vec::new();
    let mut first = 0usize;
    let mut acc_ms = 0u64;

    for (i, scene) in timeline.scenes.iter().enumerate() {
        if i > first && acc_ms + scene.duration_ms > max_chunk_ms {
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
            id: id.into(),
            duration_ms: ms,
            text: Some("x".into()),
            highlights: vec![],
            image: None,
            motion: MotionType::Static,
            audio: None,
        }
    }

    fn timeline(durations: &[u64]) -> VideoTimeline {
        VideoTimeline {
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
    fn full_200_second_timeline_is_within_chunk_budget() {
        let durations = vec![10_000u64; 20];
        let t = timeline(&durations);
        let chunks = plan_chunks(&t, DEFAULT_CHUNK_MS);
        assert_eq!(chunks.len(), 7);
        assert_eq!(chunks.iter().map(|c| c.frame_count).sum::<u32>(), 6_000);
    }
}
