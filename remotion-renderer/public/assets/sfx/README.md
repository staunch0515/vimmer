# Bundled sound effects

Short sounds for `scene.sfx` (see "Sound effects" in the top-level README). Every one is
synthesized by ffmpeg alone (`aevalsrc` expressions and `anoisesrc` with fixed seeds): no samples,
no third-party recordings, nothing downloaded, free to use. Mono 16-bit WAV at 32 kHz (the mix
resamples to 48 kHz). The sound starts on its first sample, so it plays exactly on its frame;
only the riser builds up.

Made by [`scripts/make-audio-assets.sh`](../../../scripts/make-audio-assets.sh) (deterministic: running it
again writes the same sounds; the exact expressions are there).

| File | Length | Peak | How it is made |
| --- | --- | --- | --- |
| `click.wav` | 40 ms | -1.2 dBFS | a 2 kHz sine with a 4 ms exponential decay |
| `pop.wav` | 150 ms | -1.3 dBFS | a sine falling from 900 to 300 Hz (phase = integral of the frequency), 30 ms decay, 1 ms attack |
| `ding.wav` | 1.2 s | -2.9 dBFS | bell partials at 1320, 2640, 3960 and 5280 Hz, each decaying faster than the one below, 2 ms attack |
| `whoosh.wav` | 0.6 s | -0.9 dBFS | pink noise band-passed at 900 Hz (peaks at 0.22 s) under white noise band-passed at 4 kHz (peaks at 0.34 s), so it sweeps up; a 15 ms attack to a third of the peak |
| `swoosh-down.wav` | 0.7 s | -0.9 dBFS | the reverse: the 5 kHz band first, then the 700 Hz band |
| `riser.wav` | 2 s | -1.0 dBFS | high-passed noise plus a sine gliding 200 → 1400 Hz, both swelling to full level at the end; start it 2 s before the moment it leads into |

The two noise sweeps go through `alimiter` (0.9) after summing the bands.
