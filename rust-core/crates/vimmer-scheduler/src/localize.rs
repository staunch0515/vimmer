//! Asset files that live outside the renderer's public dir (a Studio project's `assets/`, a file
//! next to the timeline).
//!
//! Everything after this pass reads assets as paths relative to `public_dir`: the scheduler joins
//! them for ffmpeg, and Remotion serves only that folder. So a file found in one of
//! [`SchedulerConfig::asset_dirs`] is *staged*: hard-linked (copied when a link is not possible)
//! to `public/generated/project/<content key>/<file name>`, and the timeline's path is rewritten
//! to that place. A path that is not found in an asset dir is left exactly as it is, so a
//! timeline whose files are all in `public/` goes through unchanged.
//!
//! The staged name depends on the file's content only, so the same file is staged once however
//! many renders, projects or copies of a project use it, and the caches keyed by path (images,
//! video clips, beats) keep hitting. `generated/project/` can be deleted at any time; the next
//! render stages what it needs again.

use std::collections::HashMap;
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};

use vimmer_dsl::{LayerContent, VideoTimeline};

use crate::run::{SchedulerConfig, SchedulerError};

/// Where staged files go, under the public dir.
pub const STAGE_DIR: &str = "generated/project";

/// Calls `f` on every asset path of the timeline: scene images, audio, video clips, layer
/// images, icons and clips, sound effects, the music file, brand fonts and the logo.
pub fn for_each_asset_path(timeline: &mut VideoTimeline, f: &mut dyn FnMut(&mut String)) {
    for scene in &mut timeline.scenes {
        if let Some(image) = &mut scene.image {
            f(image);
        }
        if let Some(audio) = &mut scene.audio {
            f(&mut audio.source);
        }
        if let Some(video) = &mut scene.video {
            f(&mut video.src);
        }
        for layer in &mut scene.layers {
            match &mut layer.content {
                LayerContent::Image(l) => f(&mut l.src),
                LayerContent::Icon(l) => f(&mut l.src),
                LayerContent::Video(l) => f(&mut l.src),
                _ => {}
            }
        }
        for fx in &mut scene.sfx {
            f(&mut fx.src);
        }
    }
    if let Some(source) = timeline.music.as_mut().and_then(|m| m.source.as_mut()) {
        f(source);
    }
    if let Some(brand) = &mut timeline.brand {
        for font in [&mut brand.fonts.heading, &mut brand.fonts.body] {
            if let Some(src) = font.as_mut().and_then(|f| f.src.as_mut()) {
                f(src);
            }
        }
        if let Some(logo) = &mut brand.logo {
            f(&mut logo.src);
        }
    }
}

/// True for a path this pass may resolve: relative, not empty, not a URL.
pub fn is_local_relative(path: &str) -> bool {
    !path.is_empty()
        && !path.starts_with("http://")
        && !path.starts_with("https://")
        && !Path::new(path).is_absolute()
}

/// The file `rel` names in the asset dirs, or `None` when the public dir's file (or nothing)
/// should be used. With `asset_dirs_first` an asset dir wins over the public dir; without it the
/// asset dirs only supply what the public dir lacks.
pub fn find_in_asset_dirs(rel: &str, cfg: &SchedulerConfig) -> Option<PathBuf> {
    if !is_local_relative(rel) || cfg.asset_dirs.is_empty() {
        return None;
    }
    if !cfg.asset_dirs_first && cfg.public_dir.join(rel).is_file() {
        return None;
    }
    cfg.asset_dirs
        .iter()
        .map(|dir| dir.join(rel))
        .find(|p| p.is_file())
}

/// Stages every asset found in `cfg.asset_dirs` and rewrites its path. Returns how many
/// different files were staged or reused.
pub fn stage_assets(
    timeline: &mut VideoTimeline,
    cfg: &SchedulerConfig,
    notes: &mut Vec<String>,
) -> Result<usize, SchedulerError> {
    if cfg.asset_dirs.is_empty() {
        return Ok(0);
    }
    let mut staged: HashMap<String, String> = HashMap::new();
    let mut failure: Option<SchedulerError> = None;
    for_each_asset_path(timeline, &mut |path| {
        if failure.is_some() {
            return;
        }
        if let Some(done) = staged.get(path.as_str()) {
            *path = done.clone();
            return;
        }
        let Some(found) = find_in_asset_dirs(path, cfg) else {
            return;
        };
        if cfg.asset_dirs_first && cfg.public_dir.join(path.as_str()).is_file() {
            notes.push(format!(
                "assets: `{path}` is taken from {} (a file of the same path in the renderer's public folder is not used)",
                found.display()
            ));
        }
        match stage_file(&found, &cfg.public_dir) {
            Ok(public_rel) => {
                staged.insert(path.clone(), public_rel.clone());
                *path = public_rel;
            }
            Err(e) => failure = Some(SchedulerError::Io(e)),
        }
    });
    match failure {
        Some(e) => Err(e),
        None => Ok(staged.len()),
    }
}

/// Links or copies `source` into the stage dir and returns its public-relative path.
pub fn stage_file(source: &Path, public_dir: &Path) -> std::io::Result<String> {
    let size = fs::metadata(source)?.len();
    let key = format!("{:016x}-{size:x}", content_hash(source)?);
    let name = source
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "file".into());
    let rel = format!("{STAGE_DIR}/{key}/{name}");
    let target = public_dir.join(&rel);
    if fs::metadata(&target).is_ok_and(|m| m.is_file() && m.len() == size) {
        return Ok(rel);
    }
    if let Some(dir) = target.parent() {
        fs::create_dir_all(dir)?;
    }
    let _ = fs::remove_file(&target);
    // Another render may stage the same file at the same moment: go through a private name.
    let tmp = target.with_file_name(format!(".{name}.{}.tmp", std::process::id()));
    let _ = fs::remove_file(&tmp);
    if fs::hard_link(source, &tmp).is_err() {
        fs::copy(source, &tmp)?;
    }
    fs::rename(&tmp, &target).inspect_err(|_| {
        let _ = fs::remove_file(&tmp);
    })?;
    Ok(rel)
}

/// 64-bit FNV-1a of the file's bytes.
fn content_hash(path: &Path) -> std::io::Result<u64> {
    let mut file = fs::File::open(path)?;
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    let mut buf = vec![0u8; 1 << 16];
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            return Ok(hash);
        }
        for b in &buf[..n] {
            hash ^= u64::from(*b);
            hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("vimmer-localize-{}-{name}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn write(path: &Path, body: &[u8]) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, body).unwrap();
    }

    const TIMELINE: &str = r##"{"fps":30,"resolution":{"width":1920,"height":1080},
        "scenes":[
          {"id":"a","duration_ms":1000,"image":"assets/pic.png","audio":{"source":"assets/voice.wav"},
           "layers":[{"id":"l","kind":"image","src":"assets/pic.png","x":0,"y":0,"width":0.5,"height":0.5}],
           "sfx":[{"src":"assets/sfx/pop.wav","at_ms":0}]},
          {"id":"b","duration_ms":1000,"image":"https://example.com/x.png","video":{"src":"assets/clips/c.mp4"}}],
        "music":{"source":"assets/music/m.mp3"},
        "brand":{"fonts":{"heading":{"family":"D","src":"assets/fonts/d.woff2"}},"logo":{"src":"assets/logo.png"}}}"##;

    fn timeline() -> VideoTimeline {
        serde_json::from_str(TIMELINE).unwrap()
    }

    fn cfg(root: &Path) -> SchedulerConfig {
        SchedulerConfig::new(root.join("work"), root.join("out.mp4"), root.join("remotion"))
    }

    #[test]
    fn every_kind_of_asset_path_is_visited() {
        let mut t = timeline();
        let mut seen = Vec::new();
        for_each_asset_path(&mut t, &mut |p| seen.push(p.clone()));
        assert_eq!(
            seen,
            [
                "assets/pic.png",
                "assets/voice.wav",
                "assets/pic.png",
                "assets/sfx/pop.wav",
                "https://example.com/x.png",
                "assets/clips/c.mp4",
                "assets/music/m.mp3",
                "assets/fonts/d.woff2",
                "assets/logo.png"
            ]
        );
    }

    #[test]
    fn without_asset_dirs_or_without_a_hit_nothing_changes() {
        let root = scratch("nohit");
        let before = timeline();
        let mut t = before.clone();
        let mut notes = Vec::new();
        let mut c = cfg(&root);
        assert_eq!(stage_assets(&mut t, &c, &mut notes).unwrap(), 0);
        c.asset_dirs = vec![root.join("fixtures")];
        assert_eq!(stage_assets(&mut t, &c, &mut notes).unwrap(), 0);
        c.asset_dirs_first = true;
        assert_eq!(stage_assets(&mut t, &c, &mut notes).unwrap(), 0);
        assert_eq!(t, before);
        assert!(notes.is_empty());
        assert!(!c.public_dir.join(STAGE_DIR).exists(), "nothing is created without a hit");
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn a_plain_timeline_keeps_public_files_and_only_fills_missing_ones() {
        let root = scratch("fill");
        let mut c = cfg(&root);
        let local = root.join("mine");
        c.asset_dirs = vec![local.clone()];
        // In both places: public wins. Only next to the timeline: staged.
        write(&c.public_dir.join("assets/pic.png"), b"public");
        write(&local.join("assets/pic.png"), b"local");
        write(&local.join("assets/voice.wav"), b"voice");
        let mut t = timeline();
        let mut notes = Vec::new();
        assert_eq!(stage_assets(&mut t, &c, &mut notes).unwrap(), 1);
        assert_eq!(t.scenes[0].image.as_deref(), Some("assets/pic.png"));
        let voice = t.scenes[0].audio.as_ref().unwrap().source.clone();
        assert!(voice.starts_with("generated/project/") && voice.ends_with("/voice.wav"), "{voice}");
        assert_eq!(fs::read(c.public_dir.join(&voice)).unwrap(), b"voice");
        assert!(notes.is_empty());
        assert_eq!(t.music.as_ref().unwrap().source.as_deref(), Some("assets/music/m.mp3"));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn a_project_file_wins_over_public_with_a_note_and_is_staged_once() {
        let root = scratch("project");
        let mut c = cfg(&root);
        let project = root.join("Project");
        c.asset_dirs = vec![project.clone()];
        c.asset_dirs_first = true;
        write(&c.public_dir.join("assets/pic.png"), b"public");
        write(&project.join("assets/pic.png"), b"project");
        write(&project.join("assets/logo.png"), b"logo");
        let mut t = timeline();
        let mut notes = Vec::new();
        assert_eq!(stage_assets(&mut t, &c, &mut notes).unwrap(), 2);
        let image = t.scenes[0].image.clone().unwrap();
        assert_eq!(fs::read(c.public_dir.join(&image)).unwrap(), b"project");
        let LayerContent::Image(layer) = &t.scenes[0].layers[0].content else { panic!() };
        assert_eq!(layer.src, image, "the same file is staged once");
        assert_eq!(notes.len(), 1, "{notes:?}");
        assert!(notes[0].contains("assets/pic.png"), "{notes:?}");
        // URLs and files that exist nowhere in the project are left alone.
        assert_eq!(t.scenes[1].image.as_deref(), Some("https://example.com/x.png"));
        assert_eq!(t.scenes[1].video.as_ref().unwrap().src, "assets/clips/c.mp4");

        // A second render reuses the staged file; a copy of the project maps to the same place.
        let copy = root.join("Project copy");
        write(&copy.join("assets/pic.png"), b"project");
        c.asset_dirs = vec![copy];
        let mut again = timeline();
        stage_assets(&mut again, &c, &mut Vec::new()).unwrap();
        assert_eq!(again.scenes[0].image.as_deref(), Some(image.as_str()));
        // Changed content gets a new place.
        write(&project.join("assets/pic.png"), b"edited!");
        c.asset_dirs = vec![project];
        let mut edited = timeline();
        stage_assets(&mut edited, &c, &mut Vec::new()).unwrap();
        assert_ne!(edited.scenes[0].image.as_deref(), Some(image.as_str()));
        let _ = fs::remove_dir_all(root);
    }
}
