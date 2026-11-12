//! Media files used by the timeline. They are copied into the Remotion public folder, where the
//! renderer can read them, and the timeline stores the public-relative path.

use std::fs;
use std::path::{Path, PathBuf};

use tauri::Manager;

/// The Remotion public folder: where timeline assets live.
pub fn public_dir() -> PathBuf {
    let here = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../remotion-renderer/public");
    here.canonicalize().unwrap_or(here)
}

/// Absolute path of the public folder, so the webview can build asset URLs.
#[tauri::command]
pub fn asset_root() -> Result<String, String> {
    Ok(public_dir().display().to_string())
}

/// Copies `source` into `public/assets/` and returns its public-relative path (`assets/<name>`).
/// If that name is taken by a different file, a numeric suffix is added. Identical files are reused.
#[tauri::command]
pub fn import_asset(source: String) -> Result<String, String> {
    import_into(&public_dir(), Path::new(&source))
}

/// The copy logic, for a given public folder.
pub fn import_into(public: &Path, src: &Path) -> Result<String, String> {
    let source = src.display().to_string();
    let name = src
        .file_name()
        .and_then(|n| n.to_str())
        .ok_or_else(|| format!("Not a file name: {source}"))?
        .to_string();
    let dir = public.join("assets");
    fs::create_dir_all(&dir).map_err(|e| format!("Could not create {}: {e}", dir.display()))?;
    let bytes = fs::read(&src).map_err(|e| format!("Could not read {source}: {e}"))?;

    let (stem, ext) = match name.rsplit_once('.') {
        Some((s, e)) => (s.to_string(), format!(".{e}")),
        None => (name.clone(), String::new()),
    };
    let mut candidate = name.clone();
    let mut n = 2;
    loop {
        let target = dir.join(&candidate);
        match fs::read(&target) {
            Err(_) => {
                fs::write(&target, &bytes).map_err(|e| format!("Could not copy to {}: {e}", target.display()))?;
                return Ok(format!("assets/{candidate}"));
            }
            Ok(existing) if existing == bytes => return Ok(format!("assets/{candidate}")),
            Ok(_) => {
                candidate = format!("{stem}-{n}{ext}");
                n += 1;
            }
        }
    }
}

/// Lets the webview load files from the public folder through Tauri's asset protocol.
pub fn allow_public_dir(app: &tauri::App) {
    let _ = app.asset_protocol_scope().allow_directory(public_dir(), true);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("vimmer-import-{}-{name}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn copies_into_assets_and_reuses_identical_files() {
        let root = scratch("reuse");
        let public = root.join("public");
        let src = root.join("pic.png");
        fs::write(&src, b"same").unwrap();
        assert_eq!(import_into(&public, &src).unwrap(), "assets/pic.png");
        assert_eq!(fs::read(public.join("assets/pic.png")).unwrap(), b"same");
        // Same name, same bytes: no new copy.
        assert_eq!(import_into(&public, &src).unwrap(), "assets/pic.png");
        assert!(!public.join("assets/pic-2.png").exists());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn suffixes_a_different_file_with_the_same_name() {
        let root = scratch("clash");
        let public = root.join("public");
        let first = root.join("a").join("pic.png");
        let second = root.join("b").join("pic.png");
        fs::create_dir_all(first.parent().unwrap()).unwrap();
        fs::create_dir_all(second.parent().unwrap()).unwrap();
        fs::write(&first, b"one").unwrap();
        fs::write(&second, b"two").unwrap();
        assert_eq!(import_into(&public, &first).unwrap(), "assets/pic.png");
        assert_eq!(import_into(&public, &second).unwrap(), "assets/pic-2.png");
        assert_eq!(fs::read(public.join("assets/pic-2.png")).unwrap(), b"two");
        let _ = fs::remove_dir_all(root);
    }
}
