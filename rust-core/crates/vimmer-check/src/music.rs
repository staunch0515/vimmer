//! `music.pick`: resolves it against the music library the way the render does, reports the
//! chosen track, and warns about a poor fit (no track near the energy target, a track shorter
//! than the video that cannot loop) and about library entries whose file is missing. The rest
//! of the check then sees the chosen file as `music.source`.

use std::path::Path;

use vimmer_dsl::{resolve_music_pick, MusicLibrary, VideoTimeline, FAR_ENERGY};

use crate::{secs, Finding, Severity, Suggestion};

/// The timeline with its pick resolved (unchanged without a pick or when it cannot be
/// resolved), and the findings.
pub fn resolve(
    timeline: &VideoTimeline,
    public_dir: Option<&Path>,
) -> (VideoTimeline, Vec<Finding>) {
    let mut t = timeline.clone();
    let mut out = Vec::new();
    let Some(pick) = timeline.music.as_ref().and_then(|m| m.pick.clone()) else {
        return (t, out);
    };
    let Some(dir) = public_dir else {
        return (t, out);
    };
    let (library, missing) = match MusicLibrary::load(dir) {
        Ok(l) => l,
        Err(e) => {
            out.push(Finding::new(
                Severity::Error,
                "music.library",
                e.to_string(),
                Suggestion::text(
                    "restore remotion-renderer/public/assets/music/library/library.json (scripts/make-music-library.sh) or give music.source instead",
                ),
            ));
            return (t, out);
        }
    };
    for id in &missing {
        let file = MusicLibrary::load_index(dir)
            .ok()
            .and_then(|l| l.track(id).map(|t| t.source()))
            .unwrap_or_default();
        out.push(Finding::new(
            Severity::Warning,
            "music.library_file_missing",
            format!("library.json lists `{id}`, but its file `{file}` does not exist; it is never picked"),
            Suggestion::text("add the file or remove the entry from library.json"),
        ));
    }
    let choice = match resolve_music_pick(&mut t, &library) {
        Ok(Some(c)) => c,
        Ok(None) => return (t, out),
        Err(e) => {
            out.push(Finding::new(
                Severity::Error,
                "music.pick_failed",
                e.to_string(),
                Suggestion::field(
                    format!("use one of the moods: {}", library.main_moods().join(", ")),
                    "/music/pick/mood",
                ),
            ));
            return (t, out);
        }
    };
    out.push(Finding::new(
        Severity::Info,
        "music.picked",
        format!(
            "music: `{}` ({}) - {}",
            choice.track.id,
            choice.track.source(),
            choice.reason
        ),
        Suggestion::text(
            "to keep this track whatever else changes, replace music.pick with music.source",
        ),
    ));
    if let (Some(d), Some(e)) = (choice.energy_distance(), pick.energy) {
        if d > FAR_ENERGY {
            out.push(Finding::new(
                Severity::Warning,
                "music.energy_far",
                format!(
                    "no `{}` track is near energy {e:.2}: the closest, `{}`, has {:.2}",
                    pick.mood.trim(),
                    choice.track.id,
                    choice.track.energy
                ),
                Suggestion::field(
                    "change the energy or the mood, or add a track that fits",
                    "/music/pick/energy",
                ),
            ));
        }
    }
    let video_ms = timeline.total_duration_ms();
    if let Some(len) = choice.track.duration_ms {
        if len < video_ms && !choice.track.loopable {
            out.push(Finding::new(
                Severity::Warning,
                "music.too_short",
                format!(
                    "`{}` is {} long and does not loop seamlessly, but the video is {}: the restart will be audible",
                    choice.track.id,
                    secs(len),
                    secs(video_ms)
                ),
                Suggestion::field(
                    format!("exclude it (music.pick.exclude: [\"{}\"]) or pick another mood", choice.track.id),
                    "/music/pick/exclude",
                ),
            ));
        }
    }
    (t, out)
}
