# Music library

Background music chosen by mood: a timeline says `"music": {"pick": {"mood": "calm"}}` and the
render picks a track from this folder (see the main README, "Music library"). `library.json`
lists the tracks; `vimmer music list` prints them.

## The bundled tracks are placeholders

All six were synthesized with ffmpeg alone by
[`scripts/make-music-library.sh`](../../../../scripts/make-music-library.sh): no samples, nothing
downloaded. They are public domain / CC0 by this project. They are simple (sine-based
instruments, a few echoes for room) and meant to be replaced by real music.

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

`vimmer music list` shows the result; a track whose file is missing is listed as missing and
never picked. To remove the placeholders, delete their entries (and files).

Run `scripts/make-music-library.sh` to rebuild the placeholders (about 6 minutes on a CPU; the
output is the same every time).
