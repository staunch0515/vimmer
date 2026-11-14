//! The music library: tracks tagged with moods and an energy level, listed in
//! `<public>/assets/music/library/library.json`, and the deterministic choice of a track for a
//! `music.pick` ([`MusicLibrary::pick`], [`resolve_music_pick`]).
//!
//! Choosing: only tracks whose `moods` contain the wanted mood (case does not matter) and that
//! are not excluded count. With an `energy` target, the tracks closest to it win, the distance
//! counted in steps of 0.1 (so 0.15 and 0.2 are equally close to 0.2). Ties are broken by a hash
//! of the timeline ([`pick_seed`]) and the track id (rendezvous hashing): stable for one video,
//! different between videos, and adding a track only moves the videos that now prefer it.
//!
//! The only file access is [`MusicLibrary::load`], which reads the index and checks which
//! track files exist.

use std::collections::BTreeSet;
use std::fmt;

use serde::{Deserialize, Serialize};

use crate::model::{BeatGrid, MusicPick, VideoTimeline};

/// Where the library lives, relative to the renderer's public dir.
pub const LIBRARY_DIR: &str = "assets/music/library";
/// The library index, relative to the renderer's public dir.
pub const LIBRARY_INDEX: &str = "assets/music/library/library.json";
/// A chosen track further than this from the energy target is reported as a poor fit.
pub const FAR_ENERGY: f32 = 0.25;

fn default_true() -> bool {
    true
}

/// One track of the library.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LibraryTrack {
    /// Stable id, used in `music.pick.exclude` and in reports.
    pub id: String,
    /// File name relative to the library directory.
    pub file: String,
    #[serde(default)]
    pub title: String,
    /// Mood tags, e.g. `calm`, `upbeat`. The first is the main one.
    pub moods: Vec<String>,
    /// 0 (quiet, slow) .. 1 (intense).
    pub energy: f32,
    /// Tempo, when the track has a steady beat. Beat sync uses it as the beat grid.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bpm: Option<f32>,
    /// Time of the first beat in the file.
    #[serde(default)]
    pub beat_offset_ms: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<u64>,
    /// Whether the end runs seamlessly into the start (the render loops music under long videos).
    #[serde(default = "default_true")]
    pub loopable: bool,
    #[serde(default)]
    pub license: String,
    #[serde(default)]
    pub how_made: String,
}

impl LibraryTrack {
    /// Path relative to the renderer's public dir, as `music.source` takes it.
    pub fn source(&self) -> String {
        format!("{LIBRARY_DIR}/{}", self.file)
    }

    fn has_mood(&self, mood: &str) -> bool {
        self.moods
            .iter()
            .any(|m| m.trim().eq_ignore_ascii_case(mood))
    }
}

/// `library.json`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MusicLibrary {
    #[serde(default)]
    pub version: u32,
    pub tracks: Vec<LibraryTrack>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum LibraryError {
    /// `library.json` could not be read or parsed, or an entry is wrong.
    Invalid(String),
    /// No track has the mood.
    UnknownMood {
        mood: String,
        available: Vec<String>,
    },
    /// Tracks have the mood, but `exclude` names all of them.
    AllExcluded { mood: String, excluded: Vec<String> },
}

impl fmt::Display for LibraryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            LibraryError::Invalid(m) => write!(f, "music library: {m}"),
            LibraryError::UnknownMood { mood, available } => write!(
                f,
                "no music in the library has the mood `{mood}`; available moods: {}",
                available.join(", ")
            ),
            LibraryError::AllExcluded { mood, excluded } => write!(
                f,
                "every track with the mood `{mood}` is excluded ({}); remove one from music.pick.exclude",
                excluded.join(", ")
            ),
        }
    }
}

impl std::error::Error for LibraryError {}

/// The outcome of a pick.
#[derive(Debug, Clone, PartialEq)]
pub struct PickChoice {
    pub track: LibraryTrack,
    /// Every track that had the mood and was not excluded, best first: `(id, energy distance)`
    /// (distance `None` without an energy target).
    pub candidates: Vec<(String, Option<f32>)>,
    /// One line: which track and why.
    pub reason: String,
}

impl PickChoice {
    /// How far the chosen track is from the energy target, when one was given.
    pub fn energy_distance(&self) -> Option<f32> {
        self.candidates.first().and_then(|c| c.1)
    }
}

impl MusicLibrary {
    /// Parses and checks `library.json`.
    pub fn parse(json: &str) -> Result<Self, LibraryError> {
        let lib: MusicLibrary =
            serde_json::from_str(json).map_err(|e| LibraryError::Invalid(e.to_string()))?;
        lib.check()?;
        Ok(lib)
    }

    fn check(&self) -> Result<(), LibraryError> {
        let mut ids = BTreeSet::new();
        let bad =
            |id: &str, what: String| Err(LibraryError::Invalid(format!("track `{id}`: {what}")));
        for t in &self.tracks {
            if t.id.trim().is_empty() {
                return Err(LibraryError::Invalid("a track has an empty id".into()));
            }
            if !ids.insert(t.id.as_str()) {
                return bad(&t.id, "the id is used twice".into());
            }
            let f = t.file.trim();
            if f.is_empty() || f.starts_with('/') || f.contains("..") || f.contains("://") {
                return bad(
                    &t.id,
                    format!(
                        "file `{}` must be a name inside the library directory",
                        t.file
                    ),
                );
            }
            if t.moods.iter().all(|m| m.trim().is_empty()) {
                return bad(&t.id, "needs at least one mood".into());
            }
            if !(0.0..=1.0).contains(&t.energy) {
                return bad(
                    &t.id,
                    format!("energy {} must be between 0 and 1", t.energy),
                );
            }
            if let Some(bpm) = t.bpm {
                if !(30.0..=300.0).contains(&bpm) {
                    return bad(&t.id, format!("bpm {bpm} must be between 30 and 300"));
                }
            }
        }
        Ok(())
    }

    /// Every mood of every track, lowercase, sorted, once each.
    pub fn moods(&self) -> Vec<String> {
        let set: BTreeSet<String> = self
            .tracks
            .iter()
            .flat_map(|t| t.moods.iter())
            .map(|m| m.trim().to_lowercase())
            .filter(|m| !m.is_empty())
            .collect();
        set.into_iter().collect()
    }

    /// The main mood (first tag) of each track, sorted, once each: the moods worth offering.
    pub fn main_moods(&self) -> Vec<String> {
        let set: BTreeSet<String> = self
            .tracks
            .iter()
            .filter_map(|t| t.moods.first())
            .map(|m| m.trim().to_lowercase())
            .collect();
        set.into_iter().collect()
    }

    pub fn track(&self, id: &str) -> Option<&LibraryTrack> {
        self.tracks.iter().find(|t| t.id == id)
    }

    /// Chooses a track for `pick`; `seed` comes from [`pick_seed`].
    pub fn pick(&self, pick: &MusicPick, seed: u64) -> Result<PickChoice, LibraryError> {
        let mood = pick.mood.trim().to_lowercase();
        let with_mood: Vec<&LibraryTrack> =
            self.tracks.iter().filter(|t| t.has_mood(&mood)).collect();
        if with_mood.is_empty() {
            return Err(LibraryError::UnknownMood {
                mood,
                available: self.moods(),
            });
        }
        let mut ranked: Vec<(u32, u64, &LibraryTrack, Option<f32>)> = with_mood
            .iter()
            .filter(|t| !pick.exclude.iter().any(|x| x.trim() == t.id))
            .map(|t| {
                let dist = pick.energy.map(|e| (t.energy - e).abs());
                // Steps of 0.1; the small margin keeps 0.3 - 0.2 in the step of 0.1.
                let step = dist.map_or(0, |d| (d * 10.0 + 1e-4).floor() as u32);
                (step, mix(seed, &t.id), *t, dist)
            })
            .collect();
        if ranked.is_empty() {
            return Err(LibraryError::AllExcluded {
                mood,
                excluded: with_mood.iter().map(|t| t.id.clone()).collect(),
            });
        }
        ranked.sort_by_key(|a| (a.0, a.1));
        let best = ranked[0].2.clone();
        let ties = ranked.iter().filter(|r| r.0 == ranked[0].0).count();
        let mut reason = format!(
            "mood `{mood}`: {} of {} track(s) with that mood",
            best.id,
            with_mood.len()
        );
        if pick
            .exclude
            .iter()
            .any(|x| with_mood.iter().any(|t| t.id == x.trim()))
        {
            reason.push_str(&format!(" ({} excluded)", with_mood.len() - ranked.len()));
        }
        if let Some(e) = pick.energy {
            reason.push_str(&format!(
                "; energy {:.2} is the closest to the target {e:.2}",
                best.energy
            ));
        }
        if ties > 1 {
            reason.push_str(&format!(
                "; chosen among {ties} equally good tracks by this video's scenes (always the same for this video)"
            ));
        }
        Ok(PickChoice {
            track: best,
            candidates: ranked.iter().map(|r| (r.2.id.clone(), r.3)).collect(),
            reason,
        })
    }

    /// Reads `<public_dir>/assets/music/library/library.json`. Returns the tracks whose files
    /// exist, and the ids of those whose files are missing.
    pub fn load(public_dir: &std::path::Path) -> Result<(MusicLibrary, Vec<String>), LibraryError> {
        let lib = MusicLibrary::load_index(public_dir)?;
        Ok(lib.available(|t| public_dir.join(t.source()).is_file()))
    }

    /// Reads `library.json` as it is, missing files included.
    pub fn load_index(public_dir: &std::path::Path) -> Result<MusicLibrary, LibraryError> {
        let path = public_dir.join(LIBRARY_INDEX);
        let raw = std::fs::read_to_string(&path)
            .map_err(|e| LibraryError::Invalid(format!("cannot read {}: {e}", path.display())))?;
        MusicLibrary::parse(&raw)
    }

    /// The library without the tracks for which `exists` says the file is missing, and the
    /// ids of those tracks.
    pub fn available(&self, exists: impl Fn(&LibraryTrack) -> bool) -> (MusicLibrary, Vec<String>) {
        let (have, missing): (Vec<_>, Vec<_>) =
            self.tracks.iter().cloned().partition(|t| exists(t));
        (
            MusicLibrary {
                version: self.version,
                tracks: have,
            },
            missing.into_iter().map(|t| t.id).collect(),
        )
    }
}

/// FNV-1a over the bytes, then a SplitMix64 finish so close inputs spread out.
fn fnv(mut h: u64, bytes: &[u8]) -> u64 {
    for b in bytes {
        h ^= u64::from(*b);
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    h
}

fn finish(mut z: u64) -> u64 {
    z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    z ^ (z >> 31)
}

fn mix(seed: u64, id: &str) -> u64 {
    finish(fnv(seed, id.as_bytes()))
}

/// What a pick's tie-break depends on: the scene ids and the first scene's text. Editing later
/// scenes, timing or styling never changes the music; a different video (other scenes or
/// another opening line) usually gets a different track among equally good ones.
pub fn pick_seed(t: &VideoTimeline) -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for s in &t.scenes {
        h = fnv(h, s.id.as_bytes());
        h = fnv(h, &[0]);
    }
    if let Some(first) = t.scenes.first() {
        h = fnv(h, first.text.as_deref().unwrap_or("").trim().as_bytes());
    }
    finish(h)
}

/// Turns `music.pick` into `music.source`: picks the track, sets `source`, clears `pick`, and,
/// when beat sync is on without a hand-set grid, sets `music.beats` from the track's tempo.
/// `Ok(None)` when there is no pick.
pub fn resolve_music_pick(
    t: &mut VideoTimeline,
    library: &MusicLibrary,
) -> Result<Option<PickChoice>, LibraryError> {
    let seed = pick_seed(t);
    let Some(music) = t.music.as_mut() else {
        return Ok(None);
    };
    let Some(pick) = music.pick.clone() else {
        return Ok(None);
    };
    let choice = library.pick(&pick, seed)?;
    music.source = Some(choice.track.source());
    music.pick = None;
    if music.beat_sync_on().is_some() && music.beats.is_none() {
        if let Some(bpm) = choice.track.bpm {
            music.beats = Some(BeatGrid {
                bpm,
                offset_ms: choice.track.beat_offset_ms,
            });
        }
    }
    Ok(Some(choice))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn track(id: &str, moods: &[&str], energy: f32) -> LibraryTrack {
        LibraryTrack {
            id: id.into(),
            file: format!("{id}.m4a"),
            title: String::new(),
            moods: moods.iter().map(|m| m.to_string()).collect(),
            energy,
            bpm: Some(100.0),
            beat_offset_ms: 0,
            duration_ms: Some(48_000),
            loopable: true,
            license: String::new(),
            how_made: String::new(),
        }
    }

    fn lib() -> MusicLibrary {
        MusicLibrary {
            version: 1,
            tracks: vec![
                track("calm-a", &["calm"], 0.1),
                track("calm-b", &["calm"], 0.15),
                track("warm", &["warm", "calm"], 0.35),
                track("up", &["upbeat"], 0.8),
            ],
        }
    }

    fn pick(mood: &str, energy: Option<f32>) -> MusicPick {
        MusicPick {
            mood: mood.into(),
            energy,
            exclude: vec![],
        }
    }

    #[test]
    fn the_bundled_library_parses() {
        let raw =
            include_str!("../../../../remotion-renderer/public/assets/music/library/library.json");
        let lib = MusicLibrary::parse(raw).unwrap();
        assert_eq!(lib.tracks.len(), 6);
        for mood in ["calm", "upbeat", "tense", "warm", "epic", "playful"] {
            assert!(lib.main_moods().contains(&mood.to_string()), "{mood}");
        }
    }

    #[test]
    fn same_seed_same_track_and_seeds_spread_over_ties() {
        let l = lib();
        let p = pick("calm", None);
        let a = l.pick(&p, 42).unwrap();
        assert_eq!(a, l.pick(&p, 42).unwrap());
        assert_eq!(a.candidates.len(), 3);
        let chosen: BTreeSet<String> = (0..50).map(|s| l.pick(&p, s).unwrap().track.id).collect();
        assert_eq!(chosen.len(), 3, "{chosen:?}");
    }

    #[test]
    fn energy_decides_before_the_hash() {
        let l = lib();
        for seed in 0..20 {
            assert_eq!(
                l.pick(&pick("calm", Some(0.4)), seed).unwrap().track.id,
                "warm"
            );
            // 0.1 and 0.15 are in the same 0.1 step from 0.12: either may win, never `warm`.
            let id = l.pick(&pick("Calm ", Some(0.12)), seed).unwrap().track.id;
            assert!(id.starts_with("calm-"), "{id}");
        }
    }

    #[test]
    fn exclude_and_errors() {
        let l = lib();
        let mut p = pick("upbeat", None);
        assert_eq!(l.pick(&p, 1).unwrap().track.id, "up");
        p.exclude = vec!["up".into()];
        assert!(matches!(
            l.pick(&p, 1),
            Err(LibraryError::AllExcluded { .. })
        ));
        let e = l.pick(&pick("sad", None), 1).unwrap_err();
        assert_eq!(
            e.to_string(),
            "no music in the library has the mood `sad`; available moods: calm, upbeat, warm"
        );
    }

    #[test]
    fn bad_entries_are_rejected() {
        for bad in [
            r#"{"tracks":[{"id":"a","file":"../x.m4a","moods":["calm"],"energy":0.1}]}"#,
            r#"{"tracks":[{"id":"a","file":"x.m4a","moods":[],"energy":0.1}]}"#,
            r#"{"tracks":[{"id":"a","file":"x.m4a","moods":["calm"],"energy":2}]}"#,
            r#"{"tracks":[{"id":"a","file":"x.m4a","moods":["c"],"energy":0},{"id":"a","file":"y.m4a","moods":["c"],"energy":0}]}"#,
        ] {
            assert!(MusicLibrary::parse(bad).is_err(), "{bad}");
        }
        let ok = MusicLibrary::parse(
            r#"{"tracks":[{"id":"a","file":"x.m4a","moods":["calm"],"energy":0.1}]}"#,
        )
        .unwrap();
        assert!(ok.tracks[0].loopable);
        assert_eq!(ok.tracks[0].bpm, None);
    }

    #[test]
    fn resolving_sets_source_and_beats_only_with_beat_sync() {
        let json = r#"{"fps":30,"resolution":{"width":1920,"height":1080},"scenes":[
            {"id":"a","duration_ms":3000,"text":"hi","highlights":[],"image":null,"motion":"static","audio":null}],
            "music":{"pick":{"mood":"upbeat"},"volume":0.3}}"#;
        let mut t: VideoTimeline = serde_json::from_str(json).unwrap();
        let mut synced = t.clone();
        let choice = resolve_music_pick(&mut t, &lib()).unwrap().unwrap();
        let m = t.music.as_ref().unwrap();
        assert_eq!(m.source.as_deref(), Some("assets/music/library/up.m4a"));
        assert!(m.pick.is_none() && m.beats.is_none());
        assert_eq!(choice.track.id, "up");
        synced.music.as_mut().unwrap().beat_sync = Some(Default::default());
        resolve_music_pick(&mut synced, &lib()).unwrap();
        assert_eq!(synced.music.unwrap().beats.unwrap().bpm, 100.0);
        // Nothing to do without a pick.
        assert_eq!(resolve_music_pick(&mut t, &lib()).unwrap(), None);
    }

    #[test]
    fn seed_ignores_later_scenes_text() {
        let json = |second: &str| {
            format!(
                r#"{{"fps":30,"resolution":{{"width":1920,"height":1080}},"scenes":[
                {{"id":"a","duration_ms":3000,"text":"open","highlights":[],"image":null,"motion":"static","audio":null}},
                {{"id":"b","duration_ms":3000,"text":"{second}","highlights":[],"image":null,"motion":"static","audio":null}}]}}"#
            )
        };
        let a: VideoTimeline = serde_json::from_str(&json("one")).unwrap();
        let b: VideoTimeline = serde_json::from_str(&json("two")).unwrap();
        assert_eq!(pick_seed(&a), pick_seed(&b));
        let mut c = a.clone();
        c.scenes[0].text = Some("another video".into());
        assert_ne!(pick_seed(&a), pick_seed(&c));
    }
}
