//! Studio projects: one folder per video under the projects root (`~/Documents/Vimmer` unless
//! changed in the settings).
//!
//! ```text
//! <Name>/project.json   name, times, language, orientation, length, render history
//!        timeline.json  the timeline; asset paths are relative to this folder ("assets/x.png")
//!        assets/        imported media
//!        renders/       rendered MP4s
//!        source/        input text, storyboard and usage of a generated project
//!        thumb.jpg      the picture on the Home screen
//! ```
//!
//! Everything here works on plain paths so the tests run in temp folders. The Tauri commands at
//! the bottom build a [`Studio`] from the app's own folders. Deleting always means moving to the
//! Trash.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tauri::{AppHandle, Manager};

use crate::paths::{for_each_asset, is_local_relative};

pub const SCHEMA: &str = "vimmer-project/1";
pub const PROJECT_FILE: &str = "project.json";
pub const TIMELINE_FILE: &str = "timeline.json";
pub const THUMB_FILE: &str = "thumb.jpg";
pub const ASSETS_DIR: &str = "assets";
pub const RENDERS_DIR: &str = "renders";
pub const SOURCE_DIR: &str = "source";
/// Tracks shipped with the app stay in the public folder; a project refers to them by path.
pub const MUSIC_LIBRARY_PREFIX: &str = "assets/music/library/";
const MAX_RECENT: usize = 12;

/// One finished render of a project.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RenderRecord {
    pub id: String,
    /// Project-relative path, `renders/<name>.mp4`.
    pub file: String,
    pub finished_ms: u64,
    pub bytes: u64,
    pub duration_ms: u64,
    /// The render panel's options, as the editor sent them.
    #[serde(default)]
    pub options: Value,
    /// An existing video the user added, not rendered here.
    #[serde(default)]
    pub imported: bool,
}

/// `project.json`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectMeta {
    pub schema: String,
    pub id: String,
    pub name: String,
    pub created_ms: u64,
    pub updated_ms: u64,
    /// `zh`, `ja` or `en` when known.
    #[serde(default)]
    pub lang: Option<String>,
    /// `landscape` or `portrait`.
    pub orientation: String,
    #[serde(default)]
    pub duration_ms: u64,
    /// `blank`, `generated` or `imported`.
    pub origin: String,
    /// For an imported project: the timeline file it came from.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub imported_from: Option<String>,
    #[serde(default)]
    pub renders: Vec<RenderRecord>,
    /// What `thumb.jpg` was made from (`render:<id>` or `image:<path>`), to skip redoing it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thumb_source: Option<String>,
}

/// What the Home screen shows for a project.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectSummary {
    pub dir: String,
    pub name: String,
    pub created_ms: u64,
    pub updated_ms: u64,
    pub lang: Option<String>,
    pub orientation: String,
    pub duration_ms: u64,
    pub origin: String,
    pub renders: usize,
    /// Absolute path of `thumb.jpg` when there is one, and its change time (to refresh the
    /// picture in the webview).
    pub thumb: Option<String>,
    pub thumb_version: u64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OpenedProject {
    pub project: ProjectSummary,
    pub timeline_json: String,
    pub renders: Vec<RenderRecord>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportedProject {
    pub project: ProjectSummary,
    /// Files copied into the project.
    pub copied: Vec<String>,
    /// Paths the timeline names that were found neither next to it nor in the public folder.
    pub missing: Vec<String>,
}

/// `studio.json` in the app's config folder.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Settings {
    /// The projects root; `None`: `~/Documents/Vimmer`.
    pub root: Option<String>,
    /// Project folders, most recently opened first.
    pub recent: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SettingsView {
    pub root: String,
    pub default_root: String,
    pub is_default: bool,
    pub recent: Vec<ProjectSummary>,
}

pub type TrashFn = Arc<dyn Fn(&Path) -> Result<(), String> + Send + Sync>;

/// The folders the studio works in.
#[derive(Clone)]
pub struct Studio {
    pub settings_file: PathBuf,
    /// The renderer's public folder (shipped assets; fallback for timeline paths).
    pub public_dir: PathBuf,
    pub ffmpeg: PathBuf,
    pub home: PathBuf,
    pub trash: TrashFn,
}

pub fn now_ms() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
}

fn new_id() -> String {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let nanos = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
    format!("{nanos:x}-{:x}-{:x}", std::process::id(), COUNTER.fetch_add(1, Ordering::Relaxed))
}

/// A folder name for a project name: no path separators, colons or control characters, no
/// leading dot, at most 80 characters. Never empty.
pub fn sanitize_name(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .map(|c| if matches!(c, '/' | '\\' | ':' | '\0') || c.is_control() { ' ' } else { c })
        .collect();
    let joined = cleaned.split_whitespace().collect::<Vec<_>>().join(" ");
    let trimmed: String = joined.trim_matches(|c: char| c == '.' || c.is_whitespace()).chars().take(80).collect();
    let trimmed = trimmed.trim().to_string();
    if trimmed.is_empty() {
        "Untitled".into()
    } else {
        trimmed
    }
}

/// `root/base`, or `root/base 2`, `root/base 3`, … when taken.
pub fn unique_dir(root: &Path, base: &str) -> PathBuf {
    let first = root.join(base);
    if !first.exists() {
        return first;
    }
    (2..)
        .map(|n| root.join(format!("{base} {n}")))
        .find(|p| !p.exists())
        .expect("some name is free")
}

/// Writes atomically: a sibling temp file first, then a rename over the target.
pub fn write_atomic(target: &Path, bytes: &[u8]) -> Result<(), String> {
    crate::project::write_atomic(target, bytes).map_err(|e| format!("Could not save {}: {e}", target.display()))
}

pub fn read_meta(dir: &Path) -> Result<ProjectMeta, String> {
    let file = dir.join(PROJECT_FILE);
    let raw = fs::read_to_string(&file).map_err(|e| format!("Could not read {}: {e}", file.display()))?;
    serde_json::from_str(&raw).map_err(|e| format!("{} is not a project file: {e}", file.display()))
}

pub fn write_meta(dir: &Path, meta: &ProjectMeta) -> Result<(), String> {
    let body = serde_json::to_string_pretty(meta).expect("meta serializes") + "\n";
    write_atomic(&dir.join(PROJECT_FILE), body.as_bytes())
}

pub fn summary(dir: &Path, meta: &ProjectMeta) -> ProjectSummary {
    let thumb = dir.join(THUMB_FILE);
    let thumb_version = fs::metadata(&thumb)
        .ok()
        .filter(|m| m.is_file())
        .and_then(|m| m.modified().ok())
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_millis() as u64);
    ProjectSummary {
        dir: dir.display().to_string(),
        name: meta.name.clone(),
        created_ms: meta.created_ms,
        updated_ms: meta.updated_ms,
        lang: meta.lang.clone(),
        orientation: meta.orientation.clone(),
        duration_ms: meta.duration_ms,
        origin: meta.origin.clone(),
        renders: meta.renders.len(),
        thumb: thumb_version.map(|_| thumb.display().to_string()),
        thumb_version: thumb_version.unwrap_or(0),
    }
}

/// Length and orientation of a timeline document, when it parses. Template scenes are expanded
/// first, as a render does.
pub fn timeline_facts(timeline_json: &str) -> Option<(u64, &'static str)> {
    let mut timeline: vimmer_dsl::VideoTimeline = serde_json::from_str(timeline_json).ok()?;
    let _ = vimmer_dsl::expand_templates(&mut timeline);
    let orientation = if timeline.resolution.height > timeline.resolution.width { "portrait" } else { "landscape" };
    Some((timeline.total_duration_ms(), orientation))
}

/// The timeline of a new, empty project: one scene showing the project's name.
pub fn blank_timeline(name: &str, orientation: &str) -> Value {
    let (width, height) = if orientation == "portrait" { (1080, 1920) } else { (1920, 1080) };
    json!({
        "fps": 30,
        "resolution": { "width": width, "height": height },
        "scenes": [{ "id": "scene-1", "duration_ms": 4000, "text": name, "highlights": [], "image": null, "motion": "static", "audio": null }],
        "subtitles": []
    })
}

fn copy_dir(from: &Path, to: &Path, skip: &[&str]) -> std::io::Result<()> {
    fs::create_dir_all(to)?;
    for entry in fs::read_dir(from)? {
        let entry = entry?;
        let name = entry.file_name();
        if skip.iter().any(|s| name.to_str() == Some(*s)) {
            continue;
        }
        let kind = entry.file_type()?;
        if kind.is_dir() {
            copy_dir(&entry.path(), &to.join(&name), &[])?;
        } else if kind.is_file() {
            fs::copy(entry.path(), to.join(&name))?;
        }
    }
    Ok(())
}

impl Studio {
    pub fn settings(&self) -> Settings {
        fs::read_to_string(&self.settings_file).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default()
    }

    fn save_settings(&self, settings: &Settings) -> Result<(), String> {
        if let Some(dir) = self.settings_file.parent() {
            fs::create_dir_all(dir).map_err(|e| format!("Could not create {}: {e}", dir.display()))?;
        }
        let body = serde_json::to_string_pretty(settings).expect("settings serialize") + "\n";
        write_atomic(&self.settings_file, body.as_bytes())
    }

    pub fn default_root(&self) -> PathBuf {
        self.home.join("Documents").join("Vimmer")
    }

    pub fn root(&self) -> PathBuf {
        self.settings().root.map(PathBuf::from).unwrap_or_else(|| self.default_root())
    }

    /// Changes the projects root (created when missing). Projects are not moved.
    pub fn set_root(&self, root: &str) -> Result<(), String> {
        let path = Path::new(root);
        if !path.is_absolute() {
            return Err("The projects folder must be an absolute path.".into());
        }
        fs::create_dir_all(path).map_err(|e| format!("Could not create {root}: {e}"))?;
        let mut settings = self.settings();
        settings.root = (path != self.default_root()).then(|| root.to_string());
        self.save_settings(&settings)
    }

    pub fn settings_view(&self) -> SettingsView {
        let root = self.root();
        let recent = self
            .settings()
            .recent
            .iter()
            .filter_map(|dir| self.project(dir).ok())
            .filter_map(|dir| read_meta(&dir).ok().map(|m| summary(&dir, &m)))
            .collect();
        SettingsView {
            root: root.display().to_string(),
            default_root: self.default_root().display().to_string(),
            is_default: root == self.default_root(),
            recent,
        }
    }

    /// The folder of a project the webview names. It must be a project directly inside the
    /// projects root, so a command can never be pointed at another folder.
    pub fn project(&self, dir: &str) -> Result<PathBuf, String> {
        let not_a_project = || format!("{dir} is not a project in {}", self.root().display());
        let root = self.root().canonicalize().map_err(|_| not_a_project())?;
        let path = Path::new(dir).canonicalize().map_err(|_| not_a_project())?;
        if path.parent() != Some(root.as_path()) || !path.join(PROJECT_FILE).is_file() {
            return Err(not_a_project());
        }
        Ok(path)
    }

    /// Every project in the root, most recently changed first.
    pub fn list(&self) -> Vec<ProjectSummary> {
        let root = self.root();
        let root = root.canonicalize().unwrap_or(root);
        let mut out: Vec<ProjectSummary> = fs::read_dir(&root)
            .into_iter()
            .flatten()
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.is_dir())
            .filter_map(|dir| read_meta(&dir).ok().map(|m| summary(&dir, &m)))
            .collect();
        out.sort_by(|a, b| b.updated_ms.cmp(&a.updated_ms).then_with(|| a.name.cmp(&b.name)));
        out
    }

    /// Makes the project folder with its timeline and returns the folder.
    pub fn create_with(
        &self,
        name: &str,
        timeline_json: &str,
        lang: Option<&str>,
        origin: &str,
        imported_from: Option<&str>,
    ) -> Result<PathBuf, String> {
        let root = self.root();
        fs::create_dir_all(&root).map_err(|e| format!("Could not create {}: {e}", root.display()))?;
        let root = root.canonicalize().map_err(|e| format!("{}: {e}", root.display()))?;
        let display = name.trim();
        let display = if display.is_empty() { "Untitled" } else { display };
        let dir = unique_dir(&root, &sanitize_name(display));
        for sub in [ASSETS_DIR, RENDERS_DIR] {
            fs::create_dir_all(dir.join(sub)).map_err(|e| format!("Could not create {}: {e}", dir.display()))?;
        }
        write_atomic(&dir.join(TIMELINE_FILE), timeline_json.as_bytes())?;
        let (duration_ms, orientation) = timeline_facts(timeline_json).unwrap_or((0, "landscape"));
        let now = now_ms();
        let meta = ProjectMeta {
            schema: SCHEMA.into(),
            id: new_id(),
            // A second "Demo" becomes the folder "Demo 2"; show that name too.
            name: if dir.file_name().and_then(|n| n.to_str()) == Some(sanitize_name(display).as_str()) {
                display.to_string()
            } else {
                dir.file_name().and_then(|n| n.to_str()).unwrap_or(display).to_string()
            },
            created_ms: now,
            updated_ms: now,
            lang: lang.map(String::from),
            orientation: orientation.into(),
            duration_ms,
            origin: origin.into(),
            imported_from: imported_from.map(String::from),
            renders: Vec::new(),
            thumb_source: None,
        };
        write_meta(&dir, &meta)?;
        Ok(dir)
    }

    pub fn create(&self, name: &str, lang: Option<&str>, orientation: &str) -> Result<ProjectSummary, String> {
        let display = if name.trim().is_empty() { "Untitled" } else { name.trim() };
        let timeline = serde_json::to_string_pretty(&blank_timeline(display, orientation)).expect("serializes") + "\n";
        let dir = self.create_with(display, &timeline, lang, "blank", None)?;
        self.touch_thumbnail(&dir)
    }

    pub fn open(&self, dir: &str) -> Result<OpenedProject, String> {
        let dir = self.project(dir)?;
        let meta = read_meta(&dir)?;
        let file = dir.join(TIMELINE_FILE);
        let timeline_json = fs::read_to_string(&file).map_err(|e| format!("Could not read {}: {e}", file.display()))?;
        self.remember(&dir);
        Ok(OpenedProject { project: summary(&dir, &meta), timeline_json, renders: meta.renders })
    }

    fn remember(&self, dir: &Path) {
        let mut settings = self.settings();
        let entry = dir.display().to_string();
        settings.recent.retain(|d| *d != entry);
        settings.recent.insert(0, entry);
        settings.recent.truncate(MAX_RECENT);
        let _ = self.save_settings(&settings);
    }

    fn forget(&self, dir: &Path, replacement: Option<&Path>) {
        let mut settings = self.settings();
        let entry = dir.display().to_string();
        match replacement {
            Some(new) => {
                for d in &mut settings.recent {
                    if *d == entry {
                        *d = new.display().to_string();
                    }
                }
            }
            None => settings.recent.retain(|d| *d != entry),
        }
        let _ = self.save_settings(&settings);
    }

    /// Saves the timeline (atomically) and brings `project.json` and the thumbnail up to date.
    pub fn save(&self, dir: &str, timeline_json: &str) -> Result<ProjectSummary, String> {
        let dir = self.project(dir)?;
        write_atomic(&dir.join(TIMELINE_FILE), timeline_json.as_bytes())?;
        let mut meta = read_meta(&dir)?;
        meta.updated_ms = now_ms();
        if let Some((duration_ms, orientation)) = timeline_facts(timeline_json) {
            meta.duration_ms = duration_ms;
            meta.orientation = orientation.into();
        }
        write_meta(&dir, &meta)?;
        self.touch_thumbnail(&dir)
    }

    /// Renames the project and its folder.
    pub fn rename(&self, dir: &str, name: &str) -> Result<ProjectSummary, String> {
        let dir = self.project(dir)?;
        let display = name.trim();
        if display.is_empty() {
            return Err("The name is empty.".into());
        }
        let mut meta = read_meta(&dir)?;
        let folder = sanitize_name(display);
        let mut target = dir.clone();
        if dir.file_name().and_then(|n| n.to_str()) != Some(folder.as_str()) {
            let root = dir.parent().ok_or("The project has no parent folder.")?;
            let wanted = root.join(&folder);
            // A change of letter case only is the same folder on the default macOS file system.
            let same_folder = wanted.exists() && wanted.canonicalize().ok().as_deref() == Some(dir.as_path());
            if wanted.exists() && !same_folder {
                return Err(format!("A project named \"{folder}\" already exists."));
            }
            fs::rename(&dir, &wanted).map_err(|e| format!("Could not rename {}: {e}", dir.display()))?;
            target = wanted.canonicalize().unwrap_or(wanted);
            self.forget(&dir, Some(&target));
        }
        meta.name = display.to_string();
        meta.updated_ms = now_ms();
        write_meta(&target, &meta)?;
        Ok(summary(&target, &meta))
    }

    /// A copy of the project (timeline, assets, source) without its renders.
    pub fn duplicate(&self, dir: &str) -> Result<ProjectSummary, String> {
        let dir = self.project(dir)?;
        let mut meta = read_meta(&dir)?;
        let root = dir.parent().ok_or("The project has no parent folder.")?;
        let name = format!("{} copy", meta.name);
        let target = unique_dir(root, &sanitize_name(&name));
        copy_dir(&dir, &target, &[RENDERS_DIR]).map_err(|e| format!("Could not copy {}: {e}", dir.display()))?;
        fs::create_dir_all(target.join(RENDERS_DIR)).map_err(|e| e.to_string())?;
        let now = now_ms();
        meta.id = new_id();
        meta.name = target.file_name().and_then(|n| n.to_str()).unwrap_or(&name).to_string();
        meta.created_ms = now;
        meta.updated_ms = now;
        meta.renders.clear();
        write_meta(&target, &meta)?;
        self.touch_thumbnail(&target)
    }

    /// Moves the whole project folder to the Trash.
    pub fn trash_project(&self, dir: &str) -> Result<(), String> {
        let dir = self.project(dir)?;
        (self.trash)(&dir)?;
        self.forget(&dir, None);
        Ok(())
    }

    /// Makes a project from a timeline file. Every file the timeline names is copied into the
    /// project at the same relative path (from next to the timeline first, then from the public
    /// folder), so the timeline itself is stored unchanged. The shipped music library stays where
    /// it is.
    pub fn import_timeline(&self, path: &str) -> Result<ImportedProject, String> {
        let file = Path::new(path);
        let raw = fs::read_to_string(file).map_err(|e| format!("Could not read {path}: {e}"))?;
        let mut value: Value = serde_json::from_str(&raw).map_err(|e| format!("{path} is not JSON: {e}"))?;
        serde_json::from_value::<vimmer_dsl::VideoTimeline>(value.clone())
            .map_err(|e| format!("{path} is not a timeline: {e}"))?;
        let folder = file.parent().filter(|p| !p.as_os_str().is_empty()).unwrap_or(Path::new("."));
        let stem = file.file_stem().and_then(|s| s.to_str()).unwrap_or("Imported");
        let name = if stem == "timeline" {
            folder.file_name().and_then(|n| n.to_str()).unwrap_or(stem).to_string()
        } else {
            stem.trim_end_matches(".timeline").to_string()
        };
        let lang = fs::read_to_string(folder.join(PROJECT_FILE))
            .ok()
            .and_then(|s| serde_json::from_str::<ProjectMeta>(&s).ok())
            .and_then(|m| m.lang);

        let mut wanted: Vec<String> = Vec::new();
        for_each_asset(&mut value, &mut |p, _| {
            if is_local_relative(p) && !wanted.contains(p) {
                wanted.push(p.clone());
            }
        });
        let dir = self.create_with(&name, &raw, lang.as_deref(), "imported", Some(path))?;
        let mut copied = Vec::new();
        let mut missing = Vec::new();
        for rel in wanted {
            let inside = Path::new(&rel).components().all(|c| matches!(c, std::path::Component::Normal(_)));
            let local = folder.join(&rel);
            let source = if local.is_file() {
                local
            } else if rel.starts_with(MUSIC_LIBRARY_PREFIX) {
                continue;
            } else if self.public_dir.join(&rel).is_file() {
                self.public_dir.join(&rel)
            } else {
                missing.push(rel);
                continue;
            };
            if !inside {
                // `../x.png` cannot keep its place inside the project; it stays a missing file.
                missing.push(rel);
                continue;
            }
            let target = dir.join(&rel);
            if let Some(parent) = target.parent() {
                fs::create_dir_all(parent).map_err(|e| format!("Could not create {}: {e}", parent.display()))?;
            }
            fs::copy(&source, &target).map_err(|e| format!("Could not copy {}: {e}", source.display()))?;
            copied.push(rel);
        }
        let project = self.touch_thumbnail(&dir)?;
        Ok(ImportedProject { project, copied, missing })
    }

    /// Refreshes the thumbnail when what it should show has changed, and returns the summary.
    pub fn touch_thumbnail(&self, dir: &Path) -> Result<ProjectSummary, String> {
        let mut meta = read_meta(dir)?;
        if crate::renders::refresh_thumbnail(self, dir, &mut meta) {
            write_meta(dir, &meta)?;
        }
        Ok(summary(dir, &meta))
    }
}

/// Moves a file or folder to the Trash: `/usr/bin/trash` (macOS 15 and later), else Finder.
pub fn move_to_trash(path: &Path) -> Result<(), String> {
    if !path.exists() {
        return Err(format!("{} does not exist", path.display()));
    }
    if Path::new("/usr/bin/trash").is_file() {
        let done = Command::new("/usr/bin/trash").arg(path).output();
        if done.as_ref().is_ok_and(|o| o.status.success()) && !path.exists() {
            return Ok(());
        }
    }
    let script = "on run argv\ntell application \"Finder\" to delete (POSIX file (item 1 of argv) as alias)\nend run";
    let out = Command::new("/usr/bin/osascript")
        .args(["-e", script])
        .arg(path)
        .output()
        .map_err(|e| format!("Could not move {} to the Trash: {e}", path.display()))?;
    if out.status.success() && !path.exists() {
        Ok(())
    } else {
        Err(format!(
            "Could not move {} to the Trash: {}",
            path.display(),
            String::from_utf8_lossy(&out.stderr).trim()
        ))
    }
}

/// The studio for the running app.
pub fn studio(app: &AppHandle) -> Result<Studio, String> {
    let config = app.path().app_config_dir().map_err(|e| format!("No settings folder: {e}"))?;
    let home = app.path().home_dir().map_err(|e| format!("No home folder: {e}"))?;
    Ok(Studio {
        settings_file: config.join("studio.json"),
        public_dir: crate::assets::public_dir(),
        ffmpeg: crate::render::find_in_path("ffmpeg", &crate::render::user_path()),
        home,
        trash: Arc::new(move_to_trash),
    })
}

/// Lets the webview load thumbnails, assets and renders of the projects root.
pub fn allow_root(app: &AppHandle) {
    if let Ok(studio) = studio(app) {
        let _ = app.asset_protocol_scope().allow_directory(studio.root(), true);
    }
}

/// Runs file work off the main thread.
pub(crate) async fn blocking<T: Send + 'static>(
    work: impl FnOnce() -> Result<T, String> + Send + 'static,
) -> Result<T, String> {
    tauri::async_runtime::spawn_blocking(work).await.map_err(|e| format!("The task stopped: {e}"))?
}

#[tauri::command]
pub async fn studio_settings(app: AppHandle) -> Result<SettingsView, String> {
    let studio = studio(&app)?;
    blocking(move || Ok(studio.settings_view())).await
}

#[tauri::command]
pub async fn set_projects_root(app: AppHandle, root: String) -> Result<SettingsView, String> {
    let studio = studio(&app)?;
    let view = blocking(move || {
        studio.set_root(&root)?;
        Ok(studio.settings_view())
    })
    .await?;
    allow_root(&app);
    Ok(view)
}

#[tauri::command]
pub async fn list_projects(app: AppHandle) -> Result<Vec<ProjectSummary>, String> {
    let studio = studio(&app)?;
    blocking(move || Ok(studio.list())).await
}

#[tauri::command]
pub async fn create_project(
    app: AppHandle,
    name: String,
    lang: Option<String>,
    orientation: String,
) -> Result<ProjectSummary, String> {
    let studio = studio(&app)?;
    let made = blocking(move || studio.create(&name, lang.as_deref(), &orientation)).await?;
    allow_root(&app);
    Ok(made)
}

#[tauri::command]
pub async fn open_project(app: AppHandle, dir: String) -> Result<OpenedProject, String> {
    let studio = studio(&app)?;
    blocking(move || studio.open(&dir)).await
}

#[tauri::command]
pub async fn save_project(app: AppHandle, dir: String, timeline_json: String) -> Result<ProjectSummary, String> {
    let studio = studio(&app)?;
    blocking(move || studio.save(&dir, &timeline_json)).await
}

#[tauri::command]
pub async fn rename_project(app: AppHandle, dir: String, name: String) -> Result<ProjectSummary, String> {
    let studio = studio(&app)?;
    blocking(move || studio.rename(&dir, &name)).await
}

#[tauri::command]
pub async fn duplicate_project(app: AppHandle, dir: String) -> Result<ProjectSummary, String> {
    let studio = studio(&app)?;
    blocking(move || studio.duplicate(&dir)).await
}

#[tauri::command]
pub async fn trash_project(app: AppHandle, dir: String) -> Result<(), String> {
    let studio = studio(&app)?;
    blocking(move || studio.trash_project(&dir)).await
}

#[tauri::command]
pub async fn import_timeline(app: AppHandle, path: String) -> Result<ImportedProject, String> {
    let studio = studio(&app)?;
    let made = blocking(move || studio.import_timeline(&path)).await?;
    allow_root(&app);
    Ok(made)
}

/// Shows a project folder, or a file inside the projects root, in Finder.
#[tauri::command]
pub async fn reveal_path(app: AppHandle, path: String) -> Result<(), String> {
    let studio = studio(&app)?;
    blocking(move || {
        let root = studio.root().canonicalize().map_err(|e| e.to_string())?;
        let target = Path::new(&path).canonicalize().map_err(|_| format!("{path} does not exist"))?;
        if !target.starts_with(&root) {
            return Err(format!("{path} is not inside {}", root.display()));
        }
        Command::new("open").arg("-R").arg(&target).status().map_err(|e| e.to_string())?;
        Ok(())
    })
    .await
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// A studio in a temp folder. "Trash" is a folder next to it, so nothing leaves the temp dir.
    pub(crate) struct Fixture {
        pub root: PathBuf,
        pub studio: Studio,
    }

    impl Fixture {
        pub(crate) fn new(name: &str) -> Self {
            let base = std::env::temp_dir().join(format!("vimmer-studio-{}-{name}", std::process::id()));
            let _ = fs::remove_dir_all(&base);
            fs::create_dir_all(base.join("public")).unwrap();
            fs::create_dir_all(base.join("trash")).unwrap();
            let base = base.canonicalize().unwrap();
            let trash_dir = base.join("trash");
            let studio = Studio {
                settings_file: base.join("config/studio.json"),
                public_dir: base.join("public"),
                ffmpeg: crate::render::find_in_path("ffmpeg", &crate::render::user_path()),
                home: base.join("home"),
                trash: Arc::new(move |p: &Path| {
                    let name = p.file_name().ok_or("no name")?;
                    fs::rename(p, unique_dir(&trash_dir, &name.to_string_lossy())).map_err(|e| e.to_string())
                }),
            };
            Self { root: base, studio }
        }

        pub(crate) fn trashed(&self) -> Vec<String> {
            let mut names: Vec<String> = fs::read_dir(self.root.join("trash"))
                .unwrap()
                .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
                .collect();
            names.sort();
            names
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.root);
        }
    }

    #[test]
    fn the_default_root_is_documents_vimmer_and_can_be_changed() {
        let fx = Fixture::new("root");
        assert_eq!(fx.studio.root(), fx.root.join("home/Documents/Vimmer"));
        assert!(fx.studio.settings_view().is_default);
        assert!(fx.studio.list().is_empty(), "a missing root lists nothing");
        assert!(!fx.studio.root().exists(), "listing does not create the root");
        assert!(fx.studio.set_root("relative/path").is_err());
        let other = fx.root.join("elsewhere");
        fx.studio.set_root(&other.display().to_string()).unwrap();
        assert_eq!(fx.studio.root(), other);
        assert!(other.is_dir());
        assert!(!fx.studio.settings_view().is_default);
        fx.studio.set_root(&fx.studio.default_root().display().to_string()).unwrap();
        assert_eq!(fx.studio.settings().root, None);
    }

    #[test]
    fn names_become_safe_folder_names() {
        assert_eq!(sanitize_name("  My / Video: one  "), "My Video one");
        assert_eq!(sanitize_name("..hidden"), "hidden");
        assert_eq!(sanitize_name("西安 钟楼"), "西安 钟楼");
        assert_eq!(sanitize_name(" /: "), "Untitled");
        assert_eq!(sanitize_name(&"x".repeat(200)).chars().count(), 80);
    }

    #[test]
    fn create_list_open_save_round_trip() {
        let fx = Fixture::new("create");
        let made = fx.studio.create("Demo", Some("en"), "portrait").unwrap();
        assert_eq!(made.name, "Demo");
        assert_eq!(made.orientation, "portrait");
        assert_eq!(made.duration_ms, 4000);
        assert_eq!(made.origin, "blank");
        let dir = PathBuf::from(&made.dir);
        assert_eq!(dir, fx.studio.root().canonicalize().unwrap().join("Demo"));
        assert!(dir.join("assets").is_dir() && dir.join("renders").is_dir());
        // The blank timeline is one the scheduler accepts.
        let raw = fs::read_to_string(dir.join(TIMELINE_FILE)).unwrap();
        let timeline: vimmer_dsl::VideoTimeline = serde_json::from_str(&raw).unwrap();
        vimmer_dsl::validate(&timeline).unwrap();
        assert_eq!((timeline.resolution.width, timeline.resolution.height), (1080, 1920));

        // The same name again gets its own folder and says so.
        let second = fx.studio.create("Demo", None, "landscape").unwrap();
        assert_eq!(second.name, "Demo 2");
        assert_eq!(fx.studio.list().len(), 2);

        let opened = fx.studio.open(&made.dir).unwrap();
        assert_eq!(opened.timeline_json, raw);
        assert_eq!(fx.studio.settings().recent, vec![made.dir.clone()]);

        let longer = raw.replace("4000", "9000").replace("1080", "X").replace("1920", "1080").replace("X", "1920");
        let saved = fx.studio.save(&made.dir, &longer).unwrap();
        assert_eq!(saved.duration_ms, 9000);
        assert_eq!(saved.orientation, "landscape");
        assert!(saved.updated_ms >= made.updated_ms);
        assert_eq!(fs::read_to_string(dir.join(TIMELINE_FILE)).unwrap(), longer);
        // No temp files are left behind.
        let names: Vec<String> =
            fs::read_dir(&dir).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect();
        assert!(names.iter().all(|n| !n.contains(".tmp")), "{names:?}");
        // A half-typed document is still saved; the facts keep their last good values.
        let kept = fx.studio.save(&made.dir, "{ not json").unwrap();
        assert_eq!(kept.duration_ms, 9000);
        assert_eq!(fx.studio.list()[0].dir, made.dir, "most recently changed first");
    }

    #[test]
    fn commands_only_reach_projects_inside_the_root() {
        let fx = Fixture::new("guard");
        let made = fx.studio.create("Inside", None, "landscape").unwrap();
        let outside = fx.root.join("outside");
        fs::create_dir_all(&outside).unwrap();
        fs::copy(Path::new(&made.dir).join(PROJECT_FILE), outside.join(PROJECT_FILE)).unwrap();
        for bad in [outside.display().to_string(), fx.studio.root().display().to_string(), "/".into(), format!("{}/assets", made.dir)] {
            assert!(fx.studio.project(&bad).is_err(), "{bad}");
            assert!(fx.studio.trash_project(&bad).is_err(), "{bad}");
            assert!(fx.studio.save(&bad, "{}").is_err(), "{bad}");
        }
        assert!(outside.join(PROJECT_FILE).is_file());
        assert!(fx.studio.project(&made.dir).is_ok());
    }

    #[test]
    fn rename_moves_the_folder_and_refuses_a_taken_name() {
        let fx = Fixture::new("rename");
        let a = fx.studio.create("First", None, "landscape").unwrap();
        fx.studio.create("Second", None, "landscape").unwrap();
        fx.studio.open(&a.dir).unwrap();
        fs::write(Path::new(&a.dir).join("assets/pic.png"), b"png").unwrap();

        assert!(fx.studio.rename(&a.dir, "  ").is_err());
        assert!(fx.studio.rename(&a.dir, "Second").unwrap_err().contains("already exists"));
        let renamed = fx.studio.rename(&a.dir, "Intro: take 2").unwrap();
        assert_eq!(renamed.name, "Intro: take 2");
        assert!(renamed.dir.ends_with("/Intro take 2"), "{}", renamed.dir);
        assert!(!Path::new(&a.dir).exists());
        assert_eq!(fs::read(Path::new(&renamed.dir).join("assets/pic.png")).unwrap(), b"png");
        assert_eq!(fx.studio.settings().recent, vec![renamed.dir.clone()], "recents follow the rename");
        // Same folder, new display name.
        let again = fx.studio.rename(&renamed.dir, "Intro/take 2").unwrap();
        assert_eq!(again.dir, renamed.dir);
        assert_eq!(again.name, "Intro/take 2");
    }

    #[test]
    fn duplicate_copies_everything_but_the_renders() {
        let fx = Fixture::new("duplicate");
        let a = fx.studio.create("Film", Some("ja"), "landscape").unwrap();
        let dir = Path::new(&a.dir);
        fs::create_dir_all(dir.join("assets/nested")).unwrap();
        fs::write(dir.join("assets/nested/pic.png"), b"png").unwrap();
        fs::create_dir_all(dir.join("source")).unwrap();
        fs::write(dir.join("source/input.txt"), "text").unwrap();
        fs::write(dir.join("renders/one.mp4"), b"video").unwrap();
        let mut meta = read_meta(dir).unwrap();
        meta.renders.push(RenderRecord {
            id: "r1".into(),
            file: "renders/one.mp4".into(),
            finished_ms: 1,
            bytes: 5,
            duration_ms: 1000,
            options: Value::Null,
            imported: false,
        });
        write_meta(dir, &meta).unwrap();

        let copy = fx.studio.duplicate(&a.dir).unwrap();
        assert_eq!(copy.name, "Film copy");
        assert_eq!(copy.renders, 0);
        assert_eq!(copy.lang.as_deref(), Some("ja"));
        let copied = Path::new(&copy.dir);
        assert_eq!(fs::read(copied.join("assets/nested/pic.png")).unwrap(), b"png");
        assert_eq!(fs::read_to_string(copied.join("source/input.txt")).unwrap(), "text");
        assert!(copied.join("renders").is_dir());
        assert!(!copied.join("renders/one.mp4").exists());
        assert_ne!(read_meta(copied).unwrap().id, meta.id);
        assert_eq!(fx.studio.duplicate(&a.dir).unwrap().name, "Film copy 2");
        assert_eq!(read_meta(dir).unwrap().renders.len(), 1, "the original keeps its history");
    }

    #[test]
    fn delete_moves_the_project_to_the_trash() {
        let fx = Fixture::new("trash");
        let a = fx.studio.create("Gone", None, "landscape").unwrap();
        fx.studio.open(&a.dir).unwrap();
        fs::write(Path::new(&a.dir).join("assets/pic.png"), b"png").unwrap();
        fx.studio.trash_project(&a.dir).unwrap();
        assert!(!Path::new(&a.dir).exists());
        assert_eq!(fx.trashed(), ["Gone"]);
        assert_eq!(
            fs::read(fx.root.join("trash/Gone/assets/pic.png")).unwrap(),
            b"png",
            "the files are moved, not removed"
        );
        assert!(fx.studio.list().is_empty());
        assert!(fx.studio.settings().recent.is_empty());
    }

    #[test]
    fn importing_a_timeline_copies_what_it_names_and_keeps_the_document() {
        let fx = Fixture::new("import");
        let old = fx.root.join("old");
        fs::create_dir_all(old.join("assets")).unwrap();
        fs::create_dir_all(fx.studio.public_dir.join("assets/music/library")).unwrap();
        fs::create_dir_all(fx.studio.public_dir.join("generated/illustrations")).unwrap();
        fs::write(old.join("assets/pic.png"), b"next to the timeline").unwrap();
        fs::write(fx.studio.public_dir.join("assets/pic.png"), b"public").unwrap();
        fs::write(fx.studio.public_dir.join("assets/logo.png"), b"logo").unwrap();
        fs::write(fx.studio.public_dir.join("assets/music/library/calm.mp3"), b"music").unwrap();
        fs::write(fx.studio.public_dir.join("generated/illustrations/a.png"), b"drawn").unwrap();
        let raw = r#"{ "fps": 30, "resolution": {"width": 1920, "height": 1080},
  "scenes": [
    {"id": "a", "duration_ms": 2000, "image": "assets/pic.png"},
    {"id": "b", "duration_ms": 3000, "image": "generated/illustrations/a.png"},
    {"id": "c", "duration_ms": 1000, "image": "assets/nowhere.png"},
    {"id": "d", "duration_ms": 1000, "image": "https://example.com/x.png"}
  ],
  "music": {"source": "assets/music/library/calm.mp3"},
  "brand": {"logo": {"src": "assets/logo.png"}} }"#;
        let file = old.join("my-video.timeline.json");
        fs::write(&file, raw).unwrap();

        let got = fx.studio.import_timeline(&file.display().to_string()).unwrap();
        assert_eq!(got.project.name, "my-video");
        assert_eq!(got.project.origin, "imported");
        assert_eq!(got.project.duration_ms, 7000);
        assert_eq!(got.copied, ["assets/pic.png", "generated/illustrations/a.png", "assets/logo.png"]);
        assert_eq!(got.missing, ["assets/nowhere.png"]);
        let dir = Path::new(&got.project.dir);
        assert_eq!(fs::read_to_string(dir.join(TIMELINE_FILE)).unwrap(), raw, "byte for byte");
        assert_eq!(fs::read(dir.join("assets/pic.png")).unwrap(), b"next to the timeline");
        assert_eq!(fs::read(dir.join("generated/illustrations/a.png")).unwrap(), b"drawn");
        assert!(!dir.join("assets/music").exists(), "library tracks stay in the public folder");
        assert_eq!(read_meta(dir).unwrap().imported_from.as_deref(), Some(file.display().to_string().as_str()));
        // Nothing was taken away from the old places.
        assert!(old.join("assets/pic.png").is_file() && file.is_file());
        assert!(fx.studio.public_dir.join("assets/logo.png").is_file());

        fs::write(old.join("notes.json"), r#"{"hello": 1}"#).unwrap();
        let err = fx.studio.import_timeline(&old.join("notes.json").display().to_string()).unwrap_err();
        assert!(err.contains("not a timeline"), "{err}");
        assert_eq!(fx.studio.list().len(), 1, "a failed import leaves no project");
    }
}
