//! A project's render history and its thumbnail.
//!
//! Renders go to `<project>/renders/` unless the user picks another place. Each finished render
//! is written to `project.json` with its time, options, size and length. The thumbnail is a
//! frame of the latest render; without a render, the first scene's image; without that, no file
//! (the Home screen then draws a title card).

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::UNIX_EPOCH;

use serde::Serialize;
use serde_json::Value;
use tauri::AppHandle;

use crate::paths::is_local_relative;
use crate::studio::{
    blocking, now_ms, read_meta, studio, summary, write_meta, ProjectMeta, ProjectSummary, RenderRecord, Studio,
    RENDERS_DIR, THUMB_FILE, TIMELINE_FILE,
};

/// A render as the Renders panel shows it.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RenderView {
    #[serde(flatten)]
    pub record: RenderRecord,
    /// Absolute path of the file.
    pub path: String,
    /// The file is no longer there (moved or deleted outside the app).
    pub missing: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RendersChanged {
    pub project: ProjectSummary,
    pub renders: Vec<RenderView>,
}

/// Where a record's file is: inside the project, or wherever the user saved it.
pub fn render_path(dir: &Path, record: &RenderRecord) -> PathBuf {
    let file = Path::new(&record.file);
    if file.is_absolute() {
        file.to_path_buf()
    } else {
        dir.join(file)
    }
}

pub fn views(dir: &Path, meta: &ProjectMeta) -> Vec<RenderView> {
    // Newest first.
    meta.renders
        .iter()
        .rev()
        .map(|record| {
            let path = render_path(dir, record);
            RenderView { record: record.clone(), missing: !path.is_file(), path: path.display().to_string() }
        })
        .collect()
}

/// `YYYYMMDD-HHMMSS` in local time (from `date`, so no time zone code here); the unix time when
/// that fails.
fn stamp() -> String {
    Command::new("/bin/date")
        .arg("+%Y%m%d-%H%M%S")
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| (now_ms() / 1000).to_string())
}

/// A free `renders/<stamp>.mp4` (`-2`, `-3`, … within the same second).
pub fn default_output(dir: &Path) -> PathBuf {
    free_name(&dir.join(RENDERS_DIR), &stamp(), "mp4")
}

fn free_name(folder: &Path, stem: &str, ext: &str) -> PathBuf {
    let first = folder.join(format!("{stem}.{ext}"));
    if !first.exists() {
        return first;
    }
    (2..).map(|n| folder.join(format!("{stem}-{n}.{ext}"))).find(|p| !p.exists()).expect("some name is free")
}

fn video_duration_ms(ffmpeg: &Path, file: &Path) -> u64 {
    vimmer_media::probe_video(ffmpeg, file).map(|info| info.duration_ms).unwrap_or(0)
}

/// Adds a finished render to the project's history and refreshes the thumbnail.
pub fn record_render(
    studio: &Studio,
    dir: &Path,
    output: &Path,
    options: Value,
    imported: bool,
) -> Result<RenderRecord, String> {
    let bytes = fs::metadata(output).map_err(|e| format!("{}: {e}", output.display()))?.len();
    let file = match output.strip_prefix(dir) {
        Ok(rel) => rel.display().to_string(),
        Err(_) => output.display().to_string(),
    };
    let mut meta = read_meta(dir)?;
    let record = RenderRecord {
        id: format!("{:x}-{}", now_ms(), meta.renders.len() + 1),
        file,
        finished_ms: now_ms(),
        bytes,
        duration_ms: video_duration_ms(&studio.ffmpeg, output),
        options,
        imported,
    };
    // The same file rendered again replaces its old entry.
    meta.renders.retain(|r| r.file != record.file);
    meta.renders.push(record.clone());
    meta.updated_ms = now_ms();
    refresh_thumbnail(studio, dir, &mut meta);
    write_meta(dir, &meta)?;
    Ok(record)
}

impl Studio {
    pub fn renders(&self, dir: &str) -> Result<Vec<RenderView>, String> {
        let dir = self.project(dir)?;
        Ok(views(&dir, &read_meta(&dir)?))
    }

    /// Removes a render from the history; a file inside the project goes to the Trash. A file the
    /// user saved elsewhere is left where it is.
    pub fn trash_render(&self, dir: &str, id: &str) -> Result<RendersChanged, String> {
        let dir = self.project(dir)?;
        let mut meta = read_meta(&dir)?;
        let index = meta.renders.iter().position(|r| r.id == id).ok_or("That render is not in the history.")?;
        let record = meta.renders[index].clone();
        let path = render_path(&dir, &record);
        if path.is_file() && path.starts_with(&dir) {
            (self.trash)(&path)?;
        }
        meta.renders.remove(index);
        refresh_thumbnail(self, &dir, &mut meta);
        write_meta(&dir, &meta)?;
        Ok(RendersChanged { project: summary(&dir, &meta), renders: views(&dir, &meta) })
    }

    /// Copies an existing video into `renders/` and adds it to the history.
    pub fn add_existing_render(&self, dir: &str, source: &str) -> Result<RendersChanged, String> {
        let dir = self.project(dir)?;
        let source = Path::new(source);
        if vimmer_media::probe_video(&self.ffmpeg, source).is_err() {
            return Err(format!("{} is not a video ffmpeg can read.", source.display()));
        }
        let folder = dir.join(RENDERS_DIR);
        fs::create_dir_all(&folder).map_err(|e| format!("Could not create {}: {e}", folder.display()))?;
        let stem = source.file_stem().and_then(|s| s.to_str()).unwrap_or("video");
        let ext = source.extension().and_then(|s| s.to_str()).unwrap_or("mp4");
        let target = free_name(&folder, stem, ext);
        fs::copy(source, &target).map_err(|e| format!("Could not copy {}: {e}", source.display()))?;
        record_render(self, &dir, &target, Value::Null, true)?;
        let meta = read_meta(&dir)?;
        Ok(RendersChanged { project: summary(&dir, &meta), renders: views(&dir, &meta) })
    }
}

/// What the thumbnail should be made from: `render:<id>`, `image:<path>:<bytes>` or nothing.
fn thumbnail_source(studio: &Studio, dir: &Path, meta: &ProjectMeta) -> Option<(String, PathBuf, Option<u64>)> {
    if let Some(record) = meta.renders.iter().rev().find(|r| render_path(dir, r).is_file()) {
        return Some((format!("render:{}", record.id), render_path(dir, record), Some(record.duration_ms)));
    }
    let raw = fs::read_to_string(dir.join(TIMELINE_FILE)).ok()?;
    let timeline: Value = serde_json::from_str(&raw).ok()?;
    let image = timeline.get("scenes")?.as_array()?.iter().find_map(|s| s.get("image")?.as_str())?;
    if !is_local_relative(image) {
        return None;
    }
    let file = [dir.join(image), studio.public_dir.join(image)].into_iter().find(|p| p.is_file())?;
    let bytes = fs::metadata(&file).ok()?.len();
    Some((format!("image:{image}:{bytes}"), file, None))
}

/// Brings `thumb.jpg` up to date. Returns whether `meta` changed (the caller writes it).
pub fn refresh_thumbnail(studio: &Studio, dir: &Path, meta: &mut ProjectMeta) -> bool {
    let thumb = dir.join(THUMB_FILE);
    let wanted = thumbnail_source(studio, dir, meta);
    let key = wanted.as_ref().map(|(key, _, _)| key.clone());
    // `!key`: making it failed before (no ffmpeg, an SVG); do not try on every save.
    let failed_key = key.as_ref().map(|k| format!("!{k}"));
    let up_to_date = match &key {
        None => meta.thumb_source.is_none() && !thumb.exists(),
        Some(_) => (meta.thumb_source == key && thumb.is_file()) || meta.thumb_source == failed_key,
    };
    if up_to_date {
        return false;
    }
    let made = wanted.is_some_and(|(_, file, duration_ms)| make_thumbnail(&studio.ffmpeg, &file, duration_ms, &thumb));
    if !made {
        let _ = fs::remove_file(&thumb);
    }
    let next = if made { key } else { failed_key };
    let changed = meta.thumb_source != next;
    meta.thumb_source = next;
    changed
}

/// One 640-wide JPEG frame of a video (at a tenth of its length) or of an image.
fn make_thumbnail(ffmpeg: &Path, source: &Path, duration_ms: Option<u64>, thumb: &Path) -> bool {
    let tmp = thumb.with_file_name(format!(".thumb-{}.jpg", std::process::id()));
    let mut cmd = Command::new(ffmpeg);
    cmd.args(["-y", "-hide_banner", "-nostdin", "-loglevel", "error"]);
    if let Some(ms) = duration_ms {
        cmd.arg("-ss").arg(format!("{:.3}", ms as f64 / 10_000.0));
    }
    cmd.arg("-i").arg(source).args(["-frames:v", "1", "-vf", "scale=640:-2", "-q:v", "4"]).arg(&tmp);
    let ok = cmd.output().is_ok_and(|o| o.status.success()) && fs::metadata(&tmp).is_ok_and(|m| m.len() > 0);
    if ok && fs::rename(&tmp, thumb).is_ok() {
        return true;
    }
    let _ = fs::remove_file(&tmp);
    false
}

/// The change time of a file in unix ms.
pub fn modified_ms(path: &Path) -> u64 {
    fs::metadata(path)
        .ok()
        .and_then(|m| m.modified().ok())
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

#[tauri::command]
pub async fn list_renders(app: AppHandle, dir: String) -> Result<Vec<RenderView>, String> {
    let studio = studio(&app)?;
    blocking(move || studio.renders(&dir)).await
}

#[tauri::command]
pub async fn trash_render(app: AppHandle, dir: String, id: String) -> Result<RendersChanged, String> {
    let studio = studio(&app)?;
    blocking(move || studio.trash_render(&dir, &id)).await
}

#[tauri::command]
pub async fn add_existing_render(app: AppHandle, dir: String, source: String) -> Result<RendersChanged, String> {
    let studio = studio(&app)?;
    blocking(move || studio.add_existing_render(&dir, &source)).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::studio::tests::Fixture;

    fn have_ffmpeg(studio: &Studio) -> bool {
        Command::new(&studio.ffmpeg).arg("-version").output().is_ok_and(|o| o.status.success())
    }

    /// A one-second test video and a small PNG, made by ffmpeg.
    fn make_media(studio: &Studio, video: &Path, image: &Path) {
        let run = |args: &[&str], out: &Path| {
            let ok = Command::new(&studio.ffmpeg)
                .args(["-y", "-loglevel", "error", "-f", "lavfi", "-i"])
                .args(args)
                .arg(out)
                .status()
                .unwrap()
                .success();
            assert!(ok, "ffmpeg could not make {}", out.display());
        };
        run(&["testsrc=size=320x180:rate=10:duration=1", "-pix_fmt", "yuv420p"], video);
        run(&["color=c=red:size=200x100", "-frames:v", "1"], image);
    }

    #[test]
    fn a_render_is_recorded_listed_and_trashed() {
        let fx = Fixture::new("renders");
        if !have_ffmpeg(&fx.studio) {
            eprintln!("skipped: no ffmpeg");
            return;
        }
        let made = fx.studio.create("Film", None, "landscape").unwrap();
        let dir = PathBuf::from(&made.dir);
        assert!(made.thumb.is_none(), "a blank project has no thumbnail file");

        let out = default_output(&dir);
        assert!(out.starts_with(dir.join("renders")) && out.extension().unwrap() == "mp4", "{}", out.display());
        let image = dir.join("assets/pic.png");
        make_media(&fx.studio, &out, &image);
        assert_ne!(default_output(&dir), out, "a taken name is not reused");

        let options = serde_json::json!({"tts": "say"});
        let record = record_render(&fx.studio, &dir, &out, options.clone(), false).unwrap();
        assert!(record.file.starts_with("renders/"), "{}", record.file);
        assert_eq!(record.bytes, fs::metadata(&out).unwrap().len());
        assert!((900..=1100).contains(&record.duration_ms), "{}", record.duration_ms);
        assert_eq!(record.options, options);

        let listed = fx.studio.renders(&made.dir).unwrap();
        assert_eq!(listed.len(), 1);
        assert!(!listed[0].missing);
        assert_eq!(listed[0].path, out.display().to_string());
        let meta = read_meta(&dir).unwrap();
        assert_eq!(meta.thumb_source, Some(format!("render:{}", record.id)));
        assert!(dir.join(THUMB_FILE).is_file(), "the thumbnail is a frame of the render");
        assert!(fx.studio.list()[0].thumb.is_some());

        // An existing video is copied in and marked as imported; newest first.
        let old = fx.root.join("old.mp4");
        fs::copy(&out, &old).unwrap();
        let changed = fx.studio.add_existing_render(&made.dir, &old.display().to_string()).unwrap();
        assert_eq!(changed.renders.len(), 2);
        assert!(changed.renders[0].record.imported);
        assert_eq!(changed.renders[0].record.file, "renders/old.mp4");
        assert!(old.is_file(), "the original stays");
        assert!(fx.studio.add_existing_render(&made.dir, &image.display().to_string()).is_err());

        // Deleting moves the file to the Trash and drops the entry.
        let after = fx.studio.trash_render(&made.dir, &record.id).unwrap();
        assert_eq!(after.renders.len(), 1);
        assert!(!out.exists());
        assert_eq!(fx.trashed().len(), 1);
        assert!(fx.studio.trash_render(&made.dir, &record.id).is_err());

        // A file removed outside the app shows as missing; its entry can still be removed.
        fs::remove_file(dir.join("renders/old.mp4")).unwrap();
        let listed = fx.studio.renders(&made.dir).unwrap();
        assert!(listed[0].missing);
        let id = listed[0].record.id.clone();
        let empty = fx.studio.trash_render(&made.dir, &id).unwrap();
        assert!(empty.renders.is_empty());
        assert!(empty.project.thumb.is_none(), "no render and no scene image: no thumbnail");
    }

    #[test]
    fn without_a_render_the_thumbnail_is_the_first_scene_image() {
        let fx = Fixture::new("thumb");
        if !have_ffmpeg(&fx.studio) {
            eprintln!("skipped: no ffmpeg");
            return;
        }
        let made = fx.studio.create("Pic", None, "landscape").unwrap();
        let dir = PathBuf::from(&made.dir);
        make_media(&fx.studio, &fx.root.join("v.mp4"), &dir.join("assets/pic.png"));
        let raw = fs::read_to_string(dir.join(TIMELINE_FILE)).unwrap();
        let with_image = raw.replace(r#""image": null"#, r#""image": "assets/pic.png""#);
        assert_ne!(raw, with_image);
        let saved = fx.studio.save(&made.dir, &with_image).unwrap();
        assert!(saved.thumb.is_some());
        let first = read_meta(&dir).unwrap().thumb_source.unwrap();
        assert!(first.starts_with("image:assets/pic.png:"), "{first}");
        // Saving again does not redo it.
        let version = saved.thumb_version;
        assert_eq!(fx.studio.save(&made.dir, &with_image).unwrap().thumb_version, version);
        // An image that cannot be read gives no thumbnail, and is not retried on every save.
        fs::write(dir.join("assets/broken.png"), b"not a png").unwrap();
        let broken = with_image.replace("assets/pic.png", "assets/broken.png");
        assert!(fx.studio.save(&made.dir, &broken).unwrap().thumb.is_none());
        assert!(read_meta(&dir).unwrap().thumb_source.unwrap().starts_with("!image:"));
        // Back to no image: no thumbnail, no source.
        assert!(fx.studio.save(&made.dir, &raw).unwrap().thumb.is_none());
        assert_eq!(read_meta(&dir).unwrap().thumb_source, None);
    }
}
