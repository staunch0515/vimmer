//! `vimmer expand`: prints a timeline with its template scenes expanded into layers (the brand
//! applied as for a render). The editor's dev server calls it to preview template scenes with the
//! same code the renderer uses.

use std::ffi::OsString;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use clap::Parser;
use serde_json::json;
use vimmer_dsl::{expand_templates, Brand, VideoTimeline};

/// Expand template scenes into ordinary layers and print the timeline as JSON.
///
/// Prints `{"ok":true,"timeline":{...}}`, or `{"ok":false,"errors":[...]}` with exit code 1 when
/// a template's params are wrong. Exit code 2 when the input cannot be read or parsed.
#[derive(Debug, Parser)]
#[command(name = "vimmer expand", version)]
struct ExpandCli {
    /// Timeline JSON, or `-` for standard input.
    timeline: PathBuf,
    /// Brand JSON that replaces the timeline's `brand`.
    #[arg(long)]
    brand: Option<PathBuf>,
}

/// Reads a brand file (the `brand` object of a timeline, on its own).
pub fn load_brand(path: &Path) -> Result<Brand, String> {
    let raw = std::fs::read_to_string(path)
        .map_err(|e| format!("cannot read brand {}: {e}", path.display()))?;
    let brand: Brand = serde_json::from_str(&raw)
        .map_err(|e| format!("cannot parse brand {}: {e}", path.display()))?;
    let problems = brand.problems();
    if !problems.is_empty() {
        return Err(format!(
            "brand {} is invalid: {}",
            path.display(),
            problems.join("; ")
        ));
    }
    Ok(brand)
}

pub fn main(mut args: Vec<OsString>) -> ExitCode {
    args.remove(1);
    args[0] = "vimmer expand".into();
    let cli = ExpandCli::parse_from(args);
    let raw = if cli.timeline.as_os_str() == "-" {
        let mut s = String::new();
        std::io::stdin().read_to_string(&mut s).map(|_| s)
    } else {
        std::fs::read_to_string(&cli.timeline)
    };
    let fail = |message: String| {
        println!("{}", json!({ "ok": false, "errors": [message] }));
        ExitCode::from(2)
    };
    let raw = match raw {
        Ok(r) => r,
        Err(e) => return fail(format!("cannot read the timeline: {e}")),
    };
    let mut timeline: VideoTimeline = match serde_json::from_str(&raw) {
        Ok(t) => t,
        Err(e) => return fail(format!("cannot parse the timeline: {e}")),
    };
    if let Some(path) = &cli.brand {
        match load_brand(path) {
            Ok(b) => timeline.brand = Some(b),
            Err(e) => return fail(e),
        }
    }
    match expand_templates(&mut timeline) {
        Ok(()) => {
            println!("{}", json!({ "ok": true, "timeline": timeline }));
            ExitCode::SUCCESS
        }
        Err(errors) => {
            let errors: Vec<String> = errors.iter().map(ToString::to_string).collect();
            println!("{}", json!({ "ok": false, "errors": errors }));
            ExitCode::from(1)
        }
    }
}
