//! Local speech recognition with whisper.cpp's `whisper-cli`, run as a child process like `say`.
//!
//! `whisper-cli -ojf` writes a JSON file with every decoded token and its timing (`offsets`).
//! Optionally (`$VIMMER_WHISPER_DTW`), `--dtw` adds a cross-attention-aligned time per token;
//! see [`dtw_from_env`] for why it is off by default. On Intel Macs whisper runs CPU-only
//! (`-ng`), because the Metal backend returned garbage there.
//! The JSON is read with a small byte-level parser: whisper writes token text as raw bytes, and
//! a CJK character is often split across two tokens, so strings must survive invalid UTF-8.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::align::RecognizedToken;
use crate::process::{run_limited, Limits};
use crate::MediaError;

/// Environment variable naming the `whisper-cli` binary (a path, or a name looked up on PATH).
pub const ENV_WHISPER_CLI: &str = "VIMMER_WHISPER_CLI";
/// Environment variable naming the whisper model file.
pub const ENV_WHISPER_MODEL: &str = "VIMMER_WHISPER_MODEL";
/// Environment variable forcing GPU use on (`1`) or off (`0`) for whisper.
pub const ENV_WHISPER_GPU: &str = "VIMMER_WHISPER_GPU";
/// Environment variable enabling DTW token timing: `1`/`auto` picks the preset from the model
/// name, any other non-empty value is used as the preset, `0` (or unset) disables it.
pub const ENV_WHISPER_DTW: &str = "VIMMER_WHISPER_DTW";
/// Model used when nothing else is configured, relative to the home directory.
pub const DEFAULT_MODEL_IN_HOME: &str = ".local/share/vimmer/models/ggml-large-v3-turbo-q5_0.bin";

/// What to recognize. Knowing the language and the expected words makes recognition far more
/// reliable, and both are known for narration.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TranscribeRequest {
    /// Whisper language code (`zh`, `en`, `ja`, ...). `None` lets whisper detect it.
    pub language: Option<String>,
    /// Text the speech is expected to contain, passed to whisper as its initial prompt.
    pub prompt: Option<String>,
}

/// Turns a speech clip into timed tokens, with times relative to the start of the clip.
/// Every external command runs under `limits`: it is killed when the render is cancelled
/// ([`MediaError::Cancelled`]) or runs past its time budget ([`MediaError::TimedOut`]).
pub trait Transcriber: Send + Sync {
    fn transcribe(
        &self,
        wav: &Path,
        request: &TranscribeRequest,
        limits: &Limits,
    ) -> Result<Vec<RecognizedToken>, MediaError>;

    /// Joins clips into one 16 kHz mono WAV, each followed by `gap_ms` of silence, so several
    /// clips can be recognized in one call. Callers compute clip offsets from their durations.
    fn join_clips(
        &self,
        inputs: &[PathBuf],
        gap_ms: u64,
        out: &Path,
        limits: &Limits,
    ) -> Result<(), MediaError> {
        join_clips_with_ffmpeg(Path::new("ffmpeg"), inputs, gap_ms, out, limits)
    }
}

/// [`Transcriber::join_clips`] with a given ffmpeg binary.
pub fn join_clips_with_ffmpeg(
    ffmpeg: &Path,
    inputs: &[PathBuf],
    gap_ms: u64,
    out: &Path,
    limits: &Limits,
) -> Result<(), MediaError> {
    let mut cmd = Command::new(ffmpeg);
    cmd.args(["-y", "-hide_banner", "-loglevel", "error"]);
    for input in inputs {
        cmd.arg("-i").arg(input);
    }
    let mut graph = String::new();
    for i in 0..inputs.len() {
        graph.push_str(&format!(
            "[{i}:a]aresample=16000,aformat=sample_fmts=s16:channel_layouts=mono,apad=pad_dur={:.3}[a{i}];",
            gap_ms as f64 / 1000.0
        ));
    }
    for i in 0..inputs.len() {
        graph.push_str(&format!("[a{i}]"));
    }
    graph.push_str(&format!("concat=n={}:v=0:a=1[out]", inputs.len()));
    let tmp = out.with_extension("partial.wav");
    cmd.arg("-filter_complex")
        .arg(graph)
        .args(["-map", "[out]", "-c:a", "pcm_s16le"])
        .arg(&tmp);
    let output = run_limited(cmd, limits);
    if output.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    let output = output?;
    if !output.status.success() {
        return Err(MediaError::Transcribe(format!(
            "ffmpeg could not join clips: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }
    fs::rename(&tmp, out)?;
    Ok(())
}

/// Language of a script, judged by its characters: any kana means Japanese, other CJK
/// ideographs mean Chinese, Hangul means Korean, Latin letters mean English.
pub fn guess_language(text: &str) -> Option<&'static str> {
    let has = |f: fn(char) -> bool| text.chars().any(f);
    if has(|c| ('\u{3040}'..='\u{30FF}').contains(&c) || ('\u{FF66}'..='\u{FF9F}').contains(&c)) {
        Some("ja")
    } else if has(|c| {
        ('\u{4E00}'..='\u{9FFF}').contains(&c) || ('\u{3400}'..='\u{4DBF}').contains(&c)
    }) {
        Some("zh")
    } else if has(|c| ('\u{AC00}'..='\u{D7AF}').contains(&c)) {
        Some("ko")
    } else if has(|c| c.is_ascii_alphabetic()) {
        Some("en")
    } else {
        None
    }
}

/// Finds an executable: a path containing `/` is used as is, a bare name is searched on `path_var`.
pub fn find_executable(name: &str, path_var: Option<&str>) -> Option<PathBuf> {
    if name.contains('/') {
        let p = PathBuf::from(name);
        return p.is_file().then_some(p);
    }
    let path_var = path_var
        .map(str::to_string)
        .or_else(|| std::env::var("PATH").ok())?;
    std::env::split_paths(&path_var)
        .map(|dir| dir.join(name))
        .find(|candidate| candidate.is_file())
}

/// The `--dtw` preset matching a model file name, e.g. `ggml-large-v3-turbo-q5_0.bin` →
/// `large.v3.turbo`. `None` for models whisper.cpp has no alignment-head preset for.
pub fn dtw_preset_for_model(model: &Path) -> Option<&'static str> {
    let name = model.file_name()?.to_str()?.to_ascii_lowercase();
    const PRESETS: [(&str, &str); 13] = [
        ("large-v3-turbo", "large.v3.turbo"),
        ("large-v3", "large.v3"),
        ("large-v2", "large.v2"),
        ("large-v1", "large.v1"),
        ("medium.en", "medium.en"),
        ("medium", "medium"),
        ("small.en", "small.en"),
        ("small", "small"),
        ("base.en", "base.en"),
        ("base", "base"),
        ("tiny.en", "tiny.en"),
        ("tiny", "tiny"),
        ("large", "large.v3"),
    ];
    PRESETS
        .iter()
        .find(|(needle, _)| name.contains(needle))
        .map(|(_, preset)| *preset)
}

/// The DTW preset requested by `$VIMMER_WHISPER_DTW` (see [`ENV_WHISPER_DTW`]).
///
/// Off by default: measured on an Intel Mac (large-v3-turbo, CPU), DTW needs flash attention
/// off and nearly doubled recognition time (71 s vs 39 s for a 4.6 s clip), while on clean
/// TTS speech the plain token timestamps already followed pauses as closely.
pub fn dtw_from_env(value: Option<&str>, model: &Path) -> Option<String> {
    match value.map(str::trim) {
        None | Some("" | "0" | "off" | "false") => None,
        Some("1" | "on" | "true" | "auto") => dtw_preset_for_model(model).map(String::from),
        Some(preset) => Some(preset.to_string()),
    }
}

/// whisper.cpp command-line engine.
#[derive(Debug, Clone)]
pub struct WhisperCli {
    pub binary: PathBuf,
    pub model: PathBuf,
    pub ffmpeg: PathBuf,
    pub threads: usize,
    /// `--dtw` preset. `None` (the default) uses whisper's plain token timestamps.
    pub dtw: Option<String>,
    /// Let whisper use the GPU. Off by default on Intel Macs, where the Metal backend was seen
    /// to return garbage; `$VIMMER_WHISPER_GPU` (`1`/`0`) overrides.
    pub gpu: bool,
}

/// Default for [`WhisperCli::gpu`]: `$VIMMER_WHISPER_GPU` when set, else on except on x86_64.
pub fn default_gpu() -> bool {
    match std::env::var(ENV_WHISPER_GPU).ok().as_deref() {
        Some("0" | "false" | "off") => false,
        Some("1" | "true" | "on") => true,
        _ => !cfg!(target_arch = "x86_64"),
    }
}

impl WhisperCli {
    /// Locates the engine. The binary comes from `binary`, else `$VIMMER_WHISPER_CLI`, else
    /// `whisper-cli` on `path_var` (or the process PATH). The model comes from `model`, else
    /// `$VIMMER_WHISPER_MODEL`, else [`DEFAULT_MODEL_IN_HOME`]. Errors explain what is missing.
    pub fn locate(
        binary: Option<&Path>,
        model: Option<&Path>,
        path_var: Option<&str>,
    ) -> Result<Self, MediaError> {
        let binary_name = binary
            .map(|p| p.to_string_lossy().into_owned())
            .or_else(|| {
                std::env::var(ENV_WHISPER_CLI)
                    .ok()
                    .filter(|s| !s.is_empty())
            })
            .unwrap_or_else(|| "whisper-cli".to_string());
        let binary = find_executable(&binary_name, path_var).ok_or_else(|| {
            MediaError::Transcribe(format!(
                "`{binary_name}` was not found (install whisper.cpp so `whisper-cli` is on PATH, or pass --whisper-cli / set {ENV_WHISPER_CLI})"
            ))
        })?;
        let model = model
            .map(Path::to_path_buf)
            .or_else(|| {
                std::env::var_os(ENV_WHISPER_MODEL)
                    .filter(|s| !s.is_empty())
                    .map(PathBuf::from)
            })
            .or_else(|| {
                std::env::var_os("HOME").map(|h| PathBuf::from(h).join(DEFAULT_MODEL_IN_HOME))
            })
            .ok_or_else(|| MediaError::Transcribe("no whisper model configured".into()))?;
        if !model.is_file() {
            return Err(MediaError::Transcribe(format!(
                "whisper model `{}` does not exist (set {ENV_WHISPER_MODEL} or pass a model path)",
                model.display()
            )));
        }
        let threads = std::thread::available_parallelism()
            .map(|n| n.get().min(8))
            .unwrap_or(4);
        let dtw = dtw_from_env(std::env::var(ENV_WHISPER_DTW).ok().as_deref(), &model);
        Ok(Self {
            binary,
            model,
            ffmpeg: PathBuf::from("ffmpeg"),
            threads,
            dtw,
            gpu: default_gpu(),
        })
    }

    fn cache_key(&self, request: &TranscribeRequest) -> u64 {
        let model_name = self
            .model
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        fnv1a(&format!(
            "v1\n{model_name}\n{:?}\n{:?}\n{:?}\ngpu={}",
            request.language, request.prompt, self.dtw, self.gpu
        ))
    }

    fn run_whisper(
        &self,
        input16k: &Path,
        out_prefix: &Path,
        request: &TranscribeRequest,
        dtw: Option<&str>,
        limits: &Limits,
    ) -> Result<(), MediaError> {
        let mut cmd = Command::new(&self.binary);
        cmd.arg("-m")
            .arg(&self.model)
            .arg("-f")
            .arg(input16k)
            .arg("-l")
            .arg(request.language.as_deref().unwrap_or("auto"))
            .arg("-t")
            .arg(self.threads.to_string())
            .args(["-ojf", "-np", "-of"])
            .arg(out_prefix);
        if let Some(prompt) = request.prompt.as_deref().filter(|p| !p.trim().is_empty()) {
            cmd.arg("--prompt").arg(prompt);
        }
        if !self.gpu {
            cmd.arg("-ng");
        }
        if let Some(preset) = dtw {
            // whisper.cpp silently skips DTW while flash attention is on.
            cmd.arg("-nfa").arg("-dtw").arg(preset);
        }
        let out = run_limited(cmd, limits)?;
        if out.status.success() {
            Ok(())
        } else {
            let stderr = String::from_utf8_lossy(&out.stderr);
            let tail: Vec<&str> = stderr.lines().rev().take(6).collect();
            Err(MediaError::Transcribe(format!(
                "whisper-cli exited with {}: {}",
                out.status,
                tail.into_iter().rev().collect::<Vec<_>>().join(" | ")
            )))
        }
    }
}

impl Transcriber for WhisperCli {
    fn join_clips(
        &self,
        inputs: &[PathBuf],
        gap_ms: u64,
        out: &Path,
        limits: &Limits,
    ) -> Result<(), MediaError> {
        join_clips_with_ffmpeg(&self.ffmpeg, inputs, gap_ms, out, limits)
    }

    /// Results are cached next to `wav` (keyed by model, language, prompt, DTW preset and GPU
    /// use), so an unchanged narration is recognized once. An empty result is not cached, so a
    /// failed recognition is tried again next time.
    fn transcribe(
        &self,
        wav: &Path,
        request: &TranscribeRequest,
        limits: &Limits,
    ) -> Result<Vec<RecognizedToken>, MediaError> {
        let stem = wav
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| "audio".into());
        let key = self.cache_key(request);
        let cached = wav.with_file_name(format!("{stem}.whisper-{key:016x}.json"));
        if let Ok(bytes) = fs::read(&cached) {
            if let Ok(tokens) = parse_whisper_json(&bytes) {
                if !tokens.is_empty() {
                    return Ok(tokens);
                }
            }
        }

        // whisper.cpp wants 16 kHz mono PCM.
        let input16k = wav.with_file_name(format!("{stem}.16k.wav"));
        let mut resample = Command::new(&self.ffmpeg);
        resample
            .args(["-y", "-hide_banner", "-loglevel", "error", "-i"])
            .arg(wav)
            .args(["-ac", "1", "-ar", "16000", "-c:a", "pcm_s16le"])
            .arg(&input16k);
        let converted = run_limited(resample, limits);
        if converted.is_err() {
            let _ = fs::remove_file(&input16k);
        }
        let converted = converted?;
        if !converted.status.success() {
            return Err(MediaError::Transcribe(format!(
                "ffmpeg could not resample {}: {}",
                wav.display(),
                String::from_utf8_lossy(&converted.stderr).trim()
            )));
        }

        let tmp_prefix = wav.with_file_name(format!("{stem}.whisper-{key:016x}.partial"));
        let tmp_json = tmp_prefix.with_extension("partial.json");
        let mut result =
            self.run_whisper(&input16k, &tmp_prefix, request, self.dtw.as_deref(), limits);
        if matches!(result, Err(MediaError::Transcribe(_))) && self.dtw.is_some() {
            // Older builds, or a model the preset does not fit: fall back to plain timestamps.
            // A cancel or a timeout is final, so it is not retried.
            result = self.run_whisper(&input16k, &tmp_prefix, request, None, limits);
        }
        let _ = fs::remove_file(&input16k);
        if let Err(e) = result {
            let _ = fs::remove_file(&tmp_json);
            return Err(e);
        }
        let bytes = fs::read(&tmp_json)?;
        let tokens = parse_whisper_json(&bytes);
        match &tokens {
            Ok(t) if !t.is_empty() => fs::rename(&tmp_json, &cached)?,
            _ => {
                let _ = fs::remove_file(&tmp_json);
            }
        }
        tokens
    }
}

/// Reads the tokens of a `whisper-cli -ojf` file. Special tokens (`[_BEG_]`, `[_TT_…]`) are
/// dropped, and tokens holding parts of one UTF-8 character are merged. Times come from DTW
/// (`t_dtw`, in 10 ms units: a token runs from its own DTW time to the next token's) when every
/// token has one, else from each token's `offsets`.
pub fn parse_whisper_json(bytes: &[u8]) -> Result<Vec<RecognizedToken>, MediaError> {
    let bad = |what: &str| MediaError::Transcribe(format!("unexpected whisper JSON: {what}"));
    let root = json::parse(bytes).map_err(|e| bad(&e))?;
    let segments = root
        .get("transcription")
        .and_then(json::Value::as_array)
        .ok_or_else(|| bad("no `transcription` array"))?;

    struct Raw {
        bytes: Vec<u8>,
        from: u64,
        to: u64,
        dtw: Option<u64>,
        segment_end: u64,
    }
    let mut raws = Vec::new();
    for segment in segments {
        let segment_end = segment
            .get("offsets")
            .and_then(|o| o.get("to"))
            .and_then(json::Value::as_u64)
            .unwrap_or(0);
        let Some(tokens) = segment.get("tokens").and_then(json::Value::as_array) else {
            continue;
        };
        for token in tokens {
            let Some(text) = token.get("text").and_then(json::Value::as_bytes) else {
                continue;
            };
            let special = (text.starts_with(b"[_") && text.ends_with(b"]"))
                || (text.starts_with(b"<|") && text.ends_with(b"|>"));
            if special {
                continue;
            }
            let offsets = token.get("offsets");
            let from = offsets
                .and_then(|o| o.get("from"))
                .and_then(json::Value::as_u64)
                .unwrap_or(0);
            let to = offsets
                .and_then(|o| o.get("to"))
                .and_then(json::Value::as_u64)
                .unwrap_or(from);
            let dtw = token
                .get("t_dtw")
                .and_then(json::Value::as_i64)
                .filter(|&t| t >= 0)
                .map(|t| t as u64 * 10);
            raws.push(Raw {
                bytes: text.to_vec(),
                from,
                to,
                dtw,
                segment_end,
            });
        }
    }

    let use_dtw = !raws.is_empty() && raws.iter().all(|r| r.dtw.is_some());
    let timed: Vec<(Vec<u8>, u64, u64)> = raws
        .iter()
        .enumerate()
        .map(|(i, r)| {
            if use_dtw {
                let start = r.dtw.unwrap_or(r.from);
                let next = raws.get(i + 1).and_then(|n| n.dtw);
                let end = match next {
                    Some(n) if n >= start && n <= r.segment_end.max(start) => n,
                    _ => r.segment_end.max(r.to).max(start),
                };
                (r.bytes.clone(), start, end)
            } else {
                (r.bytes.clone(), r.from, r.to.max(r.from))
            }
        })
        .collect();

    // Merge tokens until their bytes form whole UTF-8 characters.
    let mut out = Vec::new();
    let mut buf: Vec<u8> = Vec::new();
    let mut start = 0;
    for (bytes, s, e) in timed {
        if buf.is_empty() {
            start = s;
        }
        buf.extend_from_slice(&bytes);
        match std::str::from_utf8(&buf) {
            Ok(text) => {
                out.push(RecognizedToken::new(text, start, e));
                buf.clear();
            }
            Err(err) if err.error_len().is_none() => {} // incomplete: wait for the next token
            Err(_) => {
                out.push(RecognizedToken::new(
                    String::from_utf8_lossy(&buf),
                    start,
                    e,
                ));
                buf.clear();
            }
        }
    }
    if !buf.is_empty() {
        let end = out
            .last()
            .map_or(start, |t: &RecognizedToken| t.end_ms.max(start));
        out.push(RecognizedToken::new(
            String::from_utf8_lossy(&buf),
            start,
            end,
        ));
    }
    Ok(out)
}

fn fnv1a(s: &str) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in s.bytes() {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x0100_0000_01b3);
    }
    hash
}

/// Minimal JSON reader. Strings are kept as raw bytes (see the module docs).
mod json {
    #[derive(Debug, Clone, PartialEq)]
    pub enum Value {
        Null,
        Bool(bool),
        Number(f64),
        String(Vec<u8>),
        Array(Vec<Value>),
        Object(Vec<(Vec<u8>, Value)>),
    }

    impl Value {
        pub fn get(&self, key: &str) -> Option<&Value> {
            match self {
                Value::Object(fields) => fields
                    .iter()
                    .find(|(k, _)| k.as_slice() == key.as_bytes())
                    .map(|(_, v)| v),
                _ => None,
            }
        }
        pub fn as_array(&self) -> Option<&[Value]> {
            match self {
                Value::Array(items) => Some(items),
                _ => None,
            }
        }
        pub fn as_bytes(&self) -> Option<&[u8]> {
            match self {
                Value::String(bytes) => Some(bytes),
                _ => None,
            }
        }
        pub fn as_i64(&self) -> Option<i64> {
            match self {
                Value::Number(n) if n.is_finite() => Some(n.round() as i64),
                _ => None,
            }
        }
        pub fn as_u64(&self) -> Option<u64> {
            self.as_i64().filter(|&n| n >= 0).map(|n| n as u64)
        }
    }

    pub fn parse(input: &[u8]) -> Result<Value, String> {
        let mut p = Parser { s: input, i: 0 };
        let value = p.value(0)?;
        p.ws();
        if p.i != p.s.len() {
            return Err(format!("trailing data at byte {}", p.i));
        }
        Ok(value)
    }

    struct Parser<'a> {
        s: &'a [u8],
        i: usize,
    }

    impl Parser<'_> {
        fn ws(&mut self) {
            while self.i < self.s.len() && self.s[self.i].is_ascii_whitespace() {
                self.i += 1;
            }
        }

        fn peek(&self) -> Option<u8> {
            self.s.get(self.i).copied()
        }

        fn expect(&mut self, b: u8) -> Result<(), String> {
            if self.peek() == Some(b) {
                self.i += 1;
                Ok(())
            } else {
                Err(format!("expected `{}` at byte {}", b as char, self.i))
            }
        }

        fn literal(&mut self, word: &str, value: Value) -> Result<Value, String> {
            if self.s[self.i..].starts_with(word.as_bytes()) {
                self.i += word.len();
                Ok(value)
            } else {
                Err(format!("bad literal at byte {}", self.i))
            }
        }

        fn value(&mut self, depth: usize) -> Result<Value, String> {
            if depth > 64 {
                return Err("nesting too deep".into());
            }
            self.ws();
            match self.peek() {
                Some(b'{') => {
                    self.i += 1;
                    let mut fields = Vec::new();
                    self.ws();
                    if self.peek() == Some(b'}') {
                        self.i += 1;
                        return Ok(Value::Object(fields));
                    }
                    loop {
                        self.ws();
                        let key = self.string()?;
                        self.ws();
                        self.expect(b':')?;
                        let v = self.value(depth + 1)?;
                        fields.push((key, v));
                        self.ws();
                        match self.peek() {
                            Some(b',') => self.i += 1,
                            Some(b'}') => {
                                self.i += 1;
                                return Ok(Value::Object(fields));
                            }
                            _ => return Err(format!("expected `,` or `}}` at byte {}", self.i)),
                        }
                    }
                }
                Some(b'[') => {
                    self.i += 1;
                    let mut items = Vec::new();
                    self.ws();
                    if self.peek() == Some(b']') {
                        self.i += 1;
                        return Ok(Value::Array(items));
                    }
                    loop {
                        items.push(self.value(depth + 1)?);
                        self.ws();
                        match self.peek() {
                            Some(b',') => self.i += 1,
                            Some(b']') => {
                                self.i += 1;
                                return Ok(Value::Array(items));
                            }
                            _ => return Err(format!("expected `,` or `]` at byte {}", self.i)),
                        }
                    }
                }
                Some(b'"') => Ok(Value::String(self.string()?)),
                Some(b't') => self.literal("true", Value::Bool(true)),
                Some(b'f') => self.literal("false", Value::Bool(false)),
                Some(b'n') => self.literal("null", Value::Null),
                Some(b'-' | b'0'..=b'9') => {
                    let start = self.i;
                    while matches!(
                        self.peek(),
                        Some(b'-' | b'+' | b'.' | b'e' | b'E' | b'0'..=b'9')
                    ) {
                        self.i += 1;
                    }
                    std::str::from_utf8(&self.s[start..self.i])
                        .ok()
                        .and_then(|t| t.parse::<f64>().ok())
                        .map(Value::Number)
                        .ok_or_else(|| format!("bad number at byte {start}"))
                }
                _ => Err(format!("unexpected input at byte {}", self.i)),
            }
        }

        fn hex4(&mut self) -> Result<u32, String> {
            let digits = self
                .s
                .get(self.i..self.i + 4)
                .and_then(|h| std::str::from_utf8(h).ok())
                .and_then(|h| u32::from_str_radix(h, 16).ok())
                .ok_or_else(|| format!("bad \\u escape at byte {}", self.i))?;
            self.i += 4;
            Ok(digits)
        }

        fn string(&mut self) -> Result<Vec<u8>, String> {
            self.expect(b'"')?;
            let mut out = Vec::new();
            loop {
                let Some(b) = self.peek() else {
                    return Err("unterminated string".into());
                };
                self.i += 1;
                match b {
                    b'"' => return Ok(out),
                    b'\\' => {
                        let Some(e) = self.peek() else {
                            return Err("unterminated escape".into());
                        };
                        self.i += 1;
                        match e {
                            b'"' | b'\\' | b'/' => out.push(e),
                            b'b' => out.push(0x08),
                            b'f' => out.push(0x0c),
                            b'n' => out.push(b'\n'),
                            b'r' => out.push(b'\r'),
                            b't' => out.push(b'\t'),
                            b'u' => {
                                let mut code = self.hex4()?;
                                if (0xD800..0xDC00).contains(&code)
                                    && self.s[self.i..].starts_with(b"\\u")
                                {
                                    self.i += 2;
                                    let low = self.hex4()?;
                                    code = 0x10000
                                        + ((code - 0xD800) << 10)
                                        + (low.wrapping_sub(0xDC00) & 0x3FF);
                                }
                                let c = char::from_u32(code).unwrap_or('\u{FFFD}');
                                let mut buf = [0u8; 4];
                                out.extend_from_slice(c.encode_utf8(&mut buf).as_bytes());
                            }
                            other => out.push(other),
                        }
                    }
                    other => out.push(other),
                }
            }
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn parses_nested_values_and_escapes() {
            let v = parse(
                concat!(
                    r#" {"a": [1, -2.5e1, true, null], "s": "x\"\\"#,
                    r"\u00e9\ud83d\ude00",
                    r#"", "o": {}} "#
                )
                .as_bytes(),
            )
            .unwrap();
            assert_eq!(
                v.get("a").unwrap().as_array().unwrap()[1],
                Value::Number(-25.0)
            );
            assert_eq!(
                v.get("s").unwrap().as_bytes().unwrap(),
                "x\"\\é😀".as_bytes()
            );
            assert_eq!(v.get("o"), Some(&Value::Object(vec![])));
        }

        #[test]
        fn keeps_raw_invalid_utf8_bytes() {
            let v = parse(b"[\"\xe4\xbd\"]").unwrap();
            assert_eq!(v.as_array().unwrap()[0].as_bytes().unwrap(), b"\xe4\xbd");
        }

        #[test]
        fn rejects_malformed_input() {
            for bad in [
                &b"{"[..],
                b"[1,]",
                b"{\"a\" 1}",
                b"\"open",
                b"[1] x",
                b"tru",
            ] {
                assert!(parse(bad).is_err(), "{:?}", String::from_utf8_lossy(bad));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn token(text: &str, from: u64, to: u64, dtw: i64) -> String {
        format!(
            r#"{{"text": "{text}", "timestamps": {{"from": "x", "to": "y"}}, "offsets": {{"from": {from}, "to": {to}}}, "id": 1, "p": 0.9, "t_dtw": {dtw}}}"#
        )
    }

    fn doc(segments: &[(u64, u64, Vec<String>)]) -> Vec<u8> {
        let segs: Vec<String> = segments
            .iter()
            .map(|(from, to, tokens)| {
                format!(
                    r#"{{"offsets": {{"from": {from}, "to": {to}}}, "text": "", "tokens": [{}]}}"#,
                    tokens.join(",")
                )
            })
            .collect();
        format!(
            r#"{{"systeminfo": "", "model": {{"type": "large"}}, "result": {{"language": "en"}}, "transcription": [{}]}}"#,
            segs.join(",")
        )
        .into_bytes()
    }

    #[test]
    fn reads_offsets_and_skips_special_tokens() {
        let bytes = doc(&[(
            0,
            1500,
            vec![
                token("[_BEG_]", 0, 0, -1),
                token(" Hello", 0, 600, -1),
                token(" world", 600, 1400, -1),
                token("[_TT_75]", 1500, 1500, -1),
            ],
        )]);
        let tokens = parse_whisper_json(&bytes).unwrap();
        assert_eq!(
            tokens,
            vec![
                RecognizedToken::new(" Hello", 0, 600),
                RecognizedToken::new(" world", 600, 1400)
            ]
        );
    }

    #[test]
    fn prefers_dtw_times_when_every_token_has_one() {
        let bytes = doc(&[
            (
                0,
                1000,
                vec![token(" one", 0, 500, 12), token(" two", 500, 1000, 48)],
            ),
            (1000, 2000, vec![token(" three", 1000, 2000, 130)]),
        ]);
        let tokens = parse_whisper_json(&bytes).unwrap();
        assert_eq!(
            tokens,
            vec![
                RecognizedToken::new(" one", 120, 480),
                RecognizedToken::new(" two", 480, 1000),
                RecognizedToken::new(" three", 1300, 2000)
            ]
        );
    }

    #[test]
    fn merges_tokens_that_split_a_cjk_character() {
        // 钟 is E9 92 9F; whisper emits it as two byte-level tokens.
        let mut bytes = doc(&[(
            0,
            900,
            vec![
                token("@@A", 0, 300, -1),
                token("@@B", 300, 500, -1),
                token("楼", 500, 900, -1),
            ],
        )]);
        let replace = |bytes: &mut Vec<u8>, from: &[u8], to: &[u8]| {
            let at = bytes.windows(from.len()).position(|w| w == from).unwrap();
            bytes.splice(at..at + from.len(), to.iter().copied());
        };
        replace(&mut bytes, b"@@A", b"\xe9\x92");
        replace(&mut bytes, b"@@B", b"\x9f");
        let tokens = parse_whisper_json(&bytes).unwrap();
        assert_eq!(
            tokens,
            vec![
                RecognizedToken::new("钟", 0, 500),
                RecognizedToken::new("楼", 500, 900)
            ]
        );
    }

    #[test]
    fn broken_bytes_degrade_to_replacement_characters() {
        let mut bytes = doc(&[(0, 900, vec![token("@@A", 0, 300, -1)])]);
        let at = bytes.windows(3).position(|w| w == b"@@A").unwrap();
        bytes.splice(at..at + 3, b"\xff\xe9".iter().copied());
        let tokens = parse_whisper_json(&bytes).unwrap();
        assert_eq!(tokens.len(), 1);
        assert!(tokens[0].text.contains('\u{FFFD}'));
    }

    #[test]
    fn rejects_files_without_a_transcription() {
        assert!(parse_whisper_json(b"{}").is_err());
        assert!(parse_whisper_json(b"not json").is_err());
        assert_eq!(
            parse_whisper_json(br#"{"transcription": []}"#).unwrap(),
            vec![]
        );
    }

    #[test]
    fn guesses_language_from_script() {
        assert_eq!(guess_language("今日はいい天気です"), Some("ja"));
        assert_eq!(guess_language("西安钟楼"), Some("zh"));
        assert_eq!(guess_language("안녕하세요"), Some("ko"));
        assert_eq!(guess_language("Hello"), Some("en"));
        assert_eq!(guess_language("123 !!"), None);
    }

    #[test]
    fn dtw_is_opt_in() {
        let m = Path::new("ggml-large-v3-turbo-q5_0.bin");
        assert_eq!(dtw_from_env(None, m), None);
        assert_eq!(dtw_from_env(Some("0"), m), None);
        assert_eq!(
            dtw_from_env(Some("1"), m).as_deref(),
            Some("large.v3.turbo")
        );
        assert_eq!(dtw_from_env(Some("base.en"), m).as_deref(), Some("base.en"));
    }

    #[test]
    fn dtw_preset_follows_the_model_name() {
        let p = |s: &str| dtw_preset_for_model(Path::new(s));
        assert_eq!(p("/m/ggml-large-v3-turbo-q5_0.bin"), Some("large.v3.turbo"));
        assert_eq!(p("ggml-large-v3.bin"), Some("large.v3"));
        assert_eq!(p("ggml-base.en.bin"), Some("base.en"));
        assert_eq!(p("ggml-small-q8_0.bin"), Some("small"));
        assert_eq!(p("custom.bin"), None);
    }

    /// A fake engine: `ffmpeg` is `true` and `whisper-cli` is a script that never finishes.
    fn hanging_engine(dir: &Path) -> WhisperCli {
        use std::os::unix::fs::PermissionsExt;
        fs::create_dir_all(dir).unwrap();
        let script = dir.join("whisper-cli");
        fs::write(&script, "#!/bin/sh\nsleep 30\n").unwrap();
        fs::set_permissions(&script, fs::Permissions::from_mode(0o755)).unwrap();
        WhisperCli {
            binary: script,
            model: dir.join("model.bin"),
            ffmpeg: PathBuf::from("true"),
            threads: 1,
            dtw: Some("large.v3.turbo".into()),
            gpu: false,
        }
    }

    #[test]
    fn hung_whisper_times_out_without_retry_or_cache() {
        let dir = std::env::temp_dir().join(format!("vimmer-hang-timeout-{}", std::process::id()));
        let engine = hanging_engine(&dir);
        let wav = dir.join("clip.wav");
        let started = std::time::Instant::now();
        let limits = Limits::none().with_timeout(std::time::Duration::from_millis(400));
        let err = engine
            .transcribe(&wav, &TranscribeRequest::default(), &limits)
            .unwrap_err();
        assert!(matches!(err, MediaError::TimedOut { .. }), "{err}");
        // One attempt only: a timeout must not trigger the no-DTW retry.
        assert!(started.elapsed() < std::time::Duration::from_secs(3));
        let leftovers: Vec<_> = fs::read_dir(&dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|n| n.contains(".whisper-"))
            .collect();
        assert!(leftovers.is_empty(), "{leftovers:?}");
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn hung_whisper_stops_on_cancel() {
        use std::sync::atomic::{AtomicBool, Ordering};
        use std::sync::Arc;
        let dir = std::env::temp_dir().join(format!("vimmer-hang-cancel-{}", std::process::id()));
        let engine = hanging_engine(&dir);
        let flag = Arc::new(AtomicBool::new(false));
        let setter = flag.clone();
        std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(300));
            setter.store(true, Ordering::SeqCst);
        });
        let limits = Limits {
            timeout: None,
            cancelled: Arc::new(move || flag.load(Ordering::SeqCst)),
        };
        let started = std::time::Instant::now();
        let err = engine
            .transcribe(
                &dir.join("clip.wav"),
                &TranscribeRequest::default(),
                &limits,
            )
            .unwrap_err();
        assert!(matches!(err, MediaError::Cancelled), "{err}");
        assert!(started.elapsed() < std::time::Duration::from_secs(5));
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn locate_reports_a_missing_binary_or_model() {
        let err = WhisperCli::locate(None, None, Some("/nonexistent-dir")).unwrap_err();
        assert!(err.to_string().contains("not found"), "{err}");
        let sh = find_executable("sh", Some("/bin:/usr/bin")).expect("sh exists");
        let err =
            WhisperCli::locate(Some(&sh), Some(Path::new("/no/such/model.bin")), None).unwrap_err();
        assert!(err.to_string().contains("does not exist"), "{err}");
    }
}
