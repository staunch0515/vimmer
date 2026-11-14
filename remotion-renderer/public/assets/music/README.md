# Beat test tracks

Music with a known beat grid, for testing beat detection and beat sync (`fixtures/beats-sfx-demo.json`).
Synthesized with ffmpeg alone by [`scripts/make-audio-assets.sh`](../../../scripts/make-audio-assets.sh):
nothing downloaded, free to use. Mono AAC 64 kb/s, 44.1 kHz.

Each beat is a kick (a sine falling from 120 to 50 Hz, 120 ms decay), louder on the first beat of
every 4-beat bar; a hi-hat (noise above 6 kHz, 12 ms decay) on every eighth, louder on the
off-beats; and a quiet 55 Hz bass note per bar.

| File | Tempo | Ground truth | Length |
| --- | --- | --- | --- |
| `beat-120bpm.m4a` | 120 BPM | a beat every 500 ms from 0 ms; bars start at 0, 2000, 4000 … | 20 s (40 beats, loops seamlessly) |
| `beat-96bpm-intro.m4a` | 96 BPM | 750 ms of a soft A-major chord, then a beat every 625 ms from 750 ms; bars start at 750, 3250 … | 20.75 s |

`vimmer beats` finds 120.00 BPM (first beats 0, 500, 1000 ms) and 96.00 BPM (first beat 750 ms)
with confidence above 0.9; `assets/pad-loop.m4a` (no beat) scores 0.11.
