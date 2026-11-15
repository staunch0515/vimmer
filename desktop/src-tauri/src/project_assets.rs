//! The media files of a project: what is there, what uses it, importing and removing.
//!
//! Imported files are copied into `<project>/assets/`; the timeline names them by their
//! project-relative path (`assets/pic.png`). A file that is already in the project (same bytes,
//! any name) is not copied again. Removing means moving to the Trash, and only files the timeline
//! does not use can be removed.

use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::io::Read;
use std::path::{Component, Path, PathBuf};
use std::sync::{Mutex, OnceLock};

use serde::Serialize;
use serde_json::Value;
use tauri::AppHandle;
use vimmer_media::AudioProbe;

use crate::paths::for_each_asset;
use crate::renders::modified_ms;
use crate::studio::{blocking, studio, Studio, ASSETS_DIR, PROJECT_FILE, RENDERS_DIR, SOURCE_DIR, THUMB_FILE, TIMELINE_FILE};

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AssetInfo {
    /// Project-relative path, as the timeline names the file.
    pub path: String,
    pub name: String,
    /// `image`, `icon` (SVG), `video`, `audio` or `font`.
    pub kind: &'static str,
    pub bytes: u64,
    pub modified_ms: u64,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub duration_ms: Option<u64>,
    /// Scene ids (and `Music`, `Brand`) that use the file in the timeline given.
    pub used_by: Vec<String>,
    /// Absolute path, for the preview.
    pub file: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportedAsset {
    pub source: String,
    /// Project-relative path of the file in the project.
    pub path: String,
    /// The file was already in the project; nothing was copied.
    pub reused: bool,
}

/// The kind of a file by its extension; `None` for anything a timeline cannot use.
pub fn kind_of(path: &Path) -> Option<&'static str> {
    let ext = path.extension()?.to_str()?.to_ascii_lowercase();
    Some(match ext.as_str() {
        "png" | "jpg" | "jpeg" | "webp" | "gif" => "image",
        "svg" => "icon",
        "mp4" | "mov" | "m4v" | "webm" => "video",
        "wav" | "mp3" | "m4a" | "aac" | "aif" | "aiff" => "audio",
        "ttf" | "otf" | "woff" | "woff2" => "font",
        _ => return None,
    })
}

/// Which scenes (and `Music`, `Brand`) use each path of the timeline.
pub fn usage(timeline_json: &str) -> BTreeMap<String, Vec<String>> {
    let mut out: BTreeMap<String, Vec<String>> = BTreeMap::new();
    if let Ok(mut timeline) = serde_json::from_str::<Value>(timeline_json) {
        for_each_asset(&mut timeline, &mut |path, user| {
            let users = out.entry(path.clone()).or_default();
            if !users.iter().any(|u| u == user) {
                users.push(user.to_string());
            }
        });
    }
    out
}

/// Media files of the project, relative to its folder: everything under it except the renders,
/// the generation sources, the thumbnail, the project's own two files and hidden files.
fn media_files(dir: &Path) -> Vec<String> {
    fn walk(dir: &Path, rel: &Path, out: &mut Vec<String>) {
        let Ok(entries) = fs::read_dir(dir.join(rel)) else { return };
        for entry in entries.flatten() {
            let name = entry.file_name();
            let Some(text) = name.to_str() else { continue };
            if text.starts_with('.') {
                continue;
            }
            let top = rel.as_os_str().is_empty();
            if top && matches!(text, RENDERS_DIR | SOURCE_DIR | THUMB_FILE | PROJECT_FILE | TIMELINE_FILE) {
                continue;
            }
            let child = rel.join(&name);
            match entry.file_type() {
                Ok(kind) if kind.is_dir() => walk(dir, &child, out),
                Ok(kind) if kind.is_file() && kind_of(&child).is_some() => out.push(child.display().to_string()),
                _ => {}
            }
        }
    }
    let mut out = Vec::new();
    walk(dir, Path::new(""), &mut out);
    out.sort();
    out
}

/// Width, height and length of a file, where it has them.
type Probed = (Option<u32>, Option<u32>, Option<u64>);
/// Keyed by path, size and change time.
type ProbeCache = Mutex<HashMap<(PathBuf, u64, u64), Probed>>;

/// Size in pixels and length of a media file. Probing audio and video starts ffmpeg, so results
/// are remembered for as long as the file does not change.
fn probe(ffmpeg: &Path, file: &Path, kind: &str, bytes: u64, modified: u64) -> Probed {
    static CACHE: OnceLock<ProbeCache> = OnceLock::new();
    let cache = CACHE.get_or_init(Default::default);
    let key = (file.to_path_buf(), bytes, modified);
    if let Some(hit) = cache.lock().unwrap_or_else(|e| e.into_inner()).get(&key) {
        return *hit;
    }
    let found: Probed = match kind {
        "image" => match image::image_dimensions(file) {
            Ok((w, h)) => (Some(w), Some(h), None),
            Err(_) => (None, None, None),
        },
        "video" => match vimmer_media::probe_video(ffmpeg, file) {
            Ok(info) => (Some(info.width), Some(info.height), Some(info.duration_ms)),
            Err(_) => (None, None, None),
        },
        "audio" => {
            let probe = vimmer_media::FfmpegProbe { binary: ffmpeg.to_path_buf() };
            (None, None, probe.duration_ms(file).ok())
        }
        _ => (None, None, None),
    };
    cache.lock().unwrap_or_else(|e| e.into_inner()).insert(key, found);
    found
}

/// True when the two files have the same bytes.
fn same_content(a: &Path, b: &Path) -> bool {
    let (Ok(mut fa), Ok(mut fb)) = (fs::File::open(a), fs::File::open(b)) else { return false };
    let (mut ba, mut bb) = (vec![0u8; 1 << 16], vec![0u8; 1 << 16]);
    loop {
        let (Ok(na), Ok(nb)) = (read_full(&mut fa, &mut ba), read_full(&mut fb, &mut bb)) else { return false };
        if na != nb || ba[..na] != bb[..nb] {
            return false;
        }
        if na == 0 {
            return true;
        }
    }
}

fn read_full(file: &mut fs::File, buf: &mut [u8]) -> std::io::Result<usize> {
    let mut filled = 0;
    while filled < buf.len() {
        match file.read(&mut buf[filled..])? {
            0 => break,
            n => filled += n,
        }
    }
    Ok(filled)
}

/// A project-relative path the webview names must stay inside the project.
fn inside(rel: &str) -> bool {
    !rel.is_empty() && Path::new(rel).components().all(|c| matches!(c, Component::Normal(_)))
}

impl Studio {
    /// The project's media files, with what uses each in `timeline_json` (the editor's current
    /// document, saved or not).
    pub fn assets(&self, dir: &str, timeline_json: &str) -> Result<Vec<AssetInfo>, String> {
        let dir = self.project(dir)?;
        let users = usage(timeline_json);
        Ok(media_files(&dir)
            .into_iter()
            .filter_map(|rel| {
                let file = dir.join(&rel);
                let kind = kind_of(&file)?;
                let bytes = fs::metadata(&file).ok()?.len();
                let modified = modified_ms(&file);
                let (width, height, duration_ms) = probe(&self.ffmpeg, &file, kind, bytes, modified);
                Some(AssetInfo {
                    name: file.file_name()?.to_string_lossy().into_owned(),
                    kind,
                    bytes,
                    modified_ms: modified,
                    width,
                    height,
                    duration_ms,
                    used_by: users.get(&rel).cloned().unwrap_or_default(),
                    file: file.display().to_string(),
                    path: rel,
                })
            })
            .collect())
    }

    /// Copies files into `<project>/assets/`. A file whose bytes are already in the project is
    /// reused under its existing path; a different file with a taken name gets a numeric suffix.
    pub fn import_assets(&self, dir: &str, sources: &[String]) -> Result<Vec<ImportedAsset>, String> {
        let dir = self.project(dir)?;
        let folder = dir.join(ASSETS_DIR);
        fs::create_dir_all(&folder).map_err(|e| format!("Could not create {}: {e}", folder.display()))?;
        let mut out = Vec::new();
        for source in sources {
            let src = Path::new(source);
            let name = src.file_name().and_then(|n| n.to_str()).ok_or_else(|| format!("Not a file name: {source}"))?;
            if kind_of(src).is_none() {
                return Err(format!("{name} is not an image, video, audio, SVG or font file."));
            }
            let bytes = fs::metadata(src).map_err(|e| format!("Could not read {source}: {e}"))?.len();
            let existing = media_files(&dir).into_iter().find(|rel| {
                let file = dir.join(rel);
                fs::metadata(&file).is_ok_and(|m| m.len() == bytes) && same_content(&file, src)
            });
            if let Some(path) = existing {
                out.push(ImportedAsset { source: source.clone(), path, reused: true });
                continue;
            }
            let (stem, ext) = match name.rsplit_once('.') {
                Some((s, e)) => (s.to_string(), format!(".{e}")),
                None => (name.to_string(), String::new()),
            };
            let mut candidate = name.to_string();
            let mut n = 2;
            while folder.join(&candidate).exists() {
                candidate = format!("{stem}-{n}{ext}");
                n += 1;
            }
            let target = folder.join(&candidate);
            fs::copy(src, &target).map_err(|e| format!("Could not copy to {}: {e}", target.display()))?;
            out.push(ImportedAsset { source: source.clone(), path: format!("{ASSETS_DIR}/{candidate}"), reused: false });
        }
        Ok(out)
    }

    /// Moves one unused media file to the Trash.
    pub fn trash_asset(&self, dir: &str, path: &str, timeline_json: &str) -> Result<(), String> {
        let dir = self.project(dir)?;
        if !inside(path) || !media_files(&dir).iter().any(|rel| rel == path) {
            return Err(format!("{path} is not a media file of this project."));
        }
        if let Some(users) = usage(timeline_json).get(path) {
            return Err(format!("{path} is used by {}.", users.join(", ")));
        }
        (self.trash)(&dir.join(path))
    }

    /// Moves every media file the timeline does not use to the Trash; returns their paths.
    pub fn trash_unused_assets(&self, dir: &str, timeline_json: &str) -> Result<Vec<String>, String> {
        let dir = self.project(dir)?;
        if serde_json::from_str::<Value>(timeline_json).is_err() {
            return Err("The timeline is not valid JSON, so what it uses is unknown.".into());
        }
        let users = usage(timeline_json);
        let mut removed = Vec::new();
        for rel in media_files(&dir) {
            if !users.contains_key(&rel) {
                (self.trash)(&dir.join(&rel))?;
                removed.push(rel);
            }
        }
        Ok(removed)
    }
}

#[tauri::command]
pub async fn list_assets(app: AppHandle, dir: String, timeline_json: String) -> Result<Vec<AssetInfo>, String> {
    let studio = studio(&app)?;
    blocking(move || studio.assets(&dir, &timeline_json)).await
}

#[tauri::command]
pub async fn import_assets(app: AppHandle, dir: String, sources: Vec<String>) -> Result<Vec<ImportedAsset>, String> {
    let studio = studio(&app)?;
    blocking(move || studio.import_assets(&dir, &sources)).await
}

#[tauri::command]
pub async fn trash_asset(app: AppHandle, dir: String, path: String, timeline_json: String) -> Result<(), String> {
    let studio = studio(&app)?;
    blocking(move || studio.trash_asset(&dir, &path, &timeline_json)).await
}

#[tauri::command]
pub async fn trash_unused_assets(app: AppHandle, dir: String, timeline_json: String) -> Result<Vec<String>, String> {
    let studio = studio(&app)?;
    blocking(move || studio.trash_unused_assets(&dir, &timeline_json)).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::studio::tests::Fixture;

    /// A 3x2 PNG, so the size can be read without ffmpeg.
    fn tiny_png() -> Vec<u8> {
        let mut bytes = Vec::new();
        let img = image::RgbImage::from_pixel(3, 2, image::Rgb([200, 30, 30]));
        image::DynamicImage::ImageRgb8(img)
            .write_to(&mut std::io::Cursor::new(&mut bytes), image::ImageFormat::Png)
            .unwrap();
        bytes
    }

    const TIMELINE: &str = r#"{"fps":30,"resolution":{"width":1920,"height":1080},"scenes":[
        {"id":"intro","duration_ms":1000,"image":"assets/pic.png"},
        {"id":"second","duration_ms":1000,"image":"assets/pic.png","sfx":[{"src":"assets/pop.wav"}]}],
        "brand":{"logo":{"src":"assets/pic.png"}}}"#;

    #[test]
    fn import_copies_dedupes_by_content_and_suffixes_a_name_clash() {
        let fx = Fixture::new("assets-import");
        let made = fx.studio.create("P", None, "landscape").unwrap();
        let dir = PathBuf::from(&made.dir);
        let outside = fx.root.join("downloads");
        fs::create_dir_all(outside.join("other")).unwrap();
        fs::write(outside.join("pic.png"), tiny_png()).unwrap();
        fs::write(outside.join("same-bytes.png"), tiny_png()).unwrap();
        fs::write(outside.join("other/pic.png"), b"a different picture").unwrap();
        fs::write(outside.join("notes.txt"), b"text").unwrap();
        let s = |name: &str| outside.join(name).display().to_string();

        let got = fx.studio.import_assets(&made.dir, &[s("pic.png")]).unwrap();
        assert_eq!((got[0].path.as_str(), got[0].reused), ("assets/pic.png", false));
        assert_eq!(fs::read(dir.join("assets/pic.png")).unwrap(), tiny_png());
        // The same bytes under another name: nothing is copied.
        let again = fx.studio.import_assets(&made.dir, &[s("pic.png"), s("same-bytes.png")]).unwrap();
        assert!(again.iter().all(|a| a.path == "assets/pic.png" && a.reused), "{again:?}");
        // The same name with other bytes: a suffix.
        let other = fx.studio.import_assets(&made.dir, &[s("other/pic.png")]).unwrap();
        assert_eq!((other[0].path.as_str(), other[0].reused), ("assets/pic-2.png", false));
        assert_eq!(fs::read_dir(dir.join("assets")).unwrap().count(), 2);
        // Not media: refused, and the original files are untouched.
        assert!(fx.studio.import_assets(&made.dir, &[s("notes.txt")]).unwrap_err().contains("notes.txt"));
        assert!(outside.join("pic.png").is_file());
    }

    #[test]
    fn the_list_has_kind_size_and_who_uses_each_file() {
        let fx = Fixture::new("assets-list");
        let made = fx.studio.create("P", None, "landscape").unwrap();
        let dir = PathBuf::from(&made.dir);
        fs::write(dir.join("assets/pic.png"), tiny_png()).unwrap();
        fs::write(dir.join("assets/pop.wav"), b"not really audio").unwrap();
        fs::write(dir.join("assets/star.svg"), b"<svg/>").unwrap();
        fs::write(dir.join("assets/readme.txt"), b"ignored").unwrap();
        fs::write(dir.join("assets/.hidden.png"), b"ignored").unwrap();
        fs::create_dir_all(dir.join("generated/illustrations")).unwrap();
        fs::write(dir.join("generated/illustrations/a.png"), tiny_png()).unwrap();
        fs::write(dir.join("renders/out.mp4"), b"a render is not an asset").unwrap();
        fs::write(dir.join(THUMB_FILE), b"jpg").unwrap();

        let list = fx.studio.assets(&made.dir, TIMELINE).unwrap();
        let paths: Vec<&str> = list.iter().map(|a| a.path.as_str()).collect();
        assert_eq!(paths, ["assets/pic.png", "assets/pop.wav", "assets/star.svg", "generated/illustrations/a.png"]);
        let pic = &list[0];
        assert_eq!((pic.kind, pic.width, pic.height), ("image", Some(3), Some(2)));
        assert_eq!(pic.bytes, tiny_png().len() as u64);
        assert_eq!(pic.used_by, ["intro", "second", "Brand"]);
        assert_eq!(pic.file, dir.join("assets/pic.png").display().to_string());
        assert_eq!((list[1].kind, list[1].used_by.clone()), ("audio", vec!["second".to_string()]));
        assert_eq!(list[1].duration_ms, None, "an unreadable file still lists");
        assert_eq!((list[2].kind, list[2].used_by.is_empty()), ("icon", true));
        // A broken document: everything lists as unused.
        assert!(fx.studio.assets(&made.dir, "{").unwrap().iter().all(|a| a.used_by.is_empty()));
    }

    #[test]
    fn only_unused_files_go_to_the_trash() {
        let fx = Fixture::new("assets-trash");
        let made = fx.studio.create("P", None, "landscape").unwrap();
        let dir = PathBuf::from(&made.dir);
        for name in ["pic.png", "pop.wav", "old.png", "spare.mp3"] {
            fs::write(dir.join("assets").join(name), name.as_bytes()).unwrap();
        }
        let err = fx.studio.trash_asset(&made.dir, "assets/pic.png", TIMELINE).unwrap_err();
        assert!(err.contains("intro"), "{err}");
        for bad in ["../outside.png", "project.json", "timeline.json", "assets/none.png", "/etc/hosts", ""] {
            assert!(fx.studio.trash_asset(&made.dir, bad, TIMELINE).is_err(), "{bad}");
        }
        fx.studio.trash_asset(&made.dir, "assets/old.png", TIMELINE).unwrap();
        assert!(!dir.join("assets/old.png").exists());
        assert_eq!(fx.trashed(), ["old.png"]);

        // With a document that does not parse, nothing is removed.
        assert!(fx.studio.trash_unused_assets(&made.dir, "{").is_err());
        assert!(dir.join("assets/spare.mp3").is_file());
        let removed = fx.studio.trash_unused_assets(&made.dir, TIMELINE).unwrap();
        assert_eq!(removed, ["assets/spare.mp3"]);
        assert_eq!(fx.trashed(), ["old.png", "spare.mp3"]);
        assert!(dir.join("assets/pic.png").is_file() && dir.join("assets/pop.wav").is_file());
        assert!(dir.join(TIMELINE_FILE).is_file() && dir.join(PROJECT_FILE).is_file());
    }
}
