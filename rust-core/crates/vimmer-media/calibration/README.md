# Narration-length calibration

`src/estimate.rs` predicts how long a narration clip will be without synthesizing it. Its
per-engine, per-language costs come from here.

| file | what |
|---|---|
| `corpus.tsv` | 62 sentences (zh 20, ja 20, en 22): short and long, numbers, heavy punctuation, Latin words inside CJK |
| `measured.csv` | measured clip length (ms) per sentence × engine, with the voice used |
| `fit.py` | fits the costs (stdlib Python) and prints them plus the error table, old estimators vs new |

## Re-measure (after a voice, model or engine update)

From `rust-core/`:

```bash
cargo run -p vimmer-media --example measure_tts --release
```

```bash
python3 -I crates/vimmer-media/calibration/fit.py
```

`measure_tts` synthesizes every sentence with `say` (default voice for the text: Tingting,
Kyoko, Samantha) and Kokoro (zf_001, jf_alpha, af_heart), through the same code the render's
narration step uses, and measures each file the way the scheduler does (ffmpeg's duration of
the whole file, silence included: the render waits for it too). Clips are cached in
`$TMPDIR/vimmer-tts-calibration`; delete that directory after a voice update. `--engines say`
skips Kokoro when it is not installed.

Then copy the printed rates into `speech_rates()` in `src/estimate.rs` (between the
`FITTED-RATES` markers) and run `cargo test -p vimmer-media estimate`: the
`calibration_corpus_error` test fails when the mean error of any engine × language exceeds
its bound, so stale rates show up once `measured.csv` is updated.
