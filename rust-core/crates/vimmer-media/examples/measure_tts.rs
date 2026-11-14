//! Measures how long each local speech engine takes to read the calibration corpus.
//!
//! ```text
//! cargo run -p vimmer-media --example measure_tts --release -- \
//!     [--corpus calibration/corpus.tsv] [--out calibration/measured.csv] \
//!     [--cache DIR] [--engines say,kokoro]
//! ```
//!
//! Each sentence is synthesized exactly as the render's narration step does it: `say` with
//! [`default_voice_for`] and Kokoro with [`KokoroLang::from_text`]'s default voice, both through
//! the same [`SaySpeech`] / [`KokoroSpeech`] code, then measured with [`FfmpegProbe`], i.e. the
//! whole file length the scheduler compares with the scene (no silence trimming: leading and
//! trailing silence count, because the render waits for them too).
//!
//! Clips are cached under `--cache` (default `$TMPDIR/vimmer-tts-calibration`) by engine,
//! voice and text, so a re-run only synthesizes what changed. Delete the cache after a voice or
//! model update. Paths are relative to this crate's directory.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use vimmer_media::{
    default_voice_for, AudioProbe, FfmpegProbe, KokoroItem, KokoroLang, KokoroSpeech, Limits,
    SaySpeech, Speech,
};

struct Row {
    id: String,
    lang: String,
    text: String,
}

fn main() {
    if let Err(e) = run() {
        eprintln!("error: {e}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), Box<dyn std::error::Error>> {
    let crate_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut corpus = crate_dir.join("calibration/corpus.tsv");
    let mut out = crate_dir.join("calibration/measured.csv");
    let mut cache = std::env::temp_dir().join("vimmer-tts-calibration");
    let mut engines = vec!["say".to_string(), "kokoro".to_string()];
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        let mut value = || args.next().ok_or(format!("{a} needs a value"));
        match a.as_str() {
            "--corpus" => corpus = PathBuf::from(value()?),
            "--out" => out = PathBuf::from(value()?),
            "--cache" => cache = PathBuf::from(value()?),
            "--engines" => engines = value()?.split(',').map(str::to_string).collect(),
            _ => return Err(format!("unknown argument `{a}`").into()),
        }
    }

    let rows = read_corpus(&corpus)?;
    let probe = FfmpegProbe::default();
    let mut csv = String::from("id,lang,engine,voice,ms,text\n");

    if engines.iter().any(|e| e == "say") {
        let say = SaySpeech::default();
        for (n, row) in rows.iter().enumerate() {
            let voice = default_voice_for(&row.text);
            let wav = cache.join(format!(
                "say-{:016x}.wav",
                fnv1a(&format!("{voice}\n{}", row.text))
            ));
            if !wav.is_file() {
                eprintln!("say {}/{}: {}", n + 1, rows.len(), row.id);
                say.synthesize(&row.text, voice, &wav)?;
            }
            push(&mut csv, row, "say", voice, probe.duration_ms(&wav)?);
        }
    }

    if engines.iter().any(|e| e == "kokoro") {
        let kokoro = KokoroSpeech::locate(None, None, None)?;
        let jobs: Vec<(&Row, KokoroItem)> = rows
            .iter()
            .map(|row| {
                let lang = KokoroLang::from_text(&row.text);
                let voice = lang.default_voice().to_string();
                let key = kokoro.cache_key(&row.text, lang, &voice);
                let out = cache.join(format!("kokoro-{key:016x}.wav"));
                (
                    row,
                    KokoroItem {
                        text: row.text.clone(),
                        lang,
                        voice,
                        out,
                    },
                )
            })
            .collect();
        let missing: Vec<KokoroItem> = jobs
            .iter()
            .filter(|(_, item)| !item.out.is_file())
            .map(|(_, item)| item.clone())
            .collect();
        if !missing.is_empty() {
            eprintln!("kokoro: synthesizing {} clip(s)", missing.len());
            for w in kokoro.synthesize_batch(&missing, &cache, &Limits::none())? {
                eprintln!("kokoro warning: {w}");
            }
        }
        for (row, item) in &jobs {
            push(
                &mut csv,
                row,
                "kokoro",
                &item.voice,
                probe.duration_ms(&item.out)?,
            );
        }
    }

    std::fs::write(&out, csv)?;
    eprintln!("wrote {}", out.display());
    Ok(())
}

fn read_corpus(path: &Path) -> Result<Vec<Row>, Box<dyn std::error::Error>> {
    let raw = std::fs::read_to_string(path)
        .map_err(|e| format!("cannot read {}: {e}", path.display()))?;
    let mut rows = Vec::new();
    for line in raw
        .lines()
        .filter(|l| !l.trim().is_empty() && !l.starts_with('#'))
    {
        let mut parts = line.splitn(3, '\t');
        match (parts.next(), parts.next(), parts.next()) {
            (Some(id), Some(lang), Some(text)) => rows.push(Row {
                id: id.into(),
                lang: lang.into(),
                text: text.into(),
            }),
            _ => return Err(format!("bad corpus line: {line}").into()),
        }
    }
    Ok(rows)
}

fn push(csv: &mut String, row: &Row, engine: &str, voice: &str, ms: u64) {
    let text = row.text.replace('"', "\"\"");
    let _ = writeln!(
        csv,
        "{},{},{engine},{voice},{ms},\"{text}\"",
        row.id, row.lang
    );
}

fn fnv1a(s: &str) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in s.bytes() {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x0100_0000_01b3);
    }
    hash
}
