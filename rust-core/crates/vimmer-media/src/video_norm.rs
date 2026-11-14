//! Video clips: probing a file and the ffmpeg arguments that normalize a clip once before render.
//!
//! A clip is normalized to exactly the frames the renderer will show: trimmed, at its speed, at
//! the timeline fps (constant), no larger than the timeline frame, H.264 yuv420p with a short GOP
//! and no B-frames so any frame can be decoded quickly and exactly. Its sound, when it is heard,
//! goes to a separate WAV fitted to the time it plays (looped with the picture when the clip
//! loops). The scheduler runs the commands (cancellable) and caches the outputs by
//! [`fingerprint`] of the source bytes plus [`ClipJob::key`].

use std::io::Read;
use std::path::{Path, PathBuf};

use vimmer_dsl::Resolution;

use crate::audio::parse_ffmpeg_duration_ms;
use crate::MediaError;

/// Frames between keyframes of a normalized clip (half a second at 30 fps).
pub const CLIP_GOP: u32 = 15;
/// Sample rate of a clip's sound (the soundtrack's rate).
pub const CLIP_SAMPLE_RATE: u64 = 48_000;

/// What `ffmpeg -i` says about a video file.
#[derive(Debug, Clone, PartialEq)]
pub struct VideoInfo {
    pub duration_ms: u64,
    pub width: u32,
    pub height: u32,
    /// Frame rate from the stream line (`30 fps`, `29.97 fps`), when there is one.
    pub fps: Option<f64>,
    pub has_audio: bool,
}

/// Reads the first video stream of ffmpeg's banner: `Duration:`, `WxH` and `fps` of the first
/// `Video:` stream line, and whether any `Audio:` stream exists. `None` without a duration or a
/// video stream (an audio file, an image, a broken file).
pub fn parse_video_info(banner: &str) -> Option<VideoInfo> {
    let duration_ms = parse_ffmpeg_duration_ms(banner)?;
    let video = banner
        .lines()
        .find(|l| l.contains("Stream #") && l.contains("Video:"))?;
    // Attached pictures (cover art in an audio file) are not a video.
    if video.contains("(attached pic)") {
        return None;
    }
    let after = video.split("Video:").nth(1)?;
    let (width, height) = after
        .split([',', ' '])
        .filter_map(|tok| {
            let (w, h) = tok.split_once('x')?;
            Some((w.parse::<u32>().ok()?, h.parse::<u32>().ok()?))
        })
        .find(|&(w, h)| w > 0 && h > 0)?;
    let fps = after.split(',').find_map(|part| {
        let part = part.trim();
        part.strip_suffix(" fps")?.trim().parse::<f64>().ok()
    });
    let has_audio = banner
        .lines()
        .any(|l| l.contains("Stream #") && l.contains("Audio:"));
    Some(VideoInfo {
        duration_ms,
        width,
        height,
        fps,
        has_audio,
    })
}

/// Probes `path` with `ffmpeg -i`. A missing file, a file ffmpeg cannot open and a file without
/// a video stream are each a [`MediaError::Video`] with the reason.
pub fn probe_video(ffmpeg: &Path, path: &Path) -> Result<VideoInfo, MediaError> {
    let err = |reason: String| MediaError::Video {
        path: path.display().to_string(),
        reason,
    };
    if !path.is_file() {
        return Err(err("file not found".into()));
    }
    let output = std::process::Command::new(ffmpeg)
        .args(["-hide_banner", "-nostdin", "-i"])
        .arg(path)
        .output()?;
    let stderr = String::from_utf8_lossy(&output.stderr);
    parse_video_info(&stderr).ok_or_else(|| {
        let last = stderr
            .lines()
            .rev()
            .find(|l| !l.trim().is_empty() && !l.contains("At least one output file"))
            .unwrap_or("no video stream")
            .trim()
            .to_string();
        err(format!("not a decodable video ({last})"))
    })
}

/// A 128-bit FNV-1a fingerprint of the file's bytes, as 32 hex digits. Not cryptographic: it only
/// tells a changed source from an unchanged one for the cache.
pub fn fingerprint(path: &Path) -> std::io::Result<String> {
    let mut file = std::fs::File::open(path)?;
    let mut hasher = Fnv128::default();
    let mut buf = vec![0u8; 1 << 16];
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.write(&buf[..n]);
    }
    Ok(hasher.hex())
}

/// The fingerprint of a string (for combining a source fingerprint with parameters).
pub fn fingerprint_str(s: &str) -> String {
    let mut hasher = Fnv128::default();
    hasher.write(s.as_bytes());
    hasher.hex()
}

struct Fnv128(u128);

impl Default for Fnv128 {
    fn default() -> Self {
        Self(0x6c62272e07bb014262b821756295c58d)
    }
}

impl Fnv128 {
    const PRIME: u128 = 0x0000000001000000000000000000013B;

    fn write(&mut self, bytes: &[u8]) {
        for &b in bytes {
            self.0 ^= u128::from(b);
            self.0 = self.0.wrapping_mul(Self::PRIME);
        }
    }

    fn hex(&self) -> String {
        format!("{:032x}", self.0)
    }
}

/// One normalization of a clip: which part of the file, how fast, and how many output frames.
#[derive(Debug, Clone, PartialEq)]
pub struct ClipJob {
    pub src: PathBuf,
    pub trim_start_ms: u64,
    /// Length of the trimmed part of the file, in ms.
    pub source_ms: u64,
    /// Effective speed (already fitted for `stretch`).
    pub speed: f64,
    pub fps: u32,
    /// Exactly this many frames are written (the last frame is repeated if the file runs short).
    pub frames: u64,
    /// The picture is scaled down to fit inside this box, never up.
    pub max: Resolution,
}

impl ClipJob {
    /// Everything but the source bytes that changes the output, for the cache key.
    pub fn key(&self) -> String {
        format!(
            "v1|{}|{}|{:.6}|{}|{}|{}x{}",
            self.trim_start_ms,
            self.source_ms,
            self.speed,
            self.fps,
            self.frames,
            self.max.width,
            self.max.height
        )
    }

    /// Input options: accurate seek to the trim start and read only the trimmed part. A file that
    /// runs short is padded by the filter (`tpad`), so frames past the trim end never show.
    fn input_args(&self) -> Vec<String> {
        vec![
            "-ss".into(),
            secs(self.trim_start_ms),
            "-t".into(),
            secs(self.source_ms),
            "-i".into(),
            self.src.display().to_string(),
        ]
    }

    /// The video filter: restart time at 0, apply the speed, resample to a constant `fps`,
    /// scale down to fit `max` (even sizes), and repeat the last frame if the file is short.
    pub fn video_filter(&self) -> String {
        format!(
            "setpts=(PTS-STARTPTS)/{speed:.6},fps={fps},\
             scale=w='min(iw,{w})':h='min(ih,{h})':force_original_aspect_ratio=decrease:force_divisible_by=2,\
             setsar=1,format=yuv420p,tpad=stop_mode=clone:stop={frames}",
            speed = self.speed,
            fps = self.fps,
            w = self.max.width,
            h = self.max.height,
            frames = self.frames,
        )
    }

    /// Full ffmpeg arguments that write the normalized clip to `dst` (an `.mp4`).
    pub fn video_args(&self, dst: &Path) -> Vec<String> {
        let mut args: Vec<String> = ["-y", "-hide_banner", "-nostdin", "-loglevel", "error"]
            .map(String::from)
            .to_vec();
        args.extend(self.input_args());
        args.extend(
            [
                "-map".into(),
                "0:v:0".into(),
                "-vf".into(),
                self.video_filter(),
                "-frames:v".into(),
                self.frames.to_string(),
                "-r".into(),
                self.fps.to_string(),
                "-c:v".into(),
                "libx264".into(),
                "-preset".into(),
                "veryfast".into(),
                "-crf".into(),
                "18".into(),
                "-g".into(),
                CLIP_GOP.to_string(),
                "-keyint_min".into(),
                CLIP_GOP.to_string(),
                "-sc_threshold".into(),
                "0".into(),
                "-bf".into(),
                "0".into(),
                "-pix_fmt".into(),
                "yuv420p".into(),
                "-an".into(),
                "-movflags".into(),
                "+faststart".into(),
            ]
            .into_iter(),
        );
        args.push(dst.display().to_string());
        args
    }

    /// Samples of one pass of the clip's sound: exactly its frames at [`CLIP_SAMPLE_RATE`].
    pub fn clip_samples(&self) -> u64 {
        (self.frames * CLIP_SAMPLE_RATE + u64::from(self.fps) / 2) / u64::from(self.fps)
    }

    /// The audio filter: restart at 0, stereo 48 kHz, the speed as an `atempo` chain (pitch kept),
    /// cut or padded to exactly one pass of the picture, then looped up to `total` samples when
    /// `looped`. The output is `total` samples long when looped, one pass otherwise.
    pub fn audio_filter(&self, looped: bool, total: u64) -> String {
        let pass = self.clip_samples();
        let mut f = format!(
            "asetpts=PTS-STARTPTS,aresample={CLIP_SAMPLE_RATE},aformat=sample_fmts=fltp:channel_layouts=stereo"
        );
        for t in atempo_chain(self.speed) {
            f.push_str(&format!(",atempo={t:.6}"));
        }
        f.push_str(&format!(",atrim=end_sample={pass},apad=whole_len={pass}"));
        if looped && total > pass {
            f.push_str(&format!(
                ",aloop=loop=-1:size={pass},atrim=end_sample={total}"
            ));
        }
        f
    }

    /// Full ffmpeg arguments that write the clip's sound to `dst` (a 16-bit stereo `.wav`).
    pub fn audio_args(&self, looped: bool, total: u64, dst: &Path) -> Vec<String> {
        let mut args: Vec<String> = ["-y", "-hide_banner", "-nostdin", "-loglevel", "error"]
            .map(String::from)
            .to_vec();
        args.extend(self.input_args());
        args.extend([
            "-map".into(),
            "0:a:0".into(),
            "-af".into(),
            self.audio_filter(looped, total),
            "-ar".into(),
            CLIP_SAMPLE_RATE.to_string(),
            "-ac".into(),
            "2".into(),
            "-c:a".into(),
            "pcm_s16le".into(),
        ]);
        args.push(dst.display().to_string());
        args
    }
}

/// `atempo` factors whose product is `speed`, each within the filter's 0.5..=2 sweet spot.
pub fn atempo_chain(speed: f64) -> Vec<f64> {
    let mut out = Vec::new();
    if !(speed.is_finite() && speed > 0.0) || (speed - 1.0).abs() < 1e-9 {
        return out;
    }
    let mut s = speed;
    while s > 2.0 {
        out.push(2.0);
        s /= 2.0;
    }
    while s < 0.5 {
        out.push(0.5);
        s /= 0.5;
    }
    out.push(s);
    out
}

fn secs(ms: u64) -> String {
    format!("{}.{:03}", ms / 1000, ms % 1000)
}

#[cfg(test)]
mod tests {
    use super::*;

    const BANNER: &str = "Input #0, mov,mp4,m4a,3gp,3g2,mj2, from 'a.mp4':
  Metadata:
    major_brand     : isom
  Duration: 00:00:04.00, start: 0.000000, bitrate: 412 kb/s
  Stream #0:0[0x1](und): Video: h264 (High) (avc1 / 0x31637661), yuv420p(progressive), 640x360 [SAR 1:1 DAR 16:9], 403 kb/s, 30 fps, 30 tbr, 15360 tbn (default)
  Stream #0:1[0x2](und): Audio: aac (LC) (mp4a / 0x6134706D), 48000 Hz, stereo, fltp, 2 kb/s (default)
At least one output file must be specified";

    #[test]
    fn parses_size_rate_and_audio() {
        let info = parse_video_info(BANNER).unwrap();
        assert_eq!(
            info,
            VideoInfo {
                duration_ms: 4_000,
                width: 640,
                height: 360,
                fps: Some(30.0),
                has_audio: true,
            }
        );
        let webm = "  Duration: 00:00:02.50, start: 0.000000, bitrate: 1 kb/s
  Stream #0:0: Video: vp9 (Profile 0), yuv420p(tv, progressive), 1280x720, SAR 1:1 DAR 16:9, 29.97 fps, 29.97 tbr, 1k tbn (default)";
        let info = parse_video_info(webm).unwrap();
        assert_eq!(
            (info.width, info.height, info.fps),
            (1280, 720, Some(29.97))
        );
        assert!(!info.has_audio);
    }

    #[test]
    fn audio_only_and_cover_art_and_garbage_are_not_video() {
        let audio = "  Duration: 00:00:02.00, start: 0.0, bitrate: 1 kb/s
  Stream #0:0: Audio: mp3, 44100 Hz, stereo, fltp, 128 kb/s";
        assert_eq!(parse_video_info(audio), None);
        let art = "  Duration: 00:00:02.00, start: 0.0, bitrate: 1 kb/s
  Stream #0:0: Audio: mp3, 44100 Hz, stereo, fltp, 128 kb/s
  Stream #0:1: Video: mjpeg (Baseline), yuvj420p(pc), 500x500, 90k tbr, 90k tbn (attached pic)";
        assert_eq!(parse_video_info(art), None);
        assert_eq!(parse_video_info("moov atom not found"), None);
    }

    #[test]
    fn missing_file_is_a_clear_error() {
        let err = probe_video(Path::new("ffmpeg"), Path::new("/no/such/clip.mp4")).unwrap_err();
        assert_eq!(err.to_string(), "video /no/such/clip.mp4: file not found");
    }

    #[test]
    fn atempo_chain_multiplies_to_the_speed() {
        assert!(atempo_chain(1.0).is_empty());
        assert_eq!(atempo_chain(1.5), vec![1.5]);
        assert_eq!(atempo_chain(4.0), vec![2.0, 2.0]);
        assert_eq!(atempo_chain(0.25), vec![0.5, 0.5]);
        for s in [0.1, 0.3, 0.7, 3.3, 7.9] {
            let chain = atempo_chain(s);
            let p: f64 = chain.iter().product();
            assert!((p - s).abs() < 1e-9, "{s}: {chain:?}");
            assert!(chain.iter().all(|t| (0.5..=2.0).contains(t)));
        }
    }

    fn job() -> ClipJob {
        ClipJob {
            src: PathBuf::from("/p/a.mp4"),
            trim_start_ms: 1_500,
            source_ms: 3_000,
            speed: 2.0,
            fps: 30,
            frames: 45,
            max: Resolution {
                width: 1920,
                height: 1080,
            },
        }
    }

    #[test]
    fn video_args_trim_speed_and_force_the_frame_count() {
        let args = job().video_args(Path::new("/g/x.mp4"));
        let s = args.join(" ");
        assert!(s.contains("-ss 1.500 -t 3.000 -i /p/a.mp4"), "{s}");
        assert!(s.contains("setpts=(PTS-STARTPTS)/2.000000,fps=30,"), "{s}");
        assert!(
            s.contains("tpad=stop_mode=clone:stop=45 -frames:v 45"),
            "{s}"
        );
        assert!(
            s.contains("-g 15 -keyint_min 15 -sc_threshold 0 -bf 0"),
            "{s}"
        );
        assert!(s.ends_with("-an -movflags +faststart /g/x.mp4"));
    }

    #[test]
    fn audio_filter_is_one_pass_or_looped_to_the_span() {
        let j = job();
        assert_eq!(j.clip_samples(), 72_000);
        assert_eq!(
            j.audio_filter(false, 0),
            "asetpts=PTS-STARTPTS,aresample=48000,aformat=sample_fmts=fltp:channel_layouts=stereo,\
             atempo=2.000000,atrim=end_sample=72000,apad=whole_len=72000"
        );
        assert!(j
            .audio_filter(true, 200_000)
            .ends_with(",aloop=loop=-1:size=72000,atrim=end_sample=200000"));
    }

    #[test]
    fn keys_and_fingerprints_change_with_their_inputs() {
        let a = job();
        let mut b = job();
        b.speed = 1.0;
        assert_ne!(a.key(), b.key());
        assert_eq!(fingerprint_str("abc"), fingerprint_str("abc"));
        assert_ne!(fingerprint_str("abc"), fingerprint_str("abd"));
        assert_eq!(fingerprint_str("").len(), 32);
    }
}
