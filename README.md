# Vimmer — text-to-video generation

Self-hosted, modular pipeline that turns a JSON scene description into a short
(≤ 200 s) 1080p / 30 fps video.

```
repo_vimmer/
├── fixtures/                 # Shared JSON contract fixtures (Rust tests + Remotion props)
│   ├── sample-timeline.json  # 15-second single-scene baseline
│   └── motion-demo.json      # 3 scenes: a slide transition and keyframes
├── rust-core/                # Cargo workspace: orchestration + DSL
│   └── crates/vimmer-dsl/    # serde model, validation, frame math
└── remotion-renderer/        # Node/TS Remotion project: frame-based composition
    ├── public/assets/        # Images/audio referenced by the timeline (relative paths)
    └── src/                  # Root, Timeline composition, Ken Burns + text components
```

## Contract

`fixtures/sample-timeline.json` is the single source of truth for the DSL. The
Rust `VideoTimeline` / `Scene` structs and the TypeScript types in
`remotion-renderer/src/types/timeline.ts` must both match it.

- `fps` must be `30`; `resolution` must be `1920x1080` or `1080x1920`.
- Total duration is capped at 200 000 ms (6 000 frames).
- Each scene needs `text` or `image`; `motion` is one of `static`, `zoom_in`,
  `zoom_out`, `pan_left`, `pan_right`.
- Asset paths are relative to `remotion-renderer/public/`.

### Transitions (`transition_in`)

Optional per scene: `{ "kind": ..., "duration_ms": ... }`, where `kind` is `fade`,
`slide_left`, `slide_right`, `slide_up`, `slide_down`, `zoom_in` or `wipe_left`.

Overlap rule: the transition overlaps the **start of the incoming scene**. The scene
still starts at its nominal time, and the previous scene keeps its nominal end, so the
total duration and every scene's start are unchanged. For the first `duration_ms` of
the incoming scene, the previous scene's last frame is held underneath and the incoming
scene plays over it (fades in, slides in, zooms in, or is revealed by a wipe). The
transition does not change any timing.

- A transition on the first scene, a zero-length transition, and one longer than its
  scene are all rejected.
- The chunk planner never splits a scene that has a transition from the one before it,
  so the held frame is always rendered in the same chunk.

### Keyframes (`keyframes`)

Optional per scene: a list of `{ at_ms, scale?, x?, y?, opacity?, rotation?, ease? }`
that animates the image layer. Defaults: `scale` 1, `x` and `y` 0 (percent of the frame),
`opacity` 1, `rotation` 0 (degrees, clockwise), `ease` `ease_in_out`.

- Keyframes must be sorted by `at_ms`, with no duplicates, and each `at_ms` must be within
  the scene (`0..=duration_ms`). `scale` must be above 0, and `opacity` must be between 0 and 1.
- A non-empty list replaces `motion`.
- Before the first keyframe, its values hold; after the last, its values hold.
- Between two keyframes, values interpolate with the earlier keyframe's `ease`:
  `linear`, `ease_in` (t²), `ease_out` (1−(1−t)²) or `ease_in_out` (quadratic).
- `opacity` applies to the whole image layer, so fading it to 0 shows the background.

`fixtures/motion-demo.json` shows both: three scenes, one slide transition, and
keyframes. Its scene 2 is a transition, and scenes 1 and 2 share a chunk with
`--chunk-ms 3000`.

## Commands

```bash
# Rust: DSL tests
cd rust-core && cargo test

# Remotion: motion unit tests (keyframe sampling, transition poses; Node >= 22.6)
cd remotion-renderer && npm run test:motion

# Remotion: typecheck, studio, and render the 15 s test scene
cd remotion-renderer
npm install
npm run typecheck
npm run studio
npm run render:test      # writes out/test-scene.mp4
```

## Rust crates

- `vimmer-dsl`: schema, validation, frame math.
- `vimmer-media`: image normalization (blurred-background letterboxing) and audio
  duration probing via `ffmpeg`.
- `vimmer-scheduler`: chunk planner (whole scenes, ≤ 30 s per chunk) and a tokio
  pipeline: preprocess images → check audio → render chunks through Remotion with
  bounded concurrency → concatenate with ffmpeg `-c copy`.
- `vimmer-cli`: `vimmer <timeline.json> -o out.mp4`.

```bash
cd rust-core && cargo run -p vimmer-cli -- ../fixtures/sample-timeline.json \
  -o ../out/sample.mp4 --remotion-dir ../remotion-renderer
```

Subtitles: the timeline's `subtitles` array holds cues in absolute milliseconds, or
pass `--subtitles file.srt` to import SubRip. Each chunk clips and rebases cues, so
a cue that crosses a chunk boundary shows in both chunks. The overlay sits in the
lower band; scene text keeps a 260 px bottom inset to stay clear of it.

Image and audio paths are relative to `remotion-renderer/public/`. Normalized
images are written to `public/generated/`, which is gitignored.

## Desktop app (Tauri 2)

`desktop/` wraps the Studio editor in a native macOS window (Tauri 2, Rust). It is a
standalone Cargo project, not part of the `rust-core` workspace.

```bash
cd desktop && npm install
npm run dev      # starts the editor dev server on :5180 and opens the window
npm run build    # production bundle (.app and .dmg under src-tauri/target/release/bundle)
```

Add `--debug` to `npm run build` for a faster, unoptimized bundle.
The window loads the editor's `dist/` build; `index.html` keeps its font stylesheet
non-blocking so the app still renders offline.

**Render** (toolbar) renders the timeline to an MP4 chosen in a save dialog. A card in the
corner shows the stage and a progress bar, with Cancel while it runs and Show in Finder
when it finishes. Rendering runs the same pipeline as the CLI, in the app's cache folder.
Cancelling kills the Remotion and ffmpeg processes. Only one render runs at a time. The
Remotion project is found next to the repository, so the app must run from the source
tree for now.

Projects are plain timeline JSON files on disk: ⌘O opens one, ⌘S saves to its path
(or asks for one), ⇧⌘S saves as a new file, ⌘N starts a new project. Writes are atomic
(temp file, then rename). Unsaved work is mirrored to localStorage as a draft and comes
back on the next launch, marked unsaved. Closing the window with unsaved changes asks first.

## Background music

Music is one track under the whole video. In the editor, open the **Music** tab in the
Inspector and choose a file (or type a path under `public/`). Set the volume (0–1) and
optional fades in and out, in milliseconds. The track loops until the video ends.

The renderer does not render music inside the segments. After the segments are joined, ffmpeg
loops the track, applies volume and fades, and mixes it under any scene audio (narration).
Music that runs past the end is cut. The editor preview plays the same loop and fades.

## Images and audio

In the Inspector, the Image and Audio source fields have a **Choose** button (desktop app).
It opens a file dialog and copies the file into `remotion-renderer/public/assets/` (a
second file with the same name gets a `-2` suffix; an identical file is reused). The
timeline stores `assets/<name>`. The preview reads the public folder from disk, so new files
show without a rebuild. Typing a path still works in both the browser and the desktop app.

## Narration (local TTS)

A scene with **Narrate the text** checked (Inspector, Audio) gets its speech generated
when you render: its `text` is synthesized with macOS `say` (Samantha for English, Tingting
for Chinese, Kyoko for Japanese; or pick a voice) and converted to WAV. The scene is stretched to fit the speech,
plus 300 ms. Files are cached in `remotion-renderer/public/generated/tts/`, keyed by voice and
text. A scene that already has its own audio file keeps it. The preview stays silent until
you render. Local narration needs macOS.

## Word-by-word subtitles (local speech recognition)

After narration is synthesized, each clip is recognized again with
[whisper.cpp](https://github.com/ggml-org/whisper.cpp) (`whisper-cli`, run as a child process,
fully offline) and the result is aligned back to the scene text. Subtitles always show the
script, never the recognizer's spelling; recognition only supplies the timing. Chinese,
English and Japanese are supported (any language whisper knows works).

```bash
brew install whisper-cpp
mkdir -p ~/.local/share/vimmer/models
curl -L -o ~/.local/share/vimmer/models/ggml-large-v3-turbo-q5_0.bin \
  https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-large-v3-turbo-q5_0.bin
cd rust-core && cargo run -p vimmer-cli -- ../fixtures/word-subtitles-demo.json -o /tmp/words.mp4 --remotion-dir ../remotion-renderer
```

- **Two switches.**
  - `--word-subtitles on|off` (default **on**): add word timing to subtitle cues that cover a
    synthesized narration. Scenes without narration, and cues that already have `words`, are
    left alone.
  - `--auto-subtitles on|off` (default **off**): also create cues from the narration text for
    narrated scenes that no cue covers. Off by default so an existing timeline renders the same
    subtitles it always did; when nothing needs timing, whisper is not even looked up.
- When `whisper-cli` or the model is missing, the render prints one warning and the subtitles
  stay exactly as they were. Recognition stops when the render is cancelled, and gives up after
  180 s plus 30× the audio length (then plain subtitles are kept and later clips are skipped).
- **Engine**: `whisper-cli` from PATH, or `--whisper-cli` / `$VIMMER_WHISPER_CLI`. Model:
  `--whisper-model` / `$VIMMER_WHISPER_MODEL`, default
  `~/.local/share/vimmer/models/ggml-large-v3-turbo-q5_0.bin`. Language: `--whisper-language`
  (`zh`, `en`, `ja`, … or `auto`); by default it is guessed from each scene's text (kana → ja,
  other ideographs → zh, Latin → en). The scene text is also passed as whisper's prompt.
- **Timing**: `-ojf` token timestamps. `VIMMER_WHISPER_DTW=1` adds DTW token timing (with
  flash attention off); it is off by default because on an Intel Mac it nearly doubled
  recognition time with no visible gain on TTS speech. On Intel Macs whisper runs CPU-only
  (`-ng`): the Metal backend returned garbage there (`VIMMER_WHISPER_GPU=1/0` overrides).
  Every whisper call encodes a full 30 s window, so clips of the same language are joined
  (1 s of silence between them, at most 28 s per call), recognized once, and split back by
  offset. Results are cached next to the narration WAV in `public/generated/tts/`. Expect roughly
  40 s per clip with large-v3-turbo on a 4-core Intel CPU; a smaller model
  (`--whisper-model …/ggml-small.bin`) is much faster.
- **Alignment**: the script is cut into units (an English word with its punctuation; one
  Chinese character; one Japanese mora, so `きょ` stays together). Recognized words are cut the
  same way and the two sequences are aligned by edit distance, where pairing two different
  words costs their character distance. Misheard words keep their slot, missed words are
  interpolated between neighbours by length, extra words are ignored.
- **Cues**: subtitle cues overlapping a narrated clip get `words` for their own text. With
  `--auto-subtitles on`, a clip no cue covers gets cues generated from the script: a sentence
  end closes a line, and a long line breaks at a comma or before the word that overflows.

Each cue can carry absolute word times, which must lie inside the cue, in order and without
overlap (zero-length words are allowed). Empty `words` is omitted from the JSON, and cues
without words render exactly as before:

```json
{ "start_ms": 1000, "end_ms": 2600, "text": "你好世界。",
  "words": [ { "text": "你", "start_ms": 1000, "end_ms": 1200 }, { "text": "好", "start_ms": 1200, "end_ms": 1500 },
             { "text": "世", "start_ms": 1500, "end_ms": 1800 }, { "text": "界。", "start_ms": 1800, "end_ms": 2100 } ] }
```

The renderer shows the spoken word in yellow, words already said in white and words to come
dimmed. Chunked renders clip words with their cue: words outside a chunk become zero-length at
its edge, so a line split across two chunks reads the same in both.

## Studio (editor)

A browser editor in `remotion-renderer/editor/`. The preview is the same `Timeline`
composition the renderer uses, running in `@remotion/player`.

```bash
cd remotion-renderer && npm install && npm run editor   # http://localhost:5180
```

- **Preview**: Remotion Player with frame sync. Space plays and pauses.
- **Timeline**: scene, subtitle and audio tracks. Click the ruler to scrub, drag a
  scene's right edge to change its duration, and zoom with the slider. Edges snap to
  neighbouring boundaries and show a cyan guide.
- **Inspector**: edit scene text, duration, highlights, image, motion and audio, and
  edit subtitle cues. Add a cue at the playhead.
- **Toolbar**: format switch, undo and redo (⌘Z / ⇧⌘Z), import and export JSON.
  Validation mirrors the Rust rules and appears above the workspace.
- The draft autosaves to localStorage. Export JSON, then render it with the CLI.

## Not yet built

- TTS generation (duration tracking exists for existing audio files).
- Studio: no direct render from the editor yet (export JSON, then run the CLI).
  Subtitle cues are not drag-movable on the timeline; edit them in the inspector.
