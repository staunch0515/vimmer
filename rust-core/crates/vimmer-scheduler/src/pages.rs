//! Text paging (文字分页): when each page of a paged scene text starts, decided once the scene
//! lengths and the narration's word timings are final, and handed to the renderer as
//! `scene.text_pages` (render props only; see `vimmer_dsl::text_layout`).
//!
//! * A scene whose synthesized narration speaks its own text: each page from just before its
//!   first word is spoken (the words recognized for word subtitles, or recognized for this when
//!   no subtitle needs them), else by characters over the narration clip.
//! * Any other scene (no narration, its own audio, or a spoken line that differs from the text):
//!   by characters over the scene, at least `MIN_PAGE_MS` per page when the scene is long enough.
//!
//! Scenes without paging (or whose text fits one page) get no pages and render exactly as before.

use std::collections::HashMap;
use std::sync::Arc;

use vimmer_dsl::text_layout::{timed_pages, PageClock};
use vimmer_dsl::{Scene, VideoTimeline};
use vimmer_media::AudioProbe;

use crate::run::{SchedulerConfig, SchedulerError};
use crate::wordsubs::PageWords;

/// How a scene's pages are timed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PageTiming {
    Words,
    Clip,
    Scene,
}

impl PageTiming {
    fn label(self) -> &'static str {
        match self {
            PageTiming::Words => "on the narration's words",
            PageTiming::Clip => "by length over the narration",
            PageTiming::Scene => "by length over the scene",
        }
    }
}

/// The synthesized narration of a scene that speaks its own text: (clip path, offset ms).
fn own_narration(scene: &Scene) -> Option<(&str, u64)> {
    let audio = scene.audio.as_ref()?;
    if scene.narration.is_none() || !audio.source.starts_with("generated/tts/") {
        return None;
    }
    (scene.spoken_text() == scene.text.as_deref()).then_some((audio.source.as_str(), audio.offset_ms))
}

/// Sets `text_pages` on every scene (cleared where there is no paging). `clip_ms` holds the
/// narration clip length of the scenes that speak their own text, by scene index. Returns one
/// note per paged scene.
pub fn resolve_text_pages(
    timeline: &mut VideoTimeline,
    page_words: &PageWords,
    clip_ms: &HashMap<usize, u64>,
) -> Vec<String> {
    let fps = u64::from(timeline.fps);
    let starts = timeline.scene_start_frames();
    let (brand, res) = (timeline.brand.clone(), timeline.resolution);
    let mut notes = Vec::new();
    for (i, scene) in timeline.scenes.iter_mut().enumerate() {
        scene.text_pages.clear();
        let pages = scene.text_page_texts(brand.as_ref(), res);
        if pages.len() <= 1 {
            continue;
        }
        // Where narration_clips puts the scene: its first frame, in ms.
        let scene_start = (u64::from(starts[i]) * 1000 + fps / 2) / fps;
        let narration = own_narration(scene).and_then(|(_, offset)| Some((offset, *clip_ms.get(&i)?)));
        let words: Option<Vec<(String, u64, u64)>> = page_words.get(&i).map(|ws| {
            ws.iter()
                .map(|(t, at, end)| {
                    (
                        t.clone(),
                        at.saturating_sub(scene_start),
                        end.saturating_sub(scene_start),
                    )
                })
                .collect()
        });
        let (clock, timing) = match (narration, &words) {
            (Some((offset_ms, clip_ms)), Some(words)) => (
                PageClock::Words {
                    words,
                    offset_ms,
                    clip_ms,
                },
                PageTiming::Words,
            ),
            (Some((offset_ms, clip_ms)), None) => {
                (PageClock::Clip { offset_ms, clip_ms }, PageTiming::Clip)
            }
            _ => (
                PageClock::Scene {
                    duration_ms: scene.duration_ms,
                },
                PageTiming::Scene,
            ),
        };
        let timed = timed_pages(pages, &clock);
        notes.push(format!(
            "scene `{}`: text in {} pages, changing {} (at {} ms)",
            scene.id,
            timed.len(),
            timing.label(),
            timed
                .iter()
                .map(|p| p.start_ms.to_string())
                .collect::<Vec<_>>()
                .join(", ")
        ));
        scene.text_pages = timed;
    }
    notes
}

/// [`resolve_text_pages`] with the narration clip lengths probed from the files.
pub async fn add_text_pages(
    timeline: &mut VideoTimeline,
    page_words: &PageWords,
    cfg: &SchedulerConfig,
    probe: Arc<dyn AudioProbe>,
    notes: &mut Vec<String>,
) -> Result<(), SchedulerError> {
    let (brand, res) = (timeline.brand.as_ref(), timeline.resolution);
    let clips: Vec<(usize, std::path::PathBuf)> = timeline
        .scenes
        .iter()
        .enumerate()
        .filter(|(_, s)| s.text_page_texts(brand, res).len() > 1)
        .filter_map(|(i, s)| own_narration(s).map(|(src, _)| (i, cfg.public_dir.join(src))))
        .collect();
    let mut clip_ms = HashMap::new();
    if !clips.is_empty() {
        let measured = tokio::task::spawn_blocking(move || {
            clips
                .into_iter()
                .map(|(i, path)| probe.duration_ms(&path).map(|ms| (i, ms)))
                .collect::<Result<Vec<_>, _>>()
        })
        .await
        .map_err(|e| SchedulerError::Join(e.to_string()))??;
        clip_ms.extend(measured);
    }
    notes.extend(resolve_text_pages(timeline, page_words, &clip_ms));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use vimmer_dsl::TextPage;

    fn timeline(json: serde_json::Value) -> VideoTimeline {
        serde_json::from_value(serde_json::json!({
            "fps": 30, "resolution": {"width": 1920, "height": 1080}, "scenes": json
        }))
        .unwrap()
    }

    const TEXT: &str = "第一句话在这里。第二句话也在这里。第三句话就在这里！";

    fn paged(extra: serde_json::Value) -> serde_json::Value {
        let mut s = serde_json::json!({
            "id": "p", "duration_ms": 6000, "text": TEXT, "text_layout": {"paging": "auto", "size": 160}
        });
        for (k, v) in extra.as_object().unwrap() {
            s[k] = v.clone();
        }
        s
    }

    #[test]
    fn unpaged_scenes_get_no_pages_and_serialize_as_before() {
        let raw = serde_json::json!([{"id": "a", "duration_ms": 1000, "text": "One. Two. Three."},
            {"id": "b", "duration_ms": 1000, "text": "短", "text_layout": {"paging": "auto"}}]);
        let mut t = timeline(raw);
        let before = serde_json::to_string(&t).unwrap();
        t.scenes[0].text_pages = vec![TextPage {
            text: "stale".into(),
            start_ms: 0,
        }];
        let notes = resolve_text_pages(&mut t, &PageWords::new(), &HashMap::new());
        assert!(notes.is_empty());
        assert_eq!(serde_json::to_string(&t).unwrap(), before);
    }

    #[test]
    fn a_silent_scene_pages_by_length_over_its_duration() {
        let mut t = timeline(serde_json::json!([paged(serde_json::json!({}))]));
        let notes = resolve_text_pages(&mut t, &PageWords::new(), &HashMap::new());
        let p = &t.scenes[0].text_pages;
        assert_eq!(p.len(), 3);
        assert_eq!(p[0].text, "第一句话在这里。");
        // 7 / 8 / 7 characters over 6000 ms with 1200 ms each: 1200 + 2400 * 7/22.
        assert_eq!(
            p.iter().map(|x| x.start_ms).collect::<Vec<_>>(),
            vec![0, 1200 + 2400 * 7 / 23, 2400 + 2400 * 15 / 23]
        );
        assert!(notes[0].contains("by length over the scene"), "{notes:?}");
    }

    #[test]
    fn narration_of_the_text_times_pages_by_words_else_by_the_clip() {
        let narrated = serde_json::json!({
            "narration": {"voice": null},
            "audio": {"source": "generated/tts/p-1.wav", "offset_ms": 0}
        });
        let mut t = timeline(serde_json::json!([
            {"id": "intro", "duration_ms": 1000, "text": "Hi"},
            paged(narrated.clone())
        ]));
        let clip = HashMap::from([(1usize, 4_400u64)]);
        // Clip length only: by characters over the clip.
        resolve_text_pages(&mut t, &PageWords::new(), &clip);
        assert_eq!(
            t.scenes[1].text_pages.iter().map(|x| x.start_ms).collect::<Vec<_>>(),
            vec![0, 4_400 * 7 / 23, 4_400 * 15 / 23]
        );
        // Words (video time): each page 150 ms before its first word.
        // Each character 200 ms, spoken for 120 ms (a pause before every page's first word).
        let words: Vec<(String, u64, u64)> = TEXT
            .chars()
            .enumerate()
            .map(|(k, c)| (c.to_string(), 1_000 + 200 * k as u64, 1_120 + 200 * k as u64))
            .collect();
        let pw = PageWords::from([(1usize, words)]);
        let notes = resolve_text_pages(&mut t, &pw, &clip);
        // Page 2 starts at character 8 (第二…), page 3 at character 17.
        assert_eq!(
            t.scenes[1].text_pages.iter().map(|x| x.start_ms).collect::<Vec<_>>(),
            vec![0, 200 * 8 - 150, 200 * 17 - 150]
        );
        assert!(notes[0].contains("on the narration's words"), "{notes:?}");
        // A different spoken line: the scene's length decides.
        let mut other = narrated;
        other["narration"]["text"] = "Something else entirely".into();
        let mut t = timeline(serde_json::json!([paged(other)]));
        resolve_text_pages(&mut t, &pw, &HashMap::new());
        assert_eq!(t.scenes[0].text_pages[1].start_ms, 1200 + 2400 * 7 / 23);
    }

    #[test]
    fn the_same_timeline_always_gets_the_same_pages() {
        let mut a = timeline(serde_json::json!([paged(serde_json::json!({}))]));
        let mut b = a.clone();
        resolve_text_pages(&mut a, &PageWords::new(), &HashMap::new());
        resolve_text_pages(&mut b, &PageWords::new(), &HashMap::new());
        assert_eq!(a, b);
    }
}
