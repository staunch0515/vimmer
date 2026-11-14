//! The music panel's "By mood" mode: the library's moods and tracks, and which track a pick gets
//! for the open timeline. Same code and JSON as `vimmer music list|pick --json`.

use std::path::Path;

use serde_json::{json, Value};
use vimmer_dsl::{list_json, pick_json, wanted_pick, MusicLibrary, VideoTimeline};

use crate::assets::public_dir;

/// `{"ok": true, "moods", "main_moods", "tracks", "missing"}` or `{"ok": false, "error"}`.
#[tauri::command]
pub fn music_library() -> Value {
    library_in(&public_dir())
}

/// The track `music.pick` (or `mood`/`energy`, when given) gets for `timeline_json`:
/// `{"ok": true, "track", "reason", "candidates", ...}` or `{"ok": false, "error"}`.
#[tauri::command]
pub fn music_pick(timeline_json: String, mood: Option<String>, energy: Option<f32>) -> Value {
    pick_in(&public_dir(), &timeline_json, mood.as_deref(), energy)
}

pub fn library_in(public: &Path) -> Value {
    MusicLibrary::load(public)
        .and_then(|(lib, missing)| list_json(&lib, &missing, None))
        .unwrap_or_else(|e| json!({ "ok": false, "error": e.to_string() }))
}

pub fn pick_in(
    public: &Path,
    timeline_json: &str,
    mood: Option<&str>,
    energy: Option<f32>,
) -> Value {
    let timeline: VideoTimeline = match serde_json::from_str(timeline_json) {
        Ok(t) => t,
        Err(e) => {
            return json!({ "ok": false, "error": format!("the timeline does not parse: {e}") })
        }
    };
    let Some(pick) = wanted_pick(&timeline, mood, energy) else {
        return json!({ "ok": false, "error": "the music has no mood to pick by" });
    };
    MusicLibrary::load(public)
        .and_then(|(lib, missing)| pick_json(&timeline, &lib, &missing, &pick))
        .unwrap_or_else(|e| json!({ "ok": false, "error": e.to_string() }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lists_the_bundled_library_and_picks_deterministically() {
        let public = public_dir();
        let lib = library_in(&public);
        assert_eq!(lib["ok"], true, "{lib}");
        assert!(lib["main_moods"]
            .as_array()
            .unwrap()
            .iter()
            .any(|m| m == "upbeat"));
        let t =
            std::fs::read_to_string(public.join("../../fixtures/mood-music-demo.json")).unwrap();
        let a = pick_in(&public, &t, None, None);
        assert_eq!(a["ok"], true, "{a}");
        assert_eq!(a, pick_in(&public, &t, None, None));
        let calm = pick_in(&public, &t, Some("calm"), Some(0.1));
        assert_eq!(calm["track"]["id"], "calm-drift", "{calm}");
        let bad = pick_in(&public, &t, Some("sad"), None);
        assert!(
            bad["error"].as_str().unwrap().contains("available moods"),
            "{bad}"
        );
    }
}
