# Music library

Background music chosen by mood: a timeline says `"music": {"pick": {"mood": "calm"}}` and the
render picks a track from this folder (see the main README, "Music library"). `library.json`
lists the tracks; `vimmer music list` prints them.

There are two kinds of tracks: six real tracks (CC0, from OpenGameArt) and six synthesized
placeholders. A pick prefers real tracks: a placeholder plays only when no real track has the
mood (`ambient`) or every real one is excluded.

## Real tracks

### Credits

All six are released under CC0 1.0 (public domain dedication) by their authors; the license was
checked on each page before downloading. No attribution is required, but here it is:

| id | title | artist | page | license |
| --- | --- | --- | --- | --- |
| `another-august` | Another August | cynicmusic | <https://opengameart.org/content/another-august> | CC0-1.0 |
| `bright-eyes` | Bright Eyes | tomcat | <https://opengameart.org/content/bright-eyes> | CC0-1.0 |
| `tension-theme` | Tension Theme | Umplix | <https://opengameart.org/content/tension-theme> | CC0-1.0 |
| `happy-clappy-loop` | Happy Clappy Loop | owlishmedia | <https://opengameart.org/content/happy-clappy-loop> | CC0-1.0 |
| `overdrive` | OverDrive [Final] | CenturionOfWar | <https://opengameart.org/content/overdrive-final> | CC0-1.0 |
| `ancient-legend` | The Ancient Legend | Vitalezzz | <https://opengameart.org/content/the-ancient-legend> | CC0-1.0 |

Each entry in `library.json` carries the same in `credit` (`title`, `artist`, `page`,
`license`), which `vimmer music list --json` and the desktop music panel pass on.

### The files

[`scripts/make-music-library-real.sh DIR`](../../../../scripts/make-music-library-real.sh)
makes them from the original downloads (not in the repository) the same way the placeholders are
finished: 44.1 kHz stereo, brought to -18 LUFS integrated, a limiter at -3 dBFS sample peak (it
hardly works: every source peaked below that after the gain), then the gain corrected once after
the limiter, AAC 128 kb/s with bit-exact flags (running it again writes the same files). Full
length everywhere; only the 3.5 s of digital silence at the end of The Ancient Legend is cut to
0.5 s.

| id | moods | energy | bpm (offset) | length | loop | LUFS | true peak | size |
| --- | --- | --- | --- | --- | --- | --- | --- | --- |
| `another-august` | calm, reflective | 0.20 | - | 206.6 s | no | -18.07 | -3.9 dBTP | 3.34 MB |
| `bright-eyes` | warm, hopeful, happy | 0.45 | 130 (11 ms) | 66.5 s | no | -18.24 | -6.4 dBTP | 1.08 MB |
| `tension-theme` | tense, suspense, dark | 0.50 | 120 (0 ms) | 43.0 s | no | -18.20 | -4.4 dBTP | 0.70 MB |
| `happy-clappy-loop` | playful, happy, upbeat | 0.60 | 110.09 (23 ms) | 17.4 s | yes | -18.10 | -4.7 dBTP | 0.29 MB |
| `overdrive` | upbeat, energetic | 0.80 | - | 81.2 s | no | -18.12 | -4.6 dBTP | 1.32 MB |
| `ancient-legend` | epic, dramatic, inspiring | 0.80 | - | 183.2 s | no | -18.04 | -3.4 dBTP | 2.96 MB |

(Loudness and true peak of the .m4a files, measured with ffmpeg `loudnorm`/`ebur128`.)

### How the values were chosen

Without listening: from each page's description, the tempo, and measurements made after
normalizing every source to -18 LUFS (high-frequency share above 3 kHz, low share below 200 Hz,
loudness range, short-term level spread), compared with the placeholders, whose energies are
known by construction.

- `another-august` 0.20: no steady beat (`vimmer beats` confidence 0.19, tempo different in
  every 20 s window), the widest dynamics in the library (loudness range 14.4 LU, quiet passages
  6 dB below the median level), little low end (-9.7 dB below 200 Hz). A little above
  `calm-drift` (0.15), which is darker and steadier.
- `bright-eyes` 0.45: a steady 130 BPM pulse and a bright top (-15 dB above 3 kHz, among the
  brightest) put it above `warm-hearth` (0.35); a narrow loudness range (3.0 LU) and moderate
  onsets keep it below the upbeat tracks.
- `tension-theme` 0.50: a strict 120 BPM grid, but sparse (the lowest onset rate of the six) and
  little bass; a bit below `tense-pulse` (0.55).
- `happy-clappy-loop` 0.60: claps on a constant 110 BPM, full bass, a flat level (1.3 LU);
  lively but light, next to `playful-hop` (0.65).
- `overdrive` 0.80: the brightest (-14.9 dB above 3 kHz) and most percussive real track, with
  driving syncopated drums; between `upbeat-sunny` (0.75) and `epic-rise` (0.90).
- `ancient-legend` 0.80: a full orchestral piece (loudness range 8.1 LU), big but sustained rather
  than drum-driven, so below the taiko-heavy `epic-rise` (0.90).

Moods use the library's existing tags; the first is the main one.

### Beat grids

Measured with the project's own detector (`vimmer beats`) on the encoded files, then checked by
folding the detected beats on candidate tempos over the whole track and in 20 s windows. A
`bpm` is set only where one grid holds across the track:

- `happy-clappy-loop`: 110 BPM detected, every beat on the grid. The loop is 769132 samples,
  exactly 32 beats at 110.09 BPM; that tempo keeps the grid on the beat when the render repeats
  the loop. First beat 23 ms in.
- `tension-theme`: 120.0 BPM, every detected beat within 50 ms (typically 11 ms) of the grid from 0 ms (the
  detector's own confidence is low, 0.26, because the piece is sparse).
- `bright-eyes`: 130 BPM in three of four 20 s windows (between 40 and 60 s the detected beats
  leave the grid), first beat 11 ms in.

The other three have no `bpm`, so beat sync detects their beats at render time (or skips with a
warning): `another-august` and `ancient-legend` have no steady pulse (confidence 0.19 and 0.23),
and `overdrive`'s syncopated drums read as 99.9 BPM on the source and 130 BPM on the encoded file,
with the detected beats drifting off any single grid.

### The loop seam

Happy Clappy Loop is processed as three copies in a row with the middle one kept, so the limiter
and the encoder see the loop's end before its start. Its source has a small step where the end
meets the start (the waveform jumps by about 7% of full scale at a quiet moment: a soft click,
larger than 99.9% of the sample-to-sample changes in the file). The script spreads that step over
5 ms on each side of the join with a raised-cosine ramp. Measured on the encoded file played
twice in a row: the sample at the join is predicted from the two before it within 39 (16-bit)
where the median prediction error elsewhere is 198, and the 2 ms high-frequency (above 6 kHz)
window at the join is -52.6 dBFS against a median window of -47.6 dBFS; the next clap starts
6 ms later.

## Placeholders

The six placeholders (`"placeholder": true`) were synthesized with ffmpeg alone by
[`scripts/make-music-library.sh`](../../../../scripts/make-music-library.sh): no samples, nothing
downloaded. They are public domain / CC0 by this project. They are simple (sine-based
instruments, a few echoes for room). They stay in the library so timelines that name them still
play, and they cover the `ambient` mood, which no real track has.

| id | mood tags | energy | tempo | loop | how it sounds |
| --- | --- | --- | --- | --- | --- |
| `calm-drift` | calm, reflective, ambient | 0.15 | 75 BPM | 51.2 s | breathing Cmaj7-Am7-Fmaj7-G6 pad, soft eighth-note arpeggio with a dotted-eighth echo, no drums |
| `warm-hearth` | warm, calm, hopeful | 0.35 | 80 BPM | 48.0 s | electric piano F-Dm-Bb-C, soft pad, half-note bass, soft kick, rim and shaker |
| `tense-pulse` | tense, suspense, dark | 0.55 | 90 BPM | 42.7 s | low A/E drone, tremolo strings Am-F-Dm-E, muted sixteenth pulse, heartbeat kick, ticking clock |
| `playful-hop` | playful, upbeat, happy | 0.65 | 128 BPM | 45.0 s | marimba-like melody over C-Am-F-G, staccato bass, kick, clap, hats, woodblock |
| `upbeat-sunny` | upbeat, happy, energetic | 0.75 | 120 BPM | 48.0 s | C-G-Am-F chords pumping on every beat (sidechain style), off-beat bass, plucked hook, four-on-the-floor |
| `epic-rise` | epic, inspiring, dramatic | 0.90 | 100 BPM | 48.0 s | brass-like Dm-Bb-F-C swelling through each 4-bar phrase, rising strings, taiko-like drums, snare roll, crash |

All are stereo AAC 128 kb/s, 44.1 kHz, 690-830 KB, -18 LUFS integrated (measured -18.0 to
-18.2), true peak -3.0 to -7.9 dBTP. Each loops seamlessly: every sound repeats exactly once per
loop (notes are written as "time since the strike", held tones use frequencies with a whole number
of periods in the loop), and the file is the second of two rendered loops, so it starts with the
echoes and note tails of its own end. The joint's high-frequency energy is that of an ordinary
phrase start (calm-drift, which has no drums, shows a faint AAC edge at about -40 dBFS).

`vimmer beats` finds the tempo of five of them within 0.01 BPM. `calm-drift` reads as 150 BPM
(its eighth-note arpeggio; it has no percussion), and `epic-rise` is found at 100 BPM but with low
confidence (syncopated drums). Neither matters for beat sync: a picked track's beat grid comes
from `bpm` and `beat_offset_ms` here, not from detection.

## Adding real tracks (no code change)

1. Put the file in this folder (anything ffmpeg reads; M4A/AAC or MP3 at 44.1 or 48 kHz).
   Bring it to roughly -18 LUFS so tracks sit at the same level, for example
   `ffmpeg -i in.wav -af loudnorm=I=-18:TP=-1.5 -c:a aac -b:a 160k my-track.m4a`.
2. Add an entry to `library.json`:

```json
{
  "id": "my-track",
  "file": "my-track.m4a",
  "title": "My Track",
  "moods": ["calm", "hopeful"],
  "energy": 0.3,
  "bpm": 92,
  "beat_offset_ms": 120,
  "duration_ms": 95000,
  "loopable": false,
  "license": "Artist, CC BY 4.0, https://...",
  "how_made": "bought from ..."
}
```

- `id` (unique) and `file` (a name inside this folder), `moods` (at least one; the first is the
  main one, offered to Claude when it writes a storyboard) and `energy` (0 quiet and slow .. 1
  intense) are required.
- `bpm` and `beat_offset_ms` (the time of the first beat) make beat sync exact; leave `bpm` out
  for music without a steady beat (beat sync then detects the beats, or skips with a warning).
- `loopable`: whether the end runs into the start seamlessly. The render loops music under longer
  videos either way; `vimmer check` warns when a track that is not loopable is shorter than the
  video.
- `duration_ms`, `title`, `license`, `how_made`: for people and for `vimmer check`.
- `credit` (`title`, `artist`, `page`, `license`): where a downloaded track comes from.
- `placeholder`: `true` for a stand-in that should play only when no real track fits.

`vimmer music list` shows the result; a track whose file is missing is listed as missing and
never picked. To remove the placeholders, delete their entries (and files), after checking that
no timeline names them as a `source`.

Run `scripts/make-music-library.sh` to rebuild the placeholders (about 6 minutes on a CPU; the
output is the same every time).
