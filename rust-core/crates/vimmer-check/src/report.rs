//! Human-readable output, grouped by scene.

use std::fmt::Write;

use vimmer_dsl::VideoTimeline;

use crate::{scene_starts_ms, Finding, Report};

/// The scene a finding belongs to: its own scene, or the scene a cue starts in.
fn scene_index(timeline: &VideoTimeline, starts: &[u64], f: &Finding) -> Option<usize> {
    if let Some(id) = &f.scene_id {
        return timeline.scenes.iter().position(|s| &s.id == id);
    }
    let cue = timeline.subtitles.get(f.cue_index?)?;
    let i = starts.iter().rposition(|s| *s <= cue.start_ms)?;
    Some(i)
}

fn time(ms: u64) -> String {
    format!("{}:{:02}.{:03}", ms / 60_000, (ms / 1000) % 60, ms % 1000)
}

pub fn render_human(report: &Report, timeline: &VideoTimeline) -> String {
    let mut out = String::new();
    let starts = scene_starts_ms(timeline);
    let mut groups: Vec<Vec<&Finding>> = vec![Vec::new(); timeline.scenes.len() + 1];
    for f in &report.findings {
        let g = scene_index(timeline, &starts, f).map_or(0, |i| i + 1);
        groups[g].push(f);
    }
    for (g, findings) in groups.iter().enumerate() {
        if findings.is_empty() {
            continue;
        }
        if g == 0 {
            let _ = writeln!(out, "timeline");
        } else {
            let scene = &timeline.scenes[g - 1];
            let _ = writeln!(
                out,
                "scene `{}` ({}–{})",
                scene.id,
                time(starts[g - 1]),
                time(starts[g - 1] + scene.duration_ms)
            );
        }
        for f in findings {
            let mut place = String::new();
            if let Some(c) = f.cue_index {
                let _ = write!(place, "subtitles[{c}] ");
            }
            if let Some(t) = f.time_ms {
                let _ = write!(place, "@{} ", time(t));
            }
            let _ = writeln!(
                out,
                "  {:<7} {}{}: {}",
                f.severity.label(),
                place,
                f.code,
                f.message
            );
            let _ = write!(out, "          fix: {}", f.suggestion.text);
            if let Some(path) = &f.suggestion.path {
                match &f.suggestion.value {
                    Some(v) => {
                        let v = v.to_string();
                        let shown = if v.chars().count() > 160 {
                            format!("{}…", v.chars().take(160).collect::<String>())
                        } else {
                            v
                        };
                        let _ = write!(out, " [{path} = {shown}]");
                    }
                    None => {
                        let _ = write!(out, " [{path}]");
                    }
                }
            }
            out.push('\n');
        }
    }
    let s = report.summary;
    let _ = writeln!(
        out,
        "{} error(s), {} warning(s), {} info",
        s.errors, s.warnings, s.infos
    );
    out
}
