//! Kokoro narration: a local neural TTS (Kokoro-82M, ONNX) run through a Python helper.
//!
//! The helper (`scripts/kokoro_tts.py`, embedded in this crate) takes a whole render's clips in
//! one JSON batch, so its models load once per render rather than once per scene. It writes
//! 24 kHz mono WAVs; each is then converted to the 48 kHz stereo WAV that [`Speech`] promises.

use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use crate::process::{run_limited, Limits};
use crate::speech::Speech;
use crate::transcribe::{find_executable, guess_language};
use crate::MediaError;

/// Selects the narration engine (`say` or `kokoro`) when no explicit choice is made.
pub const ENV_TTS: &str = "VIMMER_TTS";
/// Python interpreter that has kokoro-onnx and misaki installed.
pub const ENV_KOKORO_PYTHON: &str = "VIMMER_KOKORO_PYTHON";
/// Directory holding the Kokoro model and voice files.
pub const ENV_KOKORO_MODELS: &str = "VIMMER_KOKORO_MODELS";
/// Default interpreter: the venv made by `scripts/install_kokoro.sh`.
pub const DEFAULT_KOKORO_PYTHON_IN_HOME: &str = ".local/share/vimmer/kokoro-venv/bin/python";
/// Default model directory, filled by `scripts/install_kokoro.sh`.
pub const DEFAULT_KOKORO_MODELS_IN_HOME: &str = ".local/share/vimmer/models/kokoro";
/// The batch helper, written next to the system temp dir at run time.
pub const KOKORO_SCRIPT: &str = include_str!("../scripts/kokoro_tts.py");
/// Speaking rate handed to Kokoro. Fixed for now; part of the cache key all the same.
pub const KOKORO_SPEED: &str = "1.0";

/// Time budget for one Kokoro batch: loading models and imports, plus a generous allowance per
/// character. Measured on a 4-core Intel CPU: about 25 s of start-up, then roughly real time
/// (a Chinese character is about 0.25 s of speech).
const TIMEOUT_BASE: Duration = Duration::from_secs(180);
const TIMEOUT_PER_CHAR: Duration = Duration::from_secs(1);
/// Time budget for converting one clip with ffmpeg.
const CONVERT_TIMEOUT: Duration = Duration::from_secs(60);

/// A narration engine.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TtsEngine {
    /// macOS `say` (the default).
    Say,
    /// Kokoro through the Python helper.
    Kokoro,
}

impl TtsEngine {
    /// Parses `say` or `kokoro`, ignoring case and surrounding space.
    pub fn parse(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "say" => Some(Self::Say),
            "kokoro" => Some(Self::Kokoro),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Say => "say",
            Self::Kokoro => "kokoro",
        }
    }
}

/// Languages Kokoro narrates. Each one is tied to a model.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum KokoroLang {
    Zh,
    Ja,
    En,
}

impl KokoroLang {
    /// Judged by the same rule as recognition: kana means Japanese, other CJK ideographs mean
    /// Chinese, anything else is read as English.
    pub fn from_text(text: &str) -> Self {
        match guess_language(text) {
            Some("ja") => Self::Ja,
            Some("zh") => Self::Zh,
            _ => Self::En,
        }
    }

    pub fn code(self) -> &'static str {
        match self {
            Self::Zh => "zh",
            Self::Ja => "ja",
            Self::En => "en",
        }
    }

    /// The voice used when a scene names none (or names one Kokoro cannot use).
    pub fn default_voice(self) -> &'static str {
        match self {
            Self::Zh => "zf_001",
            Self::Ja => "jf_alpha",
            Self::En => "af_heart",
        }
    }

    /// Model, voice pack and (for v1.1-zh) vocabulary config, as file names in the model dir.
    /// Chinese needs v1.1-zh: v1.0's Chinese G2P runs syllables together.
    pub fn model_files(self) -> &'static [&'static str] {
        match self {
            Self::Zh => &[
                "kokoro-v1.1-zh.onnx",
                "voices-v1.1-zh.bin",
                "config-v1.1-zh.json",
            ],
            Self::Ja | Self::En => &["kokoro-v1.0.onnx", "voices-v1.0.bin"],
        }
    }

    /// Kokoro voice names start with a language letter: `z` Chinese, `j` Japanese,
    /// `a`/`b` American/British English.
    fn speaks(self, voice: &str) -> bool {
        match self {
            Self::Zh => voice.starts_with('z'),
            Self::Ja => voice.starts_with('j'),
            Self::En => voice.starts_with('a') || voice.starts_with('b'),
        }
    }
}

/// True for names shaped like Kokoro voices: a language letter, `f` or `m`, `_`, then lowercase
/// letters or digits (`zf_001`, `jf_alpha`, `af_heart`). `say` voices are capitalized.
pub fn looks_like_kokoro_voice(voice: &str) -> bool {
    let b = voice.as_bytes();
    b.len() >= 4
        && b[0].is_ascii_lowercase()
        && matches!(b[1], b'f' | b'm')
        && b[2] == b'_'
        && b[3..]
            .iter()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
}

/// The Kokoro voice for a scene, and a warning when the requested one cannot be used.
pub fn resolve_kokoro_voice(requested: Option<&str>, lang: KokoroLang) -> (String, Option<String>) {
    let default = lang.default_voice();
    let Some(voice) = requested.map(str::trim).filter(|v| !v.is_empty()) else {
        return (default.to_string(), None);
    };
    if looks_like_kokoro_voice(voice) {
        if lang.speaks(voice) {
            return (voice.to_string(), None);
        }
        return (
            default.to_string(),
            Some(format!(
                "voice `{voice}` is not a {} Kokoro voice, using {default}",
                lang.code()
            )),
        );
    }
    (
        default.to_string(),
        Some(format!(
            "voice `{voice}` is a `say` voice, Kokoro uses {default} for {} instead",
            lang.code()
        )),
    )
}

/// One clip to synthesize.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KokoroItem {
    pub text: String,
    pub lang: KokoroLang,
    /// A Kokoro voice name (see [`resolve_kokoro_voice`]).
    pub voice: String,
    /// Final 48 kHz stereo WAV.
    pub out: PathBuf,
}

impl KokoroItem {
    /// Where the helper writes its 24 kHz mono WAV before conversion.
    fn raw_path(&self) -> PathBuf {
        self.out.with_extension("kokoro24k.wav")
    }
}

/// Kokoro engine: a Python interpreter, the helper script and the model directory.
#[derive(Debug, Clone)]
pub struct KokoroSpeech {
    pub python: PathBuf,
    pub models: PathBuf,
    /// Where the embedded helper is written before it runs.
    pub script: PathBuf,
    pub ffmpeg: PathBuf,
    /// PATH for the children (a GUI app's own PATH can be minimal).
    pub path_var: Option<String>,
}

impl KokoroSpeech {
    /// Locates the engine. The interpreter comes from `python`, else `$VIMMER_KOKORO_PYTHON`,
    /// else the default venv; the models from `models`, else `$VIMMER_KOKORO_MODELS`, else the
    /// default directory. Errors say what is missing and how to fix it.
    pub fn locate(
        python: Option<&Path>,
        models: Option<&Path>,
        path_var: Option<&str>,
    ) -> Result<Self, MediaError> {
        let home = std::env::var_os("HOME").map(PathBuf::from);
        let python_name = python
            .map(|p| p.to_string_lossy().into_owned())
            .or_else(|| env_nonempty(ENV_KOKORO_PYTHON))
            .or_else(|| {
                home.as_ref().map(|h| {
                    h.join(DEFAULT_KOKORO_PYTHON_IN_HOME)
                        .to_string_lossy()
                        .into_owned()
                })
            })
            .unwrap_or_else(|| "python3".to_string());
        let python = find_executable(&python_name, path_var).ok_or_else(|| {
            MediaError::Speech(format!(
                "Kokoro python `{python_name}` was not found (run scripts/install_kokoro.sh, or pass --kokoro-python / set {ENV_KOKORO_PYTHON})"
            ))
        })?;
        let models = models
            .map(Path::to_path_buf)
            .or_else(|| env_nonempty(ENV_KOKORO_MODELS).map(PathBuf::from))
            .or_else(|| home.map(|h| h.join(DEFAULT_KOKORO_MODELS_IN_HOME)))
            .ok_or_else(|| MediaError::Speech("no Kokoro model directory configured".into()))?;
        if !models.is_dir() {
            return Err(MediaError::Speech(format!(
                "Kokoro model directory `{}` does not exist (run scripts/install_kokoro.sh, or set {ENV_KOKORO_MODELS})",
                models.display()
            )));
        }
        Ok(Self {
            python,
            models,
            script: default_script_path(),
            ffmpeg: PathBuf::from("ffmpeg"),
            path_var: path_var.map(str::to_string),
        })
    }

    /// Checks that every model file the given languages need is present.
    pub fn check_models(&self, langs: &[KokoroLang]) -> Result<(), MediaError> {
        for lang in langs {
            for file in lang.model_files() {
                if !self.models.join(file).is_file() {
                    return Err(MediaError::Speech(format!(
                        "Kokoro model file `{}` is missing (needed for {}; run scripts/install_kokoro.sh)",
                        self.models.join(file).display(),
                        lang.code()
                    )));
                }
            }
        }
        Ok(())
    }

    /// Cache key of one clip: engine, model files (name and size), voice, language, speed and
    /// text, so changing any of them synthesizes the clip again.
    pub fn cache_key(&self, text: &str, lang: KokoroLang, voice: &str) -> u64 {
        let mut key = String::from("kokoro\n");
        for file in lang.model_files() {
            let size = fs::metadata(self.models.join(file))
                .map(|m| m.len())
                .unwrap_or(0);
            let _ = writeln!(key, "{file}:{size}");
        }
        let _ = write!(key, "{voice}\n{}\n{KOKORO_SPEED}\n{text}", lang.code());
        fnv1a(&key)
    }

    /// The helper invocation for a batch file. `-I` keeps the helper's directory and the
    /// current directory off `sys.path` and ignores `PYTHON*` variables.
    pub fn command(&self, batch_file: &Path) -> Command {
        let mut cmd = Command::new(&self.python);
        cmd.arg("-I")
            .arg(&self.script)
            .arg("--models")
            .arg(&self.models)
            .arg(batch_file);
        if let Some(path) = &self.path_var {
            cmd.env("PATH", path);
        }
        cmd
    }

    /// Synthesizes every item in one helper run, then converts each clip to 48 kHz stereo.
    /// `work_dir` holds the batch file. Returns the helper's warnings. The run is killed when
    /// `limits` say so; a cancel returns [`MediaError::Cancelled`].
    pub fn synthesize_batch(
        &self,
        items: &[KokoroItem],
        work_dir: &Path,
        limits: &Limits,
    ) -> Result<Vec<String>, MediaError> {
        if items.is_empty() {
            return Ok(Vec::new());
        }
        self.write_script()?;
        fs::create_dir_all(work_dir)?;
        for item in items {
            if let Some(dir) = item.out.parent() {
                fs::create_dir_all(dir)?;
            }
        }
        let batch = work_dir.join("kokoro-batch.json");
        fs::write(&batch, batch_json(items))?;
        let output = run_limited(self.command(&batch), limits);
        let _ = fs::remove_file(&batch);
        let output = output?;
        if !output.status.success() {
            return Err(MediaError::Speech(format!(
                "kokoro helper exited with {}: {}",
                output.status,
                tail(&String::from_utf8_lossy(&output.stderr), 3)
            )));
        }
        let warnings: Vec<String> = String::from_utf8_lossy(&output.stdout)
            .lines()
            .filter_map(|l| l.strip_prefix("warning: "))
            .map(str::to_string)
            .collect();

        let convert_limits = limits.with_timeout(CONVERT_TIMEOUT);
        for item in items {
            let raw = item.raw_path();
            if !raw.is_file() {
                return Err(MediaError::Speech(format!(
                    "kokoro helper wrote no audio for `{}`",
                    item.out.display()
                )));
            }
            let converted = self.convert(&raw, &item.out, &convert_limits);
            let _ = fs::remove_file(&raw);
            converted?;
        }
        Ok(warnings)
    }

    /// 24 kHz mono → 48 kHz stereo, written under a temporary name and renamed, so a cached
    /// file is always complete.
    fn convert(&self, raw: &Path, out: &Path, limits: &Limits) -> Result<(), MediaError> {
        let part = out.with_extension("wav.part");
        let mut cmd = Command::new(&self.ffmpeg);
        cmd.args(["-y", "-hide_banner", "-loglevel", "error", "-i"])
            .arg(raw)
            .args(["-ac", "2", "-ar", "48000", "-f", "wav"])
            .arg(&part);
        if let Some(path) = &self.path_var {
            cmd.env("PATH", path);
        }
        let output = run_limited(cmd, limits)?;
        if !output.status.success() || !part.is_file() {
            let _ = fs::remove_file(&part);
            return Err(MediaError::Speech(format!(
                "ffmpeg could not convert Kokoro audio: {}",
                tail(&String::from_utf8_lossy(&output.stderr), 3)
            )));
        }
        fs::rename(&part, out)?;
        Ok(())
    }

    /// Writes the embedded helper (named after its content, so versions never mix).
    fn write_script(&self) -> Result<(), MediaError> {
        if fs::read_to_string(&self.script).is_ok_and(|s| s == KOKORO_SCRIPT) {
            return Ok(());
        }
        if let Some(dir) = self.script.parent() {
            fs::create_dir_all(dir)?;
        }
        let tmp = self
            .script
            .with_extension(format!("{}.tmp", std::process::id()));
        fs::write(&tmp, KOKORO_SCRIPT)?;
        fs::rename(&tmp, &self.script)?;
        Ok(())
    }
}

impl Speech for KokoroSpeech {
    /// One clip in a batch of its own. Prefer [`KokoroSpeech::synthesize_batch`]: every call
    /// pays the model loading again.
    fn synthesize(&self, text: &str, voice: &str, out_wav: &Path) -> Result<(), MediaError> {
        let lang = KokoroLang::from_text(text);
        let (voice, _) = resolve_kokoro_voice(Some(voice), lang);
        let item = KokoroItem {
            text: text.to_string(),
            lang,
            voice,
            out: out_wav.to_path_buf(),
        };
        let work = out_wav.parent().unwrap_or(Path::new("."));
        let limits = Limits::none().with_timeout(batch_timeout([text]));
        self.synthesize_batch(std::slice::from_ref(&item), work, &limits)
            .map(drop)
    }
}

/// Time budget for a batch holding these texts.
pub fn batch_timeout<'a>(texts: impl IntoIterator<Item = &'a str>) -> Duration {
    let chars: u32 = texts.into_iter().map(|t| t.chars().count() as u32).sum();
    TIMEOUT_BASE + TIMEOUT_PER_CHAR * chars
}

/// The batch file the helper reads: `{"items": [{text, lang, voice, speed, out}, ...]}`, one
/// item per line. `out` is the helper's raw 24 kHz file.
pub fn batch_json(items: &[KokoroItem]) -> String {
    let mut s = String::from("{\"items\": [\n");
    for (i, item) in items.iter().enumerate() {
        let _ = writeln!(
            s,
            "  {{\"text\": {}, \"lang\": \"{}\", \"voice\": {}, \"speed\": {KOKORO_SPEED}, \"out\": {}}}{}",
            json_string(&item.text),
            item.lang.code(),
            json_string(&item.voice),
            json_string(&item.raw_path().to_string_lossy()),
            if i + 1 < items.len() { "," } else { "" }
        );
    }
    s.push_str("]}\n");
    s
}

/// A JSON string literal (RFC 8259 escaping).
fn json_string(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => {
                let _ = write!(out, "\\u{:04x}", c as u32);
            }
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

fn default_script_path() -> PathBuf {
    std::env::temp_dir().join(format!(
        "vimmer-kokoro-tts-{:016x}.py",
        fnv1a(KOKORO_SCRIPT)
    ))
}

fn env_nonempty(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|s| !s.trim().is_empty())
}

/// The last `n` non-empty lines of a child's stderr.
fn tail(text: &str, n: usize) -> String {
    let lines: Vec<&str> = text.lines().filter(|l| !l.trim().is_empty()).collect();
    lines[lines.len().saturating_sub(n)..].join(" | ")
}

/// 64-bit FNV-1a: a stable, dependency-free cache key.
fn fnv1a(s: &str) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in s.bytes() {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x0100_0000_01b3);
    }
    hash
}

#[cfg(test)]
mod tests {
    use std::os::unix::fs::PermissionsExt;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;
    use std::time::Instant;

    use super::*;

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("vimmer-kokoro-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn executable(path: &Path, body: &str) {
        fs::write(path, format!("#!/bin/sh\n{body}\n")).unwrap();
        fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
    }

    /// A stand-in interpreter: logs its arguments, prints a warning and writes each `out` named
    /// in the batch file (the last argument), which holds one item per line.
    const FAKE_PYTHON: &str = r#"
dir=$(dirname "$0")
printf '%s\n' "$@" > "$dir/args.txt"
for batch; do :; done
cp "$batch" "$dir/batch-seen.json"
echo "warning: fake voice note"
sed -n 's/.*"out": "\([^"]*\)".*/\1/p' "$batch" | while read -r out; do printf 'RIFF' > "$out"; done
"#;

    /// A stand-in ffmpeg: copies the `-i` input to the last argument.
    const FAKE_FFMPEG: &str = r#"
while [ "$1" != "-i" ]; do shift; done
in=$2
for out; do :; done
cp "$in" "$out"
"#;

    fn fake_engine(dir: &Path, python_body: &str) -> KokoroSpeech {
        let models = dir.join("models");
        fs::create_dir_all(&models).unwrap();
        for lang in [KokoroLang::Zh, KokoroLang::Ja] {
            for f in lang.model_files() {
                fs::write(models.join(f), b"model").unwrap();
            }
        }
        executable(&dir.join("python"), python_body);
        executable(&dir.join("ffmpeg"), FAKE_FFMPEG);
        let mut engine =
            KokoroSpeech::locate(Some(&dir.join("python")), Some(&models), None).unwrap();
        engine.ffmpeg = dir.join("ffmpeg");
        engine.script = dir.join("kokoro_tts.py");
        engine
    }

    fn item(dir: &Path, text: &str, name: &str) -> KokoroItem {
        let lang = KokoroLang::from_text(text);
        KokoroItem {
            text: text.into(),
            lang,
            voice: lang.default_voice().into(),
            out: dir.join("tts").join(name),
        }
    }

    #[test]
    fn engine_names_parse() {
        assert_eq!(TtsEngine::parse("say"), Some(TtsEngine::Say));
        assert_eq!(TtsEngine::parse(" Kokoro "), Some(TtsEngine::Kokoro));
        assert_eq!(TtsEngine::parse("espeak"), None);
        assert_eq!(TtsEngine::Kokoro.as_str(), "kokoro");
    }

    #[test]
    fn language_and_default_voice_follow_the_text() {
        assert_eq!(KokoroLang::from_text("西安钟楼"), KokoroLang::Zh);
        assert_eq!(KokoroLang::from_text("桜が咲きました"), KokoroLang::Ja);
        assert_eq!(KokoroLang::from_text("Hello"), KokoroLang::En);
        assert_eq!(KokoroLang::from_text("123"), KokoroLang::En);
        assert_eq!(KokoroLang::Zh.default_voice(), "zf_001");
        assert_eq!(KokoroLang::Ja.default_voice(), "jf_alpha");
        assert_eq!(KokoroLang::En.default_voice(), "af_heart");
    }

    #[test]
    fn voices_kokoro_names_pass_say_names_fall_back_with_a_warning() {
        assert!(looks_like_kokoro_voice("zf_001"));
        assert!(looks_like_kokoro_voice("jm_kumo"));
        assert!(!looks_like_kokoro_voice("Tingting"));
        assert!(!looks_like_kokoro_voice("zf_"));
        assert!(!looks_like_kokoro_voice("Zf_001"));

        assert_eq!(
            resolve_kokoro_voice(None, KokoroLang::Zh),
            ("zf_001".into(), None)
        );
        assert_eq!(
            resolve_kokoro_voice(Some(" "), KokoroLang::Ja),
            ("jf_alpha".into(), None)
        );
        assert_eq!(
            resolve_kokoro_voice(Some("zm_010"), KokoroLang::Zh),
            ("zm_010".into(), None)
        );
        assert_eq!(
            resolve_kokoro_voice(Some("bf_emma"), KokoroLang::En),
            ("bf_emma".into(), None)
        );
        let (voice, warning) = resolve_kokoro_voice(Some("Tingting"), KokoroLang::Zh);
        assert_eq!(voice, "zf_001");
        assert!(warning.unwrap().contains("`Tingting` is a `say` voice"));
        let (voice, warning) = resolve_kokoro_voice(Some("jf_alpha"), KokoroLang::Zh);
        assert_eq!(voice, "zf_001");
        assert!(warning.unwrap().contains("not a zh Kokoro voice"));
    }

    #[test]
    fn batch_json_escapes_and_lists_raw_outputs() {
        let items = vec![
            KokoroItem {
                text: "say \"hi\"\\\n\t\u{1}中文".into(),
                lang: KokoroLang::Zh,
                voice: "zf_001".into(),
                out: PathBuf::from("/x/a.wav"),
            },
            KokoroItem {
                text: "b".into(),
                lang: KokoroLang::En,
                voice: "af_heart".into(),
                out: PathBuf::from("/x/b.wav"),
            },
        ];
        let json = batch_json(&items);
        assert_eq!(
            json,
            "{\"items\": [\n  {\"text\": \"say \\\"hi\\\"\\\\\\n\\t\\u0001中文\", \"lang\": \"zh\", \"voice\": \"zf_001\", \"speed\": 1.0, \"out\": \"/x/a.kokoro24k.wav\"},\n  {\"text\": \"b\", \"lang\": \"en\", \"voice\": \"af_heart\", \"speed\": 1.0, \"out\": \"/x/b.kokoro24k.wav\"}\n]}\n"
        );
    }

    #[test]
    fn cache_key_covers_engine_inputs() {
        let dir = temp_dir("key");
        let engine = fake_engine(&dir, "exit 0");
        let base = engine.cache_key("你好", KokoroLang::Zh, "zf_001");
        assert_eq!(base, engine.cache_key("你好", KokoroLang::Zh, "zf_001"));
        assert_ne!(base, engine.cache_key("你好!", KokoroLang::Zh, "zf_001"));
        assert_ne!(base, engine.cache_key("你好", KokoroLang::Zh, "zf_002"));
        assert_ne!(base, engine.cache_key("你好", KokoroLang::Ja, "zf_001"));
        // A replaced model file (another size) invalidates the cache.
        fs::write(engine.models.join("kokoro-v1.1-zh.onnx"), b"model v2").unwrap();
        assert_ne!(base, engine.cache_key("你好", KokoroLang::Zh, "zf_001"));
        // Not the say key (`voice\ntext`).
        assert_ne!(base, fnv1a("zf_001\n你好"));
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn locate_and_check_models_explain_what_is_missing() {
        let dir = temp_dir("locate");
        let err = KokoroSpeech::locate(Some(Path::new("/nonexistent/python")), None, None)
            .unwrap_err()
            .to_string();
        assert!(
            err.contains("/nonexistent/python") && err.contains("--kokoro-python"),
            "{err}"
        );

        let engine = fake_engine(&dir, "exit 0");
        let err = KokoroSpeech::locate(Some(&engine.python), Some(&dir.join("no-models")), None)
            .unwrap_err()
            .to_string();
        assert!(err.contains("no-models"), "{err}");

        assert!(engine
            .check_models(&[KokoroLang::Zh, KokoroLang::Ja])
            .is_ok());
        fs::remove_file(engine.models.join("config-v1.1-zh.json")).unwrap();
        let err = engine
            .check_models(&[KokoroLang::Zh])
            .unwrap_err()
            .to_string();
        assert!(err.contains("config-v1.1-zh.json"), "{err}");
        assert!(engine.check_models(&[KokoroLang::En]).is_ok());
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn command_runs_the_helper_isolated_with_the_models() {
        let dir = temp_dir("cmd");
        let mut engine = fake_engine(&dir, "exit 0");
        engine.path_var = Some("/opt/bin".into());
        let cmd = engine.command(Path::new("/w/batch.json"));
        assert_eq!(cmd.get_program(), engine.python.as_os_str());
        let args: Vec<String> = cmd
            .get_args()
            .map(|a| a.to_string_lossy().into_owned())
            .collect();
        assert_eq!(
            args,
            [
                "-I".to_string(),
                engine.script.display().to_string(),
                "--models".into(),
                engine.models.display().to_string(),
                "/w/batch.json".into(),
            ]
        );
        assert!(cmd
            .get_envs()
            .any(|(k, v)| k == "PATH" && v == Some(std::ffi::OsStr::new("/opt/bin"))));
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn one_run_writes_every_clip_and_returns_warnings() {
        let dir = temp_dir("batch");
        let engine = fake_engine(&dir, FAKE_PYTHON);
        let items = vec![
            item(&dir, "西安钟楼", "a.wav"),
            item(&dir, "桜です", "b.wav"),
        ];
        let warnings = engine
            .synthesize_batch(&items, &dir.join("work"), &Limits::none())
            .unwrap();
        assert_eq!(warnings, ["fake voice note"]);
        for it in &items {
            assert_eq!(fs::read(&it.out).unwrap(), b"RIFF");
            assert!(!it.raw_path().exists(), "raw 24 kHz file is removed");
        }
        // The helper ran once, from the embedded script, with both items.
        assert_eq!(fs::read_to_string(&engine.script).unwrap(), KOKORO_SCRIPT);
        let seen = fs::read_to_string(dir.join("batch-seen.json")).unwrap();
        assert!(seen.contains("西安钟楼") && seen.contains("桜です"));
        assert!(!dir.join("work/kokoro-batch.json").exists());
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn a_failing_helper_is_an_error_with_its_stderr() {
        let dir = temp_dir("fail");
        let engine = fake_engine(
            &dir,
            "echo noise >&2\necho 'ModuleNotFoundError: kokoro_onnx' >&2\nexit 1",
        );
        let err = engine
            .synthesize_batch(&[item(&dir, "你好", "a.wav")], &dir, &Limits::none())
            .unwrap_err();
        assert!(matches!(err, MediaError::Speech(_)));
        assert!(err.to_string().contains("ModuleNotFoundError"), "{err}");

        // Exit 0 without writing the clip is an error too.
        let engine = fake_engine(&dir, "exit 0");
        let err = engine
            .synthesize_batch(&[item(&dir, "你好", "a.wav")], &dir, &Limits::none())
            .unwrap_err();
        assert!(err.to_string().contains("wrote no audio"), "{err}");
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn a_hung_helper_times_out_and_a_cancel_stops_it() {
        let dir = temp_dir("hang");
        let engine = fake_engine(&dir, "sleep 30");
        let started = Instant::now();
        let limits = Limits::none().with_timeout(Duration::from_millis(300));
        let err = engine
            .synthesize_batch(&[item(&dir, "你好", "a.wav")], &dir, &limits)
            .unwrap_err();
        assert!(matches!(err, MediaError::TimedOut { .. }), "{err}");

        let flag = Arc::new(AtomicBool::new(false));
        let setter = flag.clone();
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(200));
            setter.store(true, Ordering::SeqCst);
        });
        let limits = Limits {
            timeout: None,
            cancelled: Arc::new(move || flag.load(Ordering::SeqCst)),
        };
        let err = engine
            .synthesize_batch(&[item(&dir, "你好", "a.wav")], &dir, &limits)
            .unwrap_err();
        assert!(matches!(err, MediaError::Cancelled), "{err}");
        assert!(started.elapsed() < Duration::from_secs(10));
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn timeout_grows_with_text() {
        assert_eq!(
            batch_timeout(["你好", "abc"]),
            TIMEOUT_BASE + Duration::from_secs(5)
        );
    }

    /// Real Kokoro, three languages. Needs the venv and models from `install_kokoro.sh`:
    /// `cargo test -p vimmer-media --lib -- --ignored kokoro_real`.
    #[test]
    #[ignore]
    fn kokoro_real_synthesizes_three_languages() {
        let dir = temp_dir("real");
        let engine = KokoroSpeech::locate(None, None, None).expect("Kokoro installed");
        engine
            .check_models(&[KokoroLang::Zh, KokoroLang::Ja, KokoroLang::En])
            .unwrap();
        let items = vec![
            item(&dir, "我们用 Python 制作视频。", "zh.wav"),
            item(&dir, "今日はいい天気ですね。", "ja.wav"),
            item(&dir, "Hello from Kokoro.", "en.wav"),
        ];
        let started = Instant::now();
        let warnings = engine
            .synthesize_batch(
                &items,
                &dir,
                &Limits::none().with_timeout(batch_timeout(items.iter().map(|i| i.text.as_str()))),
            )
            .unwrap();
        eprintln!("batch took {:?}, warnings {warnings:?}", started.elapsed());
        let probe = crate::audio::FfmpegProbe::default();
        for it in &items {
            let ms = crate::audio::AudioProbe::duration_ms(&probe, &it.out).unwrap();
            eprintln!("{}: {ms} ms", it.out.display());
            assert!(ms > 500, "{} is {ms} ms", it.out.display());
            let banner = Command::new("ffmpeg")
                .arg("-hide_banner")
                .arg("-i")
                .arg(&it.out)
                .output()
                .unwrap();
            let banner = String::from_utf8_lossy(&banner.stderr);
            assert!(
                banner.contains("48000 Hz") && banner.contains("stereo"),
                "{banner}"
            );
        }
        let _ = fs::remove_dir_all(dir);
    }
}
