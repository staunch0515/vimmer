//! Render options as a front end (the CLI's flags, the desktop app's render panel) chooses them,
//! and the one place that turns them into a [`SchedulerConfig`], so both front ends render the
//! same way.

use std::path::PathBuf;

use vimmer_media::TtsEngine;

use crate::plan::DEFAULT_CHUNK_MS;
use crate::run::SchedulerConfig;
use crate::wordsubs::AutoSubtitles;

/// The user-facing render choices. `Default` is what the CLI does without flags.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RenderOptions {
    /// Target length of each render chunk, in milliseconds.
    pub chunk_ms: u64,
    /// Parallel image normalization jobs.
    pub preprocess_jobs: usize,
    /// Parallel Remotion render jobs.
    pub render_jobs: usize,
    /// Word-by-word subtitle timing from the synthesized narration.
    pub word_subtitles: bool,
    /// Which narrated scenes get generated subtitle cues.
    pub auto_subtitles: AutoSubtitles,
    /// `None`: `$VIMMER_WHISPER_CLI`, then `whisper-cli` on PATH.
    pub whisper_cli: Option<PathBuf>,
    /// `None`: `$VIMMER_WHISPER_MODEL`, then the default model.
    pub whisper_model: Option<PathBuf>,
    /// A whisper code or `auto`. `None`: guessed from each scene's text.
    pub whisper_language: Option<String>,
    /// `None`: `$VIMMER_TTS`, else `say`.
    pub tts: Option<TtsEngine>,
    /// `None`: `$VIMMER_KOKORO_PYTHON`, then the default venv.
    pub kokoro_python: Option<PathBuf>,
    /// `None`: `$VIMMER_KOKORO_MODELS`, then the default directory.
    pub kokoro_models: Option<PathBuf>,
}

impl Default for RenderOptions {
    fn default() -> Self {
        Self {
            chunk_ms: DEFAULT_CHUNK_MS,
            preprocess_jobs: 4,
            render_jobs: 2,
            word_subtitles: true,
            auto_subtitles: AutoSubtitles::Off,
            whisper_cli: None,
            whisper_model: None,
            whisper_language: None,
            tts: None,
            kokoro_python: None,
            kokoro_models: None,
        }
    }
}

impl RenderOptions {
    /// The scheduler config for these options. Fields the options do not cover (ffmpeg, the
    /// child PATH, the public dir, the Kokoro timeout) keep [`SchedulerConfig::new`]'s defaults.
    pub fn config(self, work_dir: PathBuf, output: PathBuf, remotion_dir: PathBuf) -> SchedulerConfig {
        let mut cfg = SchedulerConfig::new(work_dir, output, remotion_dir);
        cfg.chunk_ms = self.chunk_ms;
        cfg.preprocess_concurrency = self.preprocess_jobs;
        cfg.render_concurrency = self.render_jobs;
        cfg.word_subtitles.enabled = self.word_subtitles;
        cfg.word_subtitles.auto_subtitles = self.auto_subtitles;
        cfg.word_subtitles.whisper_cli = self.whisper_cli;
        cfg.word_subtitles.model = self.whisper_model;
        cfg.word_subtitles.language = self.whisper_language;
        cfg.tts.engine = self.tts;
        cfg.tts.kokoro_python = self.kokoro_python;
        cfg.tts.kokoro_models = self.kokoro_models;
        cfg
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dirs() -> (PathBuf, PathBuf, PathBuf) {
        (PathBuf::from("w"), PathBuf::from("o.mp4"), PathBuf::from("r"))
    }

    #[test]
    fn defaults_give_the_plain_scheduler_config() {
        let (w, o, r) = dirs();
        let built = RenderOptions::default().config(w.clone(), o.clone(), r.clone());
        assert_eq!(format!("{built:?}"), format!("{:?}", SchedulerConfig::new(w, o, r)));
    }

    #[test]
    fn every_option_reaches_the_config() {
        let (w, o, r) = dirs();
        let opts = RenderOptions {
            chunk_ms: 7_000,
            preprocess_jobs: 3,
            render_jobs: 5,
            word_subtitles: false,
            auto_subtitles: AutoSubtitles::All,
            whisper_cli: Some("wc".into()),
            whisper_model: Some("m.bin".into()),
            whisper_language: Some("ja".into()),
            tts: Some(TtsEngine::Kokoro),
            kokoro_python: Some("py".into()),
            kokoro_models: Some("models".into()),
        };
        let cfg = opts.config(w, o, r.clone());
        assert_eq!(cfg.chunk_ms, 7_000);
        assert_eq!(cfg.preprocess_concurrency, 3);
        assert_eq!(cfg.render_concurrency, 5);
        assert!(!cfg.word_subtitles.enabled);
        assert_eq!(cfg.word_subtitles.auto_subtitles, AutoSubtitles::All);
        assert_eq!(cfg.word_subtitles.whisper_cli, Some("wc".into()));
        assert_eq!(cfg.word_subtitles.model, Some("m.bin".into()));
        assert_eq!(cfg.word_subtitles.language.as_deref(), Some("ja"));
        assert_eq!(cfg.tts.engine, Some(TtsEngine::Kokoro));
        assert_eq!(cfg.tts.kokoro_python, Some("py".into()));
        assert_eq!(cfg.tts.kokoro_models, Some("models".into()));
        // Untouched: derived from the Remotion dir, and the scheduler's own defaults.
        assert_eq!(cfg.public_dir, r.join("public"));
        assert_eq!(cfg.ffmpeg, PathBuf::from("ffmpeg"));
        assert!(cfg.child_path.is_none());
        assert!(cfg.tts.kokoro_timeout.is_none());
    }
}
