//! File access for project documents. The webview asks for a path through a native dialog,
//! then reads and writes it here, so the frontend never gets general filesystem access.

use std::fs;
use std::path::{Path, PathBuf};

/// Expands the template scenes of a timeline (the same Rust code a render runs), for the editor's
/// preview. Returns `{"ok": true, "timeline": ...}` or `{"ok": false, "errors": [...]}`.
#[tauri::command]
pub fn expand_timeline(timeline_json: String) -> serde_json::Value {
    let mut timeline: vimmer_dsl::VideoTimeline = match serde_json::from_str(&timeline_json) {
        Ok(t) => t,
        Err(e) => return serde_json::json!({ "ok": false, "errors": [format!("the timeline does not parse: {e}")] }),
    };
    match vimmer_dsl::expand_templates(&mut timeline) {
        Ok(()) => serde_json::json!({ "ok": true, "timeline": timeline }),
        Err(errors) => {
            let errors: Vec<String> = errors.iter().map(ToString::to_string).collect();
            serde_json::json!({ "ok": false, "errors": errors })
        }
    }
}

/// Reads a UTF-8 text file.
#[tauri::command]
pub fn read_project_file(path: String) -> Result<String, String> {
    fs::read_to_string(&path).map_err(|e| format!("Could not read {path}: {e}"))
}

/// Writes text atomically: the content goes to a sibling temp file first, then replaces the target,
/// so a crash mid-write cannot leave a half-written project behind.
#[tauri::command]
pub fn write_project_file(path: String, contents: String) -> Result<(), String> {
    write_atomic(Path::new(&path), contents.as_bytes()).map_err(|e| format!("Could not save {path}: {e}"))
}

fn write_atomic(target: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let dir = target.parent().filter(|p| !p.as_os_str().is_empty()).unwrap_or(Path::new("."));
    let name = target.file_name().and_then(|n| n.to_str()).unwrap_or("project");
    let tmp: PathBuf = dir.join(format!(".{name}.tmp-{}", std::process::id()));
    fs::write(&tmp, bytes)?;
    fs::rename(&tmp, target).inspect_err(|_| {
        let _ = fs::remove_file(&tmp);
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("vimmer-project-test-{}-{name}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn templates_are_expanded_for_the_preview() {
        let raw = r#"{"fps":30,"resolution":{"width":1920,"height":1080},"scenes":[{"id":"q","duration_ms":5000,"template":{"name":"quote","params":{"quote":"Q"}}}]}"#;
        let out = expand_timeline(raw.into());
        assert_eq!(out["ok"], true);
        assert!(out["timeline"]["scenes"][0]["layers"].as_array().unwrap().len() >= 3);
        assert!(out["timeline"]["scenes"][0].get("template").is_none());
        let bad = expand_timeline(raw.replace(r#""quote":"Q""#, r#""colour":"red""#));
        assert_eq!(bad["ok"], false);
        assert!(bad["errors"][0].as_str().unwrap().contains("unknown param `colour`"));
    }

    #[test]
    fn write_then_read_round_trips() {
        let dir = scratch("roundtrip");
        let path = dir.join("timeline.json");
        let body = "{\n  \"fps\": 30\n}\n";
        write_project_file(path.display().to_string(), body.into()).unwrap();
        assert_eq!(read_project_file(path.display().to_string()).unwrap(), body);
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn overwrite_replaces_content_and_leaves_no_temp_file() {
        let dir = scratch("overwrite");
        let path = dir.join("timeline.json");
        write_project_file(path.display().to_string(), "first".into()).unwrap();
        write_project_file(path.display().to_string(), "second".into()).unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), "second");
        let leftovers: Vec<_> = fs::read_dir(&dir).unwrap().map(|e| e.unwrap().file_name()).collect();
        assert_eq!(leftovers.len(), 1, "only the project file should remain: {leftovers:?}");
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn missing_file_reports_the_path() {
        let err = read_project_file("/nonexistent/vimmer/none.json".into()).unwrap_err();
        assert!(err.contains("/nonexistent/vimmer/none.json"));
    }
}
