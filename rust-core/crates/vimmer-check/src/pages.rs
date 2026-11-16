//! The pages a scene text is shown in (see `vimmer_dsl::text_layout`) and when, as the checks
//! see them. An unpaged text is one page for the whole scene, so every check that goes through
//! [`scene_pages`] reports an old timeline exactly as before.
//!
//! The render times narrated pages on the recognized words; here they are estimated: a scene that
//! speaks its own text spreads its pages over the estimated narration (as `say` reads it, the
//! shorter estimate), any other scene over its duration (at least `MIN_PAGE_MS` a page), a
//! scene with a spoken line over the longer of its duration and the line.

use vimmer_dsl::text_layout::{page_starts, PageClock};
use vimmer_dsl::VideoTimeline;
use vimmer_media::TtsEngine;

/// One page of a scene text.
#[derive(Debug, Clone, PartialEq)]
pub struct ScenePage {
    pub text: String,
    /// When the page shows, ms from the scene start.
    pub from_ms: u64,
    pub to_ms: u64,
}

/// How long the scene text is on screen: the scene, or longer when narration stretches it.
pub fn shown_ms(t: &VideoTimeline, i: usize) -> u64 {
    let scene = &t.scenes[i];
    let mut shown = scene.duration_ms;
    if scene.audio.is_none() {
        if let Some(said) = scene.spoken_text() {
            let speech = crate::timing::estimate_speech_ms(said, TtsEngine::Say);
            shown = shown.max(speech + crate::audio::TAIL_MS);
        }
    }
    shown
}

/// The pages of scene `i` with their windows; empty without text, one page (the whole text over
/// the whole scene) without paging.
pub fn scene_pages(t: &VideoTimeline, i: usize) -> Vec<ScenePage> {
    let scene = &t.scenes[i];
    if scene.text.as_deref().is_none_or(|s| s.trim().is_empty()) {
        return Vec::new();
    }
    let texts = scene.text_page_texts(t.brand.as_ref(), t.resolution);
    if texts.len() <= 1 {
        return texts
            .into_iter()
            .map(|text| ScenePage {
                text,
                from_ms: 0,
                to_ms: scene.duration_ms,
            })
            .collect();
    }
    let shown = shown_ms(t, i);
    let own_speech = scene.audio.is_none()
        && scene.narration.is_some()
        && scene.spoken_text() == scene.text.as_deref();
    let starts = if own_speech {
        let clip_ms = crate::timing::estimate_speech_ms(scene.text.as_deref().unwrap_or(""), TtsEngine::Say);
        page_starts(&texts, &PageClock::Clip { offset_ms: 0, clip_ms })
    } else {
        page_starts(&texts, &PageClock::Scene { duration_ms: shown })
    };
    let n = texts.len();
    texts
        .into_iter()
        .enumerate()
        .map(|(k, text)| ScenePage {
            text,
            from_ms: starts[k],
            to_ms: if k + 1 < n { starts[k + 1] } else { shown },
        })
        .collect()
}

/// How a finding names page `k` of `n` (just "scene text" when unpaged).
pub fn page_name(k: usize, n: usize) -> String {
    if n > 1 {
        format!("page {}/{n} of the scene text", k + 1)
    } else {
        "scene text".to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t(scene: serde_json::Value) -> VideoTimeline {
        serde_json::from_value(serde_json::json!({
            "fps": 30, "resolution": {"width": 1920, "height": 1080}, "scenes": [scene]
        }))
        .unwrap()
    }

    #[test]
    fn unpaged_text_is_one_page_for_the_whole_scene() {
        let tl = t(serde_json::json!({"id": "a", "duration_ms": 3000, "text": "One. Two."}));
        assert_eq!(
            scene_pages(&tl, 0),
            vec![ScenePage {
                text: "One. Two.".into(),
                from_ms: 0,
                to_ms: 3000
            }]
        );
        let tl = t(serde_json::json!({"id": "a", "duration_ms": 3000, "text": " "}));
        assert!(scene_pages(&tl, 0).is_empty());
    }

    #[test]
    fn pages_share_the_scene_or_the_estimated_narration() {
        let text = "第一句话在这里。第二句话也在这里。第三句话就在这里！";
        let mut scene = serde_json::json!({"id": "a", "duration_ms": 9000, "text": text,
            "text_layout": {"paging": "auto", "size": 160}});
        let p = scene_pages(&t(scene.clone()), 0);
        assert_eq!(p.len(), 3);
        assert_eq!((p[0].from_ms, p[2].to_ms), (0, 9000));
        assert_eq!(p[0].to_ms, p[1].from_ms);
        scene["narration"] = serde_json::json!({"voice": null});
        let p = scene_pages(&t(scene), 0);
        let speech = crate::timing::estimate_speech_ms(text, TtsEngine::Say);
        assert!(p[2].from_ms < speech, "{p:?} {speech}");
        assert_eq!(p[2].to_ms, 9000.max(speech + crate::audio::TAIL_MS));
    }

    fn codes(f: &[crate::Finding]) -> Vec<(&'static str, String)> {
        f.iter().map(|x| (x.code, x.message.clone())).collect()
    }

    const LONG: &str = "咖啡豆要新鲜研磨，香气才会完整地保留下来。水温控制在九十度左右最合适，太烫会带出苦味。最后慢慢注水，耐心等待萃取完成，然后就可以享用了！";

    #[test]
    fn overflow_is_checked_per_page_at_the_layout_size() {
        let scene = |layout: serde_json::Value| {
            t(serde_json::json!({"id": "a", "duration_ms": 30000, "text": LONG, "text_layout": layout}))
        };
        let f = crate::text::check(&scene(serde_json::json!({"size": 160})));
        assert!(f.iter().any(|x| x.code == "text.overflow" && x.message.contains("at 160 px")), "{:?}", codes(&f));
        let f = crate::text::check(&scene(serde_json::json!({"size": 160, "paging": "auto"})));
        assert!(f.iter().all(|x| x.code != "text.overflow"), "{:?}", codes(&f));
        // A manual page that is too long is named.
        let long_page = format!("短的一页。\n\n{LONG}");
        let tl = t(serde_json::json!({"id": "a", "duration_ms": 30000, "text": long_page,
            "text_layout": {"size": 160, "paging": "manual"}}));
        let f = crate::text::check(&tl);
        let o: Vec<_> = f.iter().filter(|x| x.code == "text.overflow").collect();
        assert_eq!(o.len(), 1, "{:?}", codes(&f));
        assert!(o[0].message.starts_with("page 2/2 of the scene text needs about"), "{}", o[0].message);
        assert!(o[0].suggestion.text.contains("start a new page"), "{:?}", o[0].suggestion);
    }

    #[test]
    fn reading_speed_is_checked_per_page() {
        let tl = t(serde_json::json!({"id": "a", "duration_ms": 6000, "text": LONG,
            "text_layout": {"size": 160, "paging": "auto"}}));
        let f = crate::text::check(&tl);
        let r: Vec<_> = f.iter().filter(|x| x.code == "text.reading_speed").collect();
        assert!(!r.is_empty(), "{:?}", codes(&f));
        assert!(r.iter().all(|x| x.message.starts_with("page ")), "{:?}", codes(&f));
        // Lengthening the scene as suggested fixes the page.
        let needed = r[0].suggestion.value.as_ref().unwrap().as_u64().unwrap();
        let mut longer = tl.clone();
        longer.scenes[0].duration_ms = needed;
        let still: Vec<_> = crate::text::check(&longer)
            .into_iter()
            .filter(|x| x.code == "text.reading_speed" && x.message == r[0].message)
            .collect();
        assert!(still.is_empty());
        // Narrated pages are paced by the speech, as an unpaged narrated text is.
        let mut narrated = tl;
        narrated.scenes[0].narration = Some(Default::default());
        assert!(crate::text::check(&narrated).iter().all(|x| x.code != "text.reading_speed"));
    }

    #[test]
    fn position_moves_the_text_box_for_the_logo_and_safe_area_checks() {
        let logo = r#"{"src":"l.png","corner":"bottom_left","size":0.2,"margin":0.08}"#;
        let text = "This is a long scene text that wraps across three full lines of the frame so that it reaches far to the left side";
        let mk = |pos: &str| -> VideoTimeline {
            serde_json::from_str(&format!(
                r#"{{"fps":30,"resolution":{{"width":1920,"height":1080}},"brand":{{"logo":{logo}}},
                "scenes":[{{"id":"a","duration_ms":3000,"text":"{text}","text_layout":{{"position":"{pos}"}}}}]}}"#
            ))
            .unwrap()
        };
        let fonts = crate::fonts::Fonts::none();
        assert_eq!(crate::logo::overlap(&mk("bottom"), None, &fonts).len(), 1);
        assert!(crate::logo::overlap(&mk("top"), None, &fonts).is_empty());
        // A top logo: top text starts below it, so it is not covered either.
        let top_logo = r#"{"src":"l.png","corner":"top_right","size":0.1,"margin":0.05}"#;
        let tl: VideoTimeline = serde_json::from_str(&format!(
            r#"{{"fps":30,"resolution":{{"width":1920,"height":1080}},"brand":{{"logo":{top_logo}}},
            "scenes":[{{"id":"a","duration_ms":3000,"text":"{text}","text_layout":{{"position":"top"}}}}]}}"#
        ))
        .unwrap();
        assert!(crate::logo::overlap(&tl, None, &fonts).is_empty());
        let (area, reach) = crate::layout::SceneTextArea::for_scene(&tl, 0);
        assert!(area.text_top(3, reach.total()) >= 0.17 * 1080.0 - 1e-3);
        // Middle text is centered between the top padding and the bottom inset.
        let (mid, _) = crate::layout::SceneTextArea::for_scene(&mk("middle"), 0);
        let top = mid.text_top(2, 0.0);
        let bottom = top + 2.0 * mid.line_height_px;
        assert!(((top - mid.top_y) - (mid.bottom_y - bottom)).abs() < 1e-6);
        assert!(crate::layout::check(&mk("top")).is_empty());
    }
}
