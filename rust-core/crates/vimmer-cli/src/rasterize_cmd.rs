//! `vimmer rasterize`: sanitizes SVG files and rasterizes them exactly as an illustration run
//! would, without any API call. For checking that headless Chrome works on a machine and for
//! comparing the DevTools and `--screenshot` methods.

use std::ffi::OsString;
use std::path::PathBuf;
use std::process::ExitCode;
use std::time::{Duration, Instant};

use clap::{Parser, ValueEnum};
use vimmer_media::{
    find_chrome, preflight_chrome, rasterize_svg_with, sanitize_svg, Limits, RasterMethod,
};

/// Rasterize SVG files with headless Chrome (sanitized first, as for illustrations).
///
/// Writes `<out-dir>/<name>.png` for each `<name>.svg` and prints the method used and the time.
#[derive(Debug, Parser)]
#[command(name = "vimmer rasterize", version)]
struct RasterizeCli {
    /// SVG files.
    svgs: Vec<PathBuf>,
    /// Where the PNGs go (default: next to each SVG).
    #[arg(long)]
    out_dir: Option<PathBuf>,
    /// Canvas size, WIDTHxHEIGHT.
    #[arg(long, default_value = "1920x1080")]
    size: String,
    /// How to drive Chrome.
    #[arg(long, value_enum, default_value_t = Method::Auto)]
    method: Method,
    /// Run the 160x90 preflight the illustration step runs first.
    #[arg(long)]
    preflight: bool,
    /// Headless Chrome (default: $VIMMER_CHROME, else Remotion's chrome-headless-shell).
    #[arg(long)]
    chrome: Option<PathBuf>,
    /// The Remotion project, for finding its Chrome.
    #[arg(long, default_value = "remotion-renderer")]
    remotion_dir: PathBuf,
    /// Seconds per picture.
    #[arg(long, default_value_t = 60)]
    timeout: u64,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
enum Method {
    Auto,
    Devtools,
    Screenshot,
}

pub fn main(mut args: Vec<OsString>) -> ExitCode {
    args.remove(1);
    args[0] = "vimmer rasterize".into();
    match run(RasterizeCli::parse_from(args)) {
        Ok(true) => ExitCode::SUCCESS,
        Ok(false) => ExitCode::FAILURE,
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}

fn run(cli: RasterizeCli) -> Result<bool, Box<dyn std::error::Error>> {
    let (w, h) = cli
        .size
        .split_once('x')
        .and_then(|(w, h)| Some((w.parse::<u32>().ok()?, h.parse::<u32>().ok()?)))
        .ok_or("--size must look like 1920x1080")?;
    let chrome = match cli.chrome {
        Some(c) => c,
        None => find_chrome(&cli.remotion_dir).ok_or("no headless Chrome found")?,
    };
    let method = match cli.method {
        Method::Auto => RasterMethod::Auto,
        Method::Devtools => RasterMethod::DevTools,
        Method::Screenshot => RasterMethod::Screenshot,
    };
    let limits = Limits::none().with_timeout(Duration::from_secs(cli.timeout));
    let scratch = std::env::temp_dir().join(format!("vimmer-rasterize-{}", std::process::id()));
    std::fs::create_dir_all(&scratch)?;
    println!("chrome: {}", chrome.display());
    let mut ok = true;
    if cli.preflight {
        let started = Instant::now();
        match preflight_chrome(&chrome, &scratch.join("preflight"), &limits) {
            Ok(()) => println!("preflight: ok in {:.2} s", started.elapsed().as_secs_f64()),
            Err(e) => {
                println!(
                    "preflight: FAILED in {:.2} s: {e}",
                    started.elapsed().as_secs_f64()
                );
                ok = false;
            }
        }
    }
    for (i, svg) in cli.svgs.iter().enumerate() {
        let raw = std::fs::read_to_string(svg)?;
        let clean = match sanitize_svg(&raw, w, h) {
            Ok(c) => c,
            Err(e) => {
                println!("{}: rejected: {e}", svg.display());
                ok = false;
                continue;
            }
        };
        let stem = svg.file_stem().unwrap_or_default();
        let png = match &cli.out_dir {
            Some(dir) => {
                std::fs::create_dir_all(dir)?;
                dir.join(stem).with_extension("png")
            }
            None => svg.with_extension("png"),
        };
        let clean_path = scratch.join(format!("{i}.svg"));
        std::fs::write(&clean_path, clean.svg)?;
        let started = Instant::now();
        let result = rasterize_svg_with(
            method,
            &chrome,
            &clean_path,
            &png,
            w,
            h,
            &scratch.join(format!("profile-{i}")),
            &limits,
        );
        let secs = started.elapsed().as_secs_f64();
        match result {
            Ok(used) => println!(
                "{}: {} via {used} in {secs:.2} s",
                svg.display(),
                png.display()
            ),
            Err(e) => {
                println!("{}: FAILED in {secs:.2} s: {e}", svg.display());
                ok = false;
            }
        }
    }
    let _ = std::fs::remove_dir_all(&scratch);
    Ok(ok)
}
