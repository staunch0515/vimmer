//! Beat detection for background music, with no dependencies beyond ffmpeg for decoding.
//!
//! 1. The file is decoded to mono 32-bit float PCM at [`ANALYSIS_RATE`].
//! 2. Onset strength: log-compressed magnitude spectra (Hann window of [`WIN`] samples every
//!    [`HOP`], a hand-written radix-2 FFT) and their positive difference from one frame to the
//!    next, summed over bins (spectral flux). A moving average is taken off and the result
//!    rectified and scaled to unit deviation.
//! 3. Tempo: autocorrelation of the onset envelope over lags of 60–200 BPM, weighted toward
//!    120 BPM on a log scale (so the octave a listener would tap is preferred over its double or
//!    half), refined between lags with a parabola.
//! 4. Beats: dynamic programming in the manner of Ellis (2007): every frame's score is its onset
//!    strength plus the best score of a predecessor about one period earlier, penalised by how far
//!    the gap is from the period (squared log ratio). Backtracking from the best final frame gives
//!    the beat sequence; weak beats at either end (before the music starts, after it fades) are
//!    dropped, and each beat is refined to the onset peak between frames.
//! 5. Downbeat: the beat phase (of four) with the most low-frequency onset energy (the kick).
//! 6. Confidence, 0..1: how periodic the onsets are at the chosen period and how many beats land
//!    on an actual onset. Music without a clear beat (a pad) scores low.
//!
//! Every step is plain arithmetic on the decoded samples, so the same file always gives the same
//! result.

use std::f64::consts::PI;
use std::path::Path;

use crate::MediaError;

/// Sample rate the music is analysed at.
pub const ANALYSIS_RATE: u32 = 22_050;
/// FFT window length in samples (46 ms).
pub const WIN: usize = 1024;
/// Hop between analysis frames in samples (5.8 ms).
pub const HOP: usize = 128;
/// Tempo search range.
pub const MIN_BPM: f64 = 60.0;
pub const MAX_BPM: f64 = 200.0;
/// Below this, beat sync treats the detected beats as unreliable (`vimmer check` warns).
pub const LOW_CONFIDENCE: f64 = 0.5;
/// Bumped whenever the algorithm changes, so cached results are recomputed.
pub const ALGORITHM_VERSION: u32 = 1;

/// The beats of a piece of music.
#[derive(Debug, Clone, PartialEq)]
pub struct BeatAnalysis {
    /// Tempo in beats per minute (from the slope of the beat times).
    pub bpm: f64,
    /// 0..1; below [`LOW_CONFIDENCE`] the beats are a guess.
    pub confidence: f64,
    /// Beat times in milliseconds from the start of the file.
    pub beats_ms: Vec<f64>,
    /// Index into `beats_ms` of the first beat that starts a bar (a guess, for 4/4).
    pub downbeat: usize,
    /// Length of the decoded audio in milliseconds.
    pub duration_ms: f64,
}

/// Decodes `path` with ffmpeg (mono, [`ANALYSIS_RATE`]) and analyses it.
pub fn detect_beats(ffmpeg: &Path, path: &Path) -> Result<BeatAnalysis, MediaError> {
    let samples = decode_mono(ffmpeg, path)?;
    if samples.len() < ANALYSIS_RATE as usize * 2 {
        return Err(MediaError::Probe {
            path: path.display().to_string(),
            reason: "less than 2 s of audio, too short to find a beat".into(),
        });
    }
    Ok(analyze(&samples, ANALYSIS_RATE))
}

/// [`detect_beats`] through a cache in `cache_dir`, keyed by a fingerprint of the file's bytes and
/// [`ALGORITHM_VERSION`]: an unchanged file is decoded and analysed only once. A cache file that
/// cannot be read or written is ignored.
pub fn detect_beats_cached(
    ffmpeg: &Path,
    path: &Path,
    cache_dir: &Path,
) -> Result<BeatAnalysis, MediaError> {
    let key = crate::video_norm::fingerprint(path)?;
    let cache = cache_dir.join(format!("{key}-v{ALGORITHM_VERSION}.txt"));
    if let Some(hit) = std::fs::read_to_string(&cache)
        .ok()
        .and_then(|text| BeatAnalysis::from_cache_text(&text))
    {
        return Ok(hit);
    }
    let analysis = detect_beats(ffmpeg, path)?;
    if std::fs::create_dir_all(cache_dir).is_ok() {
        let tmp = cache.with_extension("tmp");
        if std::fs::write(&tmp, analysis.to_cache_text()).is_ok() {
            let _ = std::fs::rename(&tmp, &cache);
        }
    }
    Ok(analysis)
}

impl BeatAnalysis {
    /// A small line-based text form for the cache (`key value` lines; beats comma-separated).
    pub fn to_cache_text(&self) -> String {
        let beats: Vec<String> = self.beats_ms.iter().map(|b| format!("{b:?}")).collect();
        format!(
            "vimmer-beats {ALGORITHM_VERSION}\nbpm {:?}\nconfidence {:?}\ndownbeat {}\nduration_ms {:?}\nbeats_ms {}\n",
            self.bpm,
            self.confidence,
            self.downbeat,
            self.duration_ms,
            beats.join(",")
        )
    }

    pub fn from_cache_text(text: &str) -> Option<Self> {
        let mut lines = text.lines();
        if lines.next()? != format!("vimmer-beats {ALGORITHM_VERSION}") {
            return None;
        }
        let mut field = |name: &str| -> Option<String> {
            let line = lines.next()?;
            Some(
                line.strip_prefix(name)?
                    .strip_prefix(' ')
                    .unwrap_or("")
                    .to_string(),
            )
        };
        let bpm = field("bpm")?.parse().ok()?;
        let confidence = field("confidence")?.parse().ok()?;
        let downbeat = field("downbeat")?.parse().ok()?;
        let duration_ms = field("duration_ms")?.parse().ok()?;
        let beats = field("beats_ms")?;
        let beats_ms = if beats.is_empty() {
            Vec::new()
        } else {
            beats
                .split(',')
                .map(|b| b.parse().ok())
                .collect::<Option<Vec<f64>>>()?
        };
        Some(Self {
            bpm,
            confidence,
            beats_ms,
            downbeat,
            duration_ms,
        })
    }
}

/// Mono f32 samples of the whole file at [`ANALYSIS_RATE`].
pub fn decode_mono(ffmpeg: &Path, path: &Path) -> Result<Vec<f32>, MediaError> {
    if !path.is_file() {
        return Err(MediaError::Probe {
            path: path.display().to_string(),
            reason: "file not found".into(),
        });
    }
    let output = std::process::Command::new(ffmpeg)
        .args(["-hide_banner", "-nostdin", "-loglevel", "error", "-i"])
        .arg(path)
        .args(["-map", "0:a:0", "-ac", "1", "-ar"])
        .arg(ANALYSIS_RATE.to_string())
        .args(["-f", "f32le", "-"])
        .output()?;
    if !output.status.success() {
        return Err(MediaError::Probe {
            path: path.display().to_string(),
            reason: String::from_utf8_lossy(&output.stderr).trim().to_string(),
        });
    }
    Ok(output
        .stdout
        .as_chunks::<4>()
        .0
        .iter()
        .map(|b| f32::from_le_bytes(*b))
        .collect())
}

/// Frames per second of the onset envelope.
fn frame_rate(rate: u32) -> f64 {
    f64::from(rate) / HOP as f64
}

/// Analyses mono samples at `rate`.
pub fn analyze(samples: &[f32], rate: u32) -> BeatAnalysis {
    let duration_ms = samples.len() as f64 * 1000.0 / f64::from(rate);
    let Envelopes { onset, low } = onset_envelopes(samples);
    let fr = frame_rate(rate);
    let empty = BeatAnalysis {
        bpm: 0.0,
        confidence: 0.0,
        beats_ms: Vec::new(),
        downbeat: 0,
        duration_ms,
    };
    if onset.len() < (fr * 2.0) as usize || onset.iter().all(|&v| v == 0.0) {
        return empty;
    }
    let Some((period, periodicity)) = estimate_period(&onset, fr) else {
        return empty;
    };
    let frames = track_beats(&onset, period);
    if frames.len() < 2 {
        return empty;
    }
    let mut beats_ms: Vec<f64> = frames.iter().map(|&f| f * 1000.0 / fr).collect();
    refine_to_attacks(samples, rate, &mut beats_ms);
    let bpm = 60_000.0 / slope(&beats_ms);
    let downbeat = guess_downbeat(&low, &frames);
    // How many beats sit on a real onset: the envelope at the beat against its own spread.
    let on_onset = frames
        .iter()
        .filter(|&&f| peak_near(&onset, f) > 1.0)
        .count() as f64
        / frames.len() as f64;
    let confidence = (periodicity * on_onset).sqrt().clamp(0.0, 1.0);
    BeatAnalysis {
        bpm,
        confidence,
        beats_ms,
        downbeat,
        duration_ms,
    }
}

/// Least-squares slope of `values` against their index.
fn slope(values: &[f64]) -> f64 {
    let n = values.len() as f64;
    let mean_x = (n - 1.0) / 2.0;
    let mean_y = values.iter().sum::<f64>() / n;
    let (mut num, mut den) = (0.0, 0.0);
    for (i, &y) in values.iter().enumerate() {
        let dx = i as f64 - mean_x;
        num += dx * (y - mean_y);
        den += dx * dx;
    }
    num / den
}

/// Number of log-spaced bands the spectrum is grouped into (from [`BAND_LOW_HZ`] to Nyquist).
const BANDS: usize = 24;
const BAND_LOW_HZ: f64 = 30.0;

/// Onset envelopes, one value per hop. Frame `k` is centred on sample `k * HOP` (the signal is
/// padded by half a window in front). The spectrum is grouped into log-spaced bands first, so a
/// broadband hi-hat (many FFT bins) does not outweigh a kick (a few low bins): each octave counts
/// about the same.
fn onset_envelopes(samples: &[f32]) -> Envelopes {
    let window: Vec<f64> = (0..WIN)
        .map(|i| 0.5 - 0.5 * (2.0 * PI * i as f64 / WIN as f64).cos())
        .collect();
    let bins = WIN / 2;
    let hz_per_bin = f64::from(ANALYSIS_RATE) / WIN as f64;
    let nyquist = f64::from(ANALYSIS_RATE) / 2.0;
    // Band of each bin (bins below the lowest edge are ignored).
    let band_of: Vec<Option<usize>> = (0..bins)
        .map(|b| {
            let hz = b as f64 * hz_per_bin;
            if hz < BAND_LOW_HZ {
                return None;
            }
            let pos = (hz / BAND_LOW_HZ).ln() / (nyquist / BAND_LOW_HZ).ln();
            Some(((pos * BANDS as f64) as usize).min(BANDS - 1))
        })
        .collect();
    let mut bin_count = [0usize; BANDS];
    for band in band_of.iter().flatten() {
        bin_count[*band] += 1;
    }
    let low_band = (0..BANDS)
        .filter(|&k| {
            let top = BAND_LOW_HZ * (nyquist / BAND_LOW_HZ).powf((k + 1) as f64 / BANDS as f64);
            top <= 160.0
        })
        .count()
        .max(1);
    // A 20 ms fade at the end: a file that stops abruptly would otherwise look like a broadband
    // onset there. The start is not faded: a beat on the first sample is a real onset.
    let fade = (f64::from(ANALYSIS_RATE) * 0.02) as usize;
    let len = samples.len();
    let at = |i: isize| -> f64 {
        if i < 0 || i as usize >= len {
            return 0.0;
        }
        let i = i as usize;
        let edge = len - 1 - i;
        let gain = if edge < fade {
            0.5 - 0.5 * (PI * edge as f64 / fade as f64).cos()
        } else {
            1.0
        };
        samples[i] as f64 * gain
    };
    let frames = len / HOP + 1;
    let mut prev = [0.0f64; BANDS];
    let mut flux = Vec::with_capacity(frames);
    let mut low = Vec::with_capacity(frames);
    let mut re = vec![0.0; WIN];
    let mut im = vec![0.0; WIN];
    for k in 0..frames {
        let start = (k * HOP) as isize - (WIN / 2) as isize;
        for i in 0..WIN {
            re[i] = at(start + i as isize) * window[i];
            im[i] = 0.0;
        }
        fft(&mut re, &mut im);
        let mut power = [0.0f64; BANDS];
        for (b, band) in band_of.iter().enumerate() {
            if let Some(band) = band {
                power[*band] += re[b] * re[b] + im[b] * im[b];
            }
        }
        let (mut total, mut bass) = (0.0, 0.0);
        for band in 0..BANDS {
            if bin_count[band] == 0 {
                continue;
            }
            let s = (1.0 + 100.0 * (power[band] / bin_count[band] as f64).sqrt()).ln();
            let d = s - prev[band];
            if d > 0.0 {
                total += d;
                if band < low_band {
                    bass += d;
                }
            }
            prev[band] = s;
        }
        flux.push(total);
        low.push(bass);
    }
    Envelopes {
        onset: normalize(&flux),
        low,
    }
}

/// Moves each beat (ms) to the attack of the sound under it: the steepest rise of the signal's
/// energy (1 ms steps, 4 ms windows) within [`REFINE_MS`] of the beat. The spectral onset of a
/// 46 ms window fires as soon as an attack enters the window, about 10–15 ms early; this puts the
/// beat back on the attack itself. A beat with no clear rise nearby stays where it is.
const REFINE_MS: f64 = 30.0;

fn refine_to_attacks(samples: &[f32], rate: u32, beats_ms: &mut [f64]) {
    let step = (rate / 1000).max(1) as usize; // 1 ms
    let win = 4 * step;
    let energy = |at: usize| -> f64 {
        samples[at.min(samples.len())..(at + win).min(samples.len())]
            .iter()
            .map(|&v| f64::from(v) * f64::from(v))
            .sum()
    };
    for beat in beats_ms.iter_mut() {
        let centre = (*beat * f64::from(rate) / 1000.0) as isize;
        let reach = (REFINE_MS * f64::from(rate) / 1000.0) as isize;
        let from = (centre - reach).max(0) as usize;
        let to = ((centre + reach).max(0) as usize).min(samples.len().saturating_sub(win));
        if from + win >= to {
            continue;
        }
        let mut best: Option<(f64, usize)> = None;
        let mut base = 0.0f64;
        let mut j = from;
        while j + win <= to {
            let before = if j >= win { energy(j - win) } else { 0.0 };
            let after = energy(j);
            base = base.max(after);
            let rise = after - before;
            if best.is_none_or(|(r, _)| rise > r) {
                best = Some((rise, j));
            }
            j += step;
        }
        if let Some((rise, j)) = best {
            // Only a real attack: the rise must be most of the loudest energy around.
            if rise > 0.5 * base && base > 0.0 {
                *beat = j as f64 * 1000.0 / f64::from(rate);
            }
        }
    }
}

struct Envelopes {
    /// Onset strength, in units of its own standard deviation.
    onset: Vec<f64>,
    /// Low-band (kick) spectral flux.
    low: Vec<f64>,
}

/// Removes a moving average (about 0.4 s), rectifies, and scales to unit standard deviation.
fn normalize(flux: &[f64]) -> Vec<f64> {
    let half = 35usize;
    let mut prefix = Vec::with_capacity(flux.len() + 1);
    prefix.push(0.0);
    for v in flux {
        prefix.push(prefix.last().unwrap() + v);
    }
    let mut out: Vec<f64> = (0..flux.len())
        .map(|k| {
            let a = k.saturating_sub(half);
            let b = (k + half + 1).min(flux.len());
            let mean = (prefix[b] - prefix[a]) / (b - a) as f64;
            (flux[k] - mean).max(0.0)
        })
        .collect();
    let n = out.len() as f64;
    let mean = out.iter().sum::<f64>() / n;
    let sd = (out.iter().map(|v| (v - mean).powi(2)).sum::<f64>() / n).sqrt();
    if sd > 1e-12 {
        for v in &mut out {
            *v /= sd;
        }
    }
    out
}

/// In-place iterative radix-2 FFT. `re.len()` must be a power of two.
pub fn fft(re: &mut [f64], im: &mut [f64]) {
    let n = re.len();
    debug_assert!(n.is_power_of_two() && im.len() == n);
    let mut j = 0;
    for i in 1..n {
        let mut bit = n >> 1;
        while j & bit != 0 {
            j ^= bit;
            bit >>= 1;
        }
        j |= bit;
        if i < j {
            re.swap(i, j);
            im.swap(i, j);
        }
    }
    let mut len = 2;
    while len <= n {
        let ang = -2.0 * PI / len as f64;
        let (wr, wi) = (ang.cos(), ang.sin());
        for start in (0..n).step_by(len) {
            let (mut cr, mut ci) = (1.0, 0.0);
            for k in 0..len / 2 {
                let (a, b) = (start + k, start + k + len / 2);
                let tr = re[b] * cr - im[b] * ci;
                let ti = re[b] * ci + im[b] * cr;
                re[b] = re[a] - tr;
                im[b] = im[a] - ti;
                re[a] += tr;
                im[a] += ti;
                let next = cr * wr - ci * wi;
                ci = cr * wi + ci * wr;
                cr = next;
            }
        }
        len <<= 1;
    }
}

/// Beat period in frames (fractional) and how periodic the envelope is at it (0..1: the
/// normalized autocorrelation there).
fn estimate_period(onset: &[f64], fr: f64) -> Option<(f64, f64)> {
    let n = onset.len();
    let mean = onset.iter().sum::<f64>() / n as f64;
    let x: Vec<f64> = onset.iter().map(|v| v - mean).collect();
    let ac = |lag: usize| -> f64 {
        if lag >= n {
            return 0.0;
        }
        x[..n - lag]
            .iter()
            .zip(&x[lag..])
            .map(|(a, b)| a * b)
            .sum::<f64>()
            / (n - lag) as f64
    };
    let ac0 = ac(0);
    if ac0 <= 0.0 {
        return None;
    }
    let min_lag = (60.0 * fr / MAX_BPM).floor() as usize;
    let max_lag = (60.0 * fr / MIN_BPM).ceil() as usize;
    if max_lag + 2 >= n {
        return None;
    }
    let values: Vec<f64> = (0..=max_lag + 1).map(ac).collect();
    let weight = |lag: f64| {
        let bpm = 60.0 * fr / lag;
        (-0.5 * ((bpm / 120.0).log2() / 1.0).powi(2)).exp()
    };
    let best = (min_lag.max(1)..=max_lag)
        .filter(|&l| values[l] >= values[l - 1] && values[l] >= values[l + 1])
        .max_by(|&a, &b| {
            (values[a] * weight(a as f64)).total_cmp(&(values[b] * weight(b as f64)))
        })?;
    if values[best] <= 0.0 {
        return None;
    }
    let (y0, y1, y2) = (values[best - 1], values[best], values[best + 1]);
    let den = y0 - 2.0 * y1 + y2;
    let delta = if den.abs() > 1e-12 {
        (0.5 * (y0 - y2) / den).clamp(-0.5, 0.5)
    } else {
        0.0
    };
    Some((best as f64 + delta, (y1 / ac0).clamp(0.0, 1.0)))
}

/// Ellis-style dynamic programming beat tracker. Returns beat positions in (fractional) frames.
fn track_beats(onset: &[f64], period: f64) -> Vec<f64> {
    const TIGHTNESS: f64 = 100.0;
    let n = onset.len();
    // Local score: the onset envelope smoothed with a Gaussian of period/32.
    let sigma = (period / 32.0).max(0.5);
    let radius = (sigma * 3.0).ceil() as isize;
    let kernel: Vec<f64> = (-radius..=radius)
        .map(|i| (-0.5 * (i as f64 / sigma).powi(2)).exp())
        .collect();
    let local: Vec<f64> = (0..n as isize)
        .map(|k| {
            kernel
                .iter()
                .enumerate()
                .map(|(j, w)| {
                    let idx = k + j as isize - radius;
                    if idx < 0 || idx >= n as isize {
                        0.0
                    } else {
                        w * onset[idx as usize]
                    }
                })
                .sum()
        })
        .collect();
    let lo = (period / 2.0).round() as usize;
    let hi = (period * 2.0).round() as usize;
    let mut score = vec![0.0; n];
    let mut back: Vec<Option<usize>> = vec![None; n];
    for k in 0..n {
        let mut best: Option<(f64, usize)> = None;
        if k >= lo {
            let first = k.saturating_sub(hi);
            for (tau, prev) in score.iter().enumerate().take(k - lo + 1).skip(first) {
                let gap = (k - tau) as f64 / period;
                let s = prev - TIGHTNESS * gap.ln().powi(2);
                if best.is_none_or(|(b, _)| s > b) {
                    best = Some((s, tau));
                }
            }
        }
        match best {
            Some((s, tau)) if s > 0.0 => {
                score[k] = local[k] + s;
                back[k] = Some(tau);
            }
            _ => score[k] = local[k],
        }
    }
    // The last beat: the best-scoring local peak in the final period.
    let tail = n.saturating_sub(period.ceil() as usize);
    let mut last = (tail..n)
        .max_by(|&a, &b| score[a].total_cmp(&score[b]))
        .unwrap_or(0);
    let mut beats = vec![last];
    while let Some(prev) = back[last] {
        beats.push(prev);
        last = prev;
    }
    beats.reverse();
    // Drop weak beats at the ends: below half the RMS of the local score at the beats.
    let rms = (beats.iter().map(|&b| local[b].powi(2)).sum::<f64>() / beats.len() as f64).sqrt();
    let threshold = 0.5 * rms;
    let first = beats
        .iter()
        .position(|&b| local[b] >= threshold)
        .unwrap_or(0);
    let last = beats
        .iter()
        .rposition(|&b| local[b] >= threshold)
        .unwrap_or(0);
    let mut beats = if first <= last {
        &beats[first..=last]
    } else {
        &beats[..0]
    };
    // A beat at either end whose gap to its neighbour is off the period by more than 15 % is
    // not part of the pulse (the start of an intro, a final hit).
    let off = |a: usize, b: usize| ((b - a) as f64 - period).abs() > 0.15 * period;
    while beats.len() > 2 && off(beats[0], beats[1]) {
        beats = &beats[1..];
    }
    while beats.len() > 2 && off(beats[beats.len() - 2], beats[beats.len() - 1]) {
        beats = &beats[..beats.len() - 1];
    }
    beats.iter().map(|&b| refine_peak(onset, b)).collect()
}

/// The onset peak within two frames of `frame`, interpolated with a parabola.
fn refine_peak(onset: &[f64], frame: usize) -> f64 {
    let lo = frame.saturating_sub(2);
    let hi = (frame + 2).min(onset.len() - 1);
    let peak = (lo..=hi)
        .max_by(|&a, &b| onset[a].total_cmp(&onset[b]))
        .unwrap_or(frame);
    if peak == 0 || peak + 1 >= onset.len() || onset[peak] <= 0.0 {
        return peak as f64;
    }
    let (y0, y1, y2) = (onset[peak - 1], onset[peak], onset[peak + 1]);
    let den = y0 - 2.0 * y1 + y2;
    if den.abs() < 1e-12 {
        return peak as f64;
    }
    peak as f64 + (0.5 * (y0 - y2) / den).clamp(-0.5, 0.5)
}

fn peak_near(env: &[f64], frame: f64) -> f64 {
    let f = frame.round() as usize;
    let lo = f.saturating_sub(2);
    let hi = (f + 2).min(env.len().saturating_sub(1));
    (lo..=hi).map(|i| env[i]).fold(0.0, f64::max)
}

/// Index of the first beat of the phase (of four) with the most low-band onset energy.
fn guess_downbeat(low: &[f64], beats: &[f64]) -> usize {
    let mut sums = [0.0f64; 4];
    let mut counts = [0usize; 4];
    for (i, &b) in beats.iter().enumerate() {
        sums[i % 4] += peak_near(low, b);
        counts[i % 4] += 1;
    }
    (0..4.min(beats.len()))
        .max_by(|&a, &b| {
            (sums[a] / counts[a].max(1) as f64).total_cmp(&(sums[b] / counts[b].max(1) as f64))
        })
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A kick (falling sine, 120 ms decay) on every beat, louder on each bar's first beat.
    fn click_track(bpm: f64, first_ms: f64, seconds: f64, rate: u32) -> Vec<f32> {
        let period = 60.0 / bpm;
        let first = first_ms / 1000.0;
        (0..(seconds * f64::from(rate)) as usize)
            .map(|i| {
                let t = i as f64 / f64::from(rate);
                if t < first {
                    return 0.0;
                }
                let u = (t - first) % period;
                let idx = ((t - first) / period).floor() as i64;
                let accent = if idx % 4 == 0 { 1.0 } else { 0.6 };
                let phase = 2.0 * PI * (50.0 * u + 70.0 * 0.03 * (1.0 - (-u / 0.03).exp()));
                (accent * 0.8 * phase.sin() * (-u / 0.12).exp()) as f32
            })
            .collect()
    }

    #[test]
    fn fft_matches_a_direct_dft() {
        let n = 16;
        let x: Vec<f64> = (0..n).map(|i| ((i * 7 % 5) as f64) - 1.5).collect();
        let (mut re, mut im) = (x.clone(), vec![0.0; n]);
        fft(&mut re, &mut im);
        for k in 0..n {
            let (mut r, mut m) = (0.0, 0.0);
            for (i, v) in x.iter().enumerate() {
                let a = -2.0 * PI * (k * i) as f64 / n as f64;
                r += v * a.cos();
                m += v * a.sin();
            }
            assert!(
                (re[k] - r).abs() < 1e-9 && (im[k] - m).abs() < 1e-9,
                "bin {k}"
            );
        }
    }

    #[test]
    fn finds_tempo_beats_and_downbeat_of_a_click_track() {
        for (bpm, first) in [(120.0, 0.0), (96.0, 750.0), (140.0, 300.0)] {
            let a = analyze(&click_track(bpm, first, 16.0, 22_050), 22_050);
            assert!((a.bpm - bpm).abs() < 1.0, "{bpm}: {}", a.bpm);
            assert!(a.confidence > LOW_CONFIDENCE, "{bpm}: {}", a.confidence);
            let period = 60_000.0 / bpm;
            for &b in &a.beats_ms {
                let k = ((b - first) / period).round();
                assert!((b - (first + k * period)).abs() < 30.0, "{bpm}: beat {b}");
            }
            let down = a.beats_ms[a.downbeat];
            let k = ((down - first) / period).round() as i64;
            assert_eq!(k % 4, 0, "{bpm}: downbeat {down}");
            assert!(a.beats_ms[0] < first + period + 30.0, "{bpm}: starts late");
        }
    }

    #[test]
    fn steady_tone_has_no_confident_beat() {
        let rate = 22_050;
        let s: Vec<f32> = (0..rate as usize * 8)
            .map(|i| {
                let t = i as f64 / f64::from(rate);
                (0.3 * (2.0 * PI * 220.0 * t).sin() + 0.2 * (2.0 * PI * 331.0 * t).sin()) as f32
            })
            .collect();
        let a = analyze(&s, rate);
        assert!(a.confidence < LOW_CONFIDENCE, "{}", a.confidence);
    }

    #[test]
    fn cache_text_round_trips_exactly() {
        let a = analyze(&click_track(123.0, 40.0, 6.0, 22_050), 22_050);
        assert_eq!(
            BeatAnalysis::from_cache_text(&a.to_cache_text()),
            Some(a.clone())
        );
        assert_eq!(BeatAnalysis::from_cache_text("vimmer-beats 0\nbpm 1"), None);
        let empty = BeatAnalysis {
            beats_ms: vec![],
            ..a
        };
        assert_eq!(
            BeatAnalysis::from_cache_text(&empty.to_cache_text()),
            Some(empty)
        );
    }

    #[test]
    fn analysis_is_deterministic() {
        let s = click_track(110.0, 100.0, 8.0, 22_050);
        assert_eq!(analyze(&s, 22_050), analyze(&s, 22_050));
    }
}
