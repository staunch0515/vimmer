//! "Detect beats" for the music panel: the same detection and cache a render uses.

use std::path::Path;

use serde_json::{json, Value};

use crate::assets::public_dir;
use crate::render::{find_in_path, user_path};

/// Detects the beats of `source` (public-relative). Returns what `vimmer beats --json` prints:
/// `{"ok": true, "bpm", "confidence", "low_confidence", "downbeat", "duration_ms", "beats_ms"}` or
/// `{"ok": false, "error"}`.
#[tauri::command]
pub async fn detect_beats(source: String) -> Value {
    tauri::async_runtime::spawn_blocking(move || {
        detect_in(
            &public_dir(),
            &source,
            &find_in_path("ffmpeg", &user_path()),
        )
    })
    .await
    .unwrap_or_else(|e| json!({ "ok": false, "error": e.to_string() }))
}

pub fn detect_in(public: &Path, source: &str, ffmpeg: &Path) -> Value {
    if let Some(problem) = vimmer_dsl::validation::asset_path_problem(source) {
        return json!({ "ok": false, "error": format!("music path: {problem}") });
    }
    let file = public.join(source);
    if !file.is_file() {
        return json!({ "ok": false, "error": format!("No music file at public/{source}.") });
    }
    match vimmer_media::detect_beats_cached(ffmpeg, &file, &public.join("generated").join("beats"))
    {
        Ok(a) => json!({
            "ok": true,
            "bpm": (a.bpm * 100.0).round() / 100.0,
            "confidence": (a.confidence * 1000.0).round() / 1000.0,
            "low_confidence": a.confidence < vimmer_media::LOW_CONFIDENCE,
            "downbeat": a.downbeat,
            "duration_ms": a.duration_ms.round(),
            "beats_ms": a.beats_ms.iter().map(|b| (b * 10.0).round() / 10.0).collect::<Vec<_>>(),
        }),
        Err(e) => json!({ "ok": false, "error": e.to_string() }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_the_bundled_test_track_and_refuses_paths_outside_public() {
        let public = public_dir();
        let out = detect_in(&public, "assets/music/beat-120bpm.m4a", Path::new("ffmpeg"));
        if out["ok"] == true {
            assert!((out["bpm"].as_f64().unwrap() - 120.0).abs() < 1.0, "{out}");
        } else {
            // Without ffmpeg on PATH only the error shape can be checked.
            assert!(out["error"].is_string());
        }
        let bad = detect_in(&public, "../secrets.txt", Path::new("ffmpeg"));
        assert_eq!(bad["ok"], false);
        assert!(bad["error"].as_str().unwrap().starts_with("music path:"));
    }
}
