//! The asset paths of a timeline, walked on the JSON as the editor holds it (so nothing else in
//! the document is touched when a path is rewritten). Visits the same fields as the scheduler's
//! `localize::for_each_asset_path`; a test keeps the two in step.

use serde_json::Value;

/// Who uses a path: a scene (by id), the music or the brand.
pub const USER_MUSIC: &str = "Music";
pub const USER_BRAND: &str = "Brand";

/// Calls `f(path, user)` on every asset path: scene images, audio, video clips, layer images,
/// icons and clips, sound effects, the music file, brand fonts and the logo.
pub fn for_each_asset(timeline: &mut Value, f: &mut dyn FnMut(&mut String, &str)) {
    if let Some(scenes) = timeline.get_mut("scenes").and_then(Value::as_array_mut) {
        for (index, scene) in scenes.iter_mut().enumerate() {
            let user = scene
                .get("id")
                .and_then(Value::as_str)
                .map(String::from)
                .unwrap_or_else(|| format!("scene {}", index + 1));
            visit(scene.get_mut("image"), &user, f);
            visit(scene.get_mut("audio").and_then(|a| a.get_mut("source")), &user, f);
            visit(scene.get_mut("video").and_then(|v| v.get_mut("src")), &user, f);
            if let Some(layers) = scene.get_mut("layers").and_then(Value::as_array_mut) {
                for layer in layers {
                    let kind = layer.get("kind").and_then(Value::as_str).unwrap_or("");
                    if matches!(kind, "image" | "icon" | "video") {
                        visit(layer.get_mut("src"), &user, f);
                    }
                }
            }
            if let Some(sfx) = scene.get_mut("sfx").and_then(Value::as_array_mut) {
                for fx in sfx {
                    visit(fx.get_mut("src"), &user, f);
                }
            }
        }
    }
    visit(timeline.get_mut("music").and_then(|m| m.get_mut("source")), USER_MUSIC, f);
    if let Some(brand) = timeline.get_mut("brand") {
        if let Some(fonts) = brand.get_mut("fonts") {
            for role in ["heading", "body"] {
                visit(fonts.get_mut(role).and_then(|font| font.get_mut("src")), USER_BRAND, f);
            }
        }
        visit(brand.get_mut("logo").and_then(|l| l.get_mut("src")), USER_BRAND, f);
    }
}

fn visit(value: Option<&mut Value>, user: &str, f: &mut dyn FnMut(&mut String, &str)) {
    if let Some(Value::String(path)) = value {
        f(path, user);
    }
}

/// True for a path that names a local file relative to a project or the public dir.
pub fn is_local_relative(path: &str) -> bool {
    vimmer_scheduler::localize::is_local_relative(path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn visits_what_the_scheduler_visits_on_every_fixture() {
        let fixtures = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures");
        let mut checked = 0;
        let mut paths = 0;
        for entry in std::fs::read_dir(fixtures).unwrap() {
            let path = entry.unwrap().path();
            let raw = std::fs::read_to_string(&path).unwrap();
            // Brand files are not timelines.
            let Ok(mut typed) = serde_json::from_str::<vimmer_dsl::VideoTimeline>(&raw) else { continue };
            let mut expected = Vec::new();
            vimmer_scheduler::localize::for_each_asset_path(&mut typed, &mut |p| expected.push(p.clone()));
            let mut value: Value = serde_json::from_str(&raw).unwrap();
            let mut got = Vec::new();
            for_each_asset(&mut value, &mut |p, _| got.push(p.clone()));
            assert_eq!(got, expected, "{}", path.display());
            checked += 1;
            paths += got.len();
        }
        assert!(checked >= 15 && paths >= 20, "{checked} fixtures, {paths} paths");
    }

    #[test]
    fn names_the_user_and_rewrites_in_place() {
        let mut t: Value = serde_json::from_str(
            r#"{"scenes":[{"id":"intro","image":"assets/a.png","image_fit":"cover",
                 "layers":[{"kind":"text","src":"not-an-asset"},{"kind":"icon","src":"assets/i.svg"}]},
                {"image":null,"sfx":[{"src":"assets/pop.wav"}]}],
                "music":{"source":"assets/m.mp3","volume":0.5},
                "brand":{"fonts":{"body":{"family":"B","src":"assets/f.woff2"}},"logo":{"src":"assets/logo.png"}}}"#,
        )
        .unwrap();
        let mut seen = Vec::new();
        for_each_asset(&mut t, &mut |p, user| {
            seen.push(format!("{user}:{p}"));
            *p = format!("x/{p}");
        });
        assert_eq!(
            seen,
            [
                "intro:assets/a.png",
                "intro:assets/i.svg",
                "scene 2:assets/pop.wav",
                "Music:assets/m.mp3",
                "Brand:assets/f.woff2",
                "Brand:assets/logo.png"
            ]
        );
        assert_eq!(t["scenes"][0]["image"], "x/assets/a.png");
        assert_eq!(t["scenes"][0]["layers"][0]["src"], "not-an-asset");
        assert_eq!(t["scenes"][0]["image_fit"], "cover");
        assert_eq!(t["music"]["volume"], 0.5);
    }
}
