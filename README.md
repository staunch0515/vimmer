# Vimmer — text-to-video generation

Self-hosted, modular pipeline that turns a JSON scene description into a short
(≤ 200 s) 1080p / 30 fps video.

```
repo_vimmer/
├── fixtures/                 # Shared JSON contract fixtures (Rust tests + Remotion props)
│   ├── sample-timeline.json  # 15-second single-scene baseline
│   └── motion-demo.json      # 3 scenes: a slide transition and keyframes
├── rust-core/                # Cargo workspace: orchestration + DSL
│   ├── crates/vimmer-dsl/    # serde model, validation, frame math
│   └── crates/vimmer-storyboard/  # storyboard JSON → timeline; `vimmer generate` (Claude)
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

# Rust: storyboard conversion tests
cd rust-core && cargo test -p vimmer-storyboard

# Rust: word alignment tests (align.rs)
cd rust-core && cargo test -p vimmer-media --lib align

# Remotion: motion unit tests (keyframe sampling, transition poses; Node >= 22.6)
cd remotion-renderer && npm run test:motion

# Remotion: word-subtitle tests (which word is current; Node >= 22.6)
cd remotion-renderer && npm run test:words

# Remotion: typecheck, studio, and render the 15 s test scene
cd remotion-renderer
npm install
npm run typecheck
npm run studio
npm run render:test      # writes out/test-scene.mp4
```

## Rust crates

- `vimmer-dsl`: schema, validation, frame math.
- `vimmer-media`: image normalization (blurred-background letterboxing), audio
  duration probing via `ffmpeg`, narration (`say`, or Kokoro through
  `scripts/kokoro_tts.py`), speech recognition, and the SVG sanitizer / Chrome rasterizer
  for illustrations (`svg.rs`).
- `vimmer-scheduler`: chunk planner (whole scenes, ≤ 30 s per chunk) and a tokio
  pipeline: preprocess images → check audio → render silent chunks through Remotion with
  bounded concurrency → concatenate with ffmpeg `-c copy` → mix the whole soundtrack once
  (`audio.rs`, ducking in `duck.rs`).
- `vimmer-cli`: `vimmer <timeline.json> -o out.mp4` (`--tts kokoro` for Kokoro narration), and
  `vimmer generate <input.txt> --lang zh -o out.mp4` (text → Claude storyboard → MP4, with
  `--illustrate` for Claude-drawn pictures), and `vimmer illustrate <storyboard.json>`.
- `vimmer-storyboard`: converter from a storyboard JSON (the format an LLM writes, see
  below) to a validated timeline JSON (offline, bin `vimmer-storyboard`), and the Claude
  generation behind `vimmer generate` (Python helpers `scripts/generate_storyboard.py` and
  `scripts/illustrate.py`).

```bash
cd rust-core && cargo run -p vimmer-cli -- ../fixtures/sample-timeline.json \
  -o ../out/sample.mp4 --remotion-dir ../remotion-renderer
```

Subtitles: the timeline's `subtitles` array holds cues in absolute milliseconds, or
pass `--subtitles file.srt` to import SubRip. Each chunk also gets the overlapping cues in
full-timeline time, so a cue that crosses a chunk boundary fades only at its own ends, as in
an unchunked render (no blink at the seam). The overlay sits in the
lower band; scene text keeps a 260 px bottom inset to stay clear of it.

Image and audio paths are relative to `remotion-renderer/public/`. Normalized
images are written to `public/generated/`, which is gitignored.

## Storyboard (storyboard JSON → timeline)

`vimmer-storyboard` turns a storyboard into the timeline JSON that `vimmer` renders. It is
the format a language model is asked to write (the prompt is `rust-core/crates/vimmer-storyboard/prompts/storyboard.md`),
so the model never writes timeline fields directly. Run from `rust-core/`:

```bash
cargo run -p vimmer-storyboard -- crates/vimmer-storyboard/examples/zh-xian.storyboard.json -o ../out/zh.timeline.json
cargo run -p vimmer-storyboard -- crates/vimmer-storyboard/examples/en-coffee.storyboard.json -o ../out/en.timeline.json \
  --public-dir ../remotion-renderer/public --emphasis-out ../out/en.emphasis.json
```

- `-o` / `--output`: the timeline JSON to write (required).
- `--public-dir <dir>`: the renderer's `public/` folder. When given, every scene `image` and
  `music` must exist there, or the conversion stops and names the scene and field.
- `--emphasis-out <file>`: also write the emphasis map (scene id → words), which the timeline
  does not carry.
- Warnings (unknown camera, a transition on the first scene, …) go to stderr; the output is
  still written. A summary line (`N scenes, M ms -> …`) is printed at the end.
- Output is checked with the DSL validator, so a storyboard that converts always renders.

### Global fields

| Field | Required | Meaning |
| --- | --- | --- |
| `title` | yes | Non-empty. |
| `language` | yes | `zh`, `en` or `ja`. Chooses the duration estimate. No voice is written, so `say` and Kokoro each pick their default for the language of the text; only `ja` text without kana (which would read as Chinese) gets the voice `Kyoko`. |
| `orientation` | no (`landscape`) | `landscape` = 1920×1080, `portrait` = 1080×1920. |
| `narration` | no (`true`) | When true, each scene's `text` is read aloud at render time. |
| `voice` | no | Overrides the speech voice (macOS `say`). Default: chosen by language. |
| `music` | no | Background track, relative to `remotion-renderer/public/`. Fades: in = min(500 ms, total/4), out = min(1500 ms, total/4). |
| `scenes` | yes | At least one scene. |

### Scene fields

| Field | Required | Meaning |
| --- | --- | --- |
| `text` | yes | Narration and on-screen text. Trimmed; must not be empty. |
| `duration_ms` | no | Integer > 0, at most 200 000. Omit to estimate from the text. |
| `emphasis` | no | Words copied from `text`. Those found in `text` become `highlights`; others are warned about and not highlighted. |
| `camera` | no (`static`) | Camera move, see the mapping below. |
| `transition` | no | Transition into this scene, see the mapping below. Ignored on the first scene. |
| `transition_ms` | no (`500`) | Transition length. Clamped to the scene length. Given without `transition`, it is ignored. |
| `image` | no | Relative path under `public/`, e.g. `assets/sample.png`. No `..`, no URL, no leading `/`. No `image` means text only. |
| `image_prompt` | no | Description of a wanted picture. Drawn by Claude with `--illustrate` / `vimmer illustrate` (see "Illustrations"); otherwise stored only. |

Unknown field names are rejected (`deny_unknown_fields`). Unknown `camera` or `transition`
values do not fail: they fall back to `static` or `none` with a warning. Values are trimmed,
lower-cased, and `-` or space is read as `_`, so `Slide-Left` means `slide_left`.

### Camera and transition mapping

| Storyboard `camera` | Timeline |
| --- | --- |
| `static` (default) | `motion: "static"` |
| `push_in`, `zoom_in` | `motion: "zoom_in"` |
| `pull_out`, `zoom_out` | `motion: "zoom_out"` |
| `pan_left` | `motion: "pan_left"` |
| `pan_right` | `motion: "pan_right"` |
| `ken_burns` | two keyframes over the whole scene: scale 1 → 1.15, x 0 → −3 %, y 0 → −2 %, `ease_in_out` |

| Storyboard `transition` | Timeline `transition_in` |
| --- | --- |
| `none` (default), `cut` | none |
| `fade`, `slide_left`, `slide_right`, `slide_up`, `slide_down`, `zoom_in`, `wipe_left` | the same `kind`, `duration_ms` = `transition_ms` (default 500, clamped to the scene) |

The first scene's transition is dropped with a warning.

### Duration estimate (when `duration_ms` is omitted)

- `zh`: 230 ms per CJK character. `ja`: 200 ms per character. Punctuation and whitespace are free.
  A run of ASCII letters or digits counts as 2 characters.
- `en`: 400 ms per word (whitespace-separated tokens with a letter or digit).
- Then 600 ms of padding is added, and the result is at least 2000 ms.

The total length must stay within the 200 s limit, or the conversion fails.

## Generate a video from text (Claude)

`vimmer generate` sends a text to Claude, gets a storyboard back, checks it with the storyboard
converter and renders it. This is the one part of Vimmer that calls a paid cloud API: every run
costs money (see "Cost" below). Without a key it stops with an error; there is no offline
fallback.

```
input.txt ──► scripts/generate_storyboard.py (Claude, structured output)
          ──► storyboard JSON ──► convert() ──ok──► timeline ──► render ──► MP4
                                      │ error
                                      └──► sent back to Claude for a fix (at most 2 times)
```

### Install

```bash
sh rust-core/crates/vimmer-storyboard/scripts/install_llm.sh
```

This makes a venv at `~/.local/share/vimmer/llm-venv` with the official `anthropic` SDK (uses
`uv` when it is on `PATH`). Use another interpreter with `--llm-python` or `VIMMER_LLM_PYTHON`.

### API key

The key is read **only** from the environment variable `ANTHROPIC_API_KEY`. Add it to
`~/.zshrc` and open a new terminal:

```bash
export ANTHROPIC_API_KEY='sk-ant-...'
```

Vimmer never prints the key, passes it on the command line, or writes it to a file; the
Python helper inherits it from the environment. Keep it out of the repository.

### Usage

Run from `rust-core/` (or use the built `vimmer` binary anywhere; paths below are examples):

```bash
# Text → MP4, with Kokoro narration and word-by-word subtitles
cargo run --release -p vimmer-cli -- generate crates/vimmer-storyboard/examples/zh-xian.source.txt \
  --lang zh -o ../out/xian.mp4 --remotion-dir ../remotion-renderer \
  --tts kokoro --auto-subtitles on --save-dir ../out/xian

# Only the storyboard (no render); printed to stdout, or saved with --save-dir
cargo run --release -p vimmer-cli -- generate story.txt --lang en --no-render > storyboard.json
```

- `--lang zh|en|ja` (required), `--orientation landscape|portrait`, `--target-seconds N`.
- `--model` (default `claude-opus-5-5`), `--effort low|medium|high|xhigh|max` (default
  `medium`), `--max-tokens` (default 32000, thinking included), `--max-retries` (default 2),
  `--llm-timeout` seconds per API call (default 900).
- `--save-dir DIR` keeps `storyboard.json`, `timeline.json`, `emphasis.json`, `usage.json`
  (tokens, cost, retries per attempt) and `reply-N.json` (the raw API replies).
- Every render option of `vimmer <timeline.json>` works here too (`--tts`, `--auto-subtitles`,
  `--word-subtitles`, `--remotion-dir`, `--work-dir`, …). Generated timelines carry no subtitle
  cues, so pass `--auto-subtitles on` to get subtitles.
- Pictures are not generated unless you pass `--illustrate` (see "Illustrations" below):
  without it each scene's `image_prompt` is kept in the storyboard, `image` stays empty, and
  scenes render as text.
- `vimmer <timeline.json> -o out.mp4` works as before. (`vimmer generate` is recognized only as
  the first argument; render a timeline file that is literally named `generate` as
  `./generate`.)
- Ctrl-C stops a running API call. Exit code is non-zero on any failure: no key, missing SDK,
  API error (bad key, rate limit, …), refusal, `max_tokens`, timeout, or a storyboard that is
  still invalid after the retries. The tokens already spent are printed either way.

How it works: the helper fills `prompts/storyboard.md` (no assets offered) as the system prompt
and asks for JSON with `output_config.format` using `schema/storyboard.schema.json` — the fields
of `model.rs` minus `image`, `music` and `voice`, with `language` / `orientation` pinned to the
request. It uses adaptive thinking, streams the reply, and on Opus / Sonnet 5.5 opts into the
server-side refusal fallback (`fallbacks: "default"`); Claude Haiku 5.5 has none. Rust then runs
`convert()` on the reply; an error (with scene number and field) goes back to Claude as the next
turn, at most `--max-retries` times. Usage of every attempt, including fallback hops, is added up.

Run the helper alone (prints one JSON result line):

```bash
~/.local/share/vimmer/llm-venv/bin/python rust-core/crates/vimmer-storyboard/scripts/generate_storyboard.py \
  --text-file story.txt --lang ja --effort low
```

### Cost

Measured on 2026-11-13 with the three `examples/*.source.txt` texts (about 250 characters / 165
words each; effort `medium`; every reply passed validation on the first try):

| Model | Input tokens | Output tokens | Cost per storyboard |
|---|---|---|---|
| `claude-opus-5-5` (default) | ~2,750 | 930–1,190 | $0.029–0.035 |
| `claude-sonnet-5-5` | ~2,750 | 630–1,000 | $0.012–0.016 |
| `claude-haiku-5-5` | ~2,750 | 960–1,810 | $0.0008–0.0012 |

About 2,600 of the input tokens are the fixed prompt and schema, so cost grows slowly with the
text. A longer text, higher `--effort`, or a retry raise it; budget roughly $0.05–0.10 per Opus
video in the worst case.

Prices used (USD per million tokens, input / output): Opus 5.5 $4 / $20, Sonnet 5.5 $2 / $10,
Haiku 5.5 $0.10 / $0.50. The figure Vimmer prints is an estimate from these prices; the
Anthropic Console has the billed amount. A validation retry resends the conversation, so it
costs roughly one more call.

### Illustrations (`--illustrate`, `vimmer illustrate`)

With `--illustrate`, Claude also draws one flat vector illustration (SVG) per scene, and the
scene renders on that picture instead of a plain background. It is **off by default** because
it costs far more than the storyboard (about $0.10 per scene with Opus, see the table below).
Before it starts, Vimmer prints an estimate; at the end, the actual tokens and cost.

```bash
# Text → storyboard → illustrations → MP4
cargo run --release -p vimmer-cli -- generate crates/vimmer-storyboard/examples/zh-xian.source.txt \
  --lang zh -o ../out/xian.mp4 --remotion-dir ../remotion-renderer \
  --tts kokoro --auto-subtitles on --illustrate --save-dir ../out/xian

# Illustrate a storyboard you already have (no new storyboard call); optionally render it
cargo run --release -p vimmer-cli -- illustrate ../out/xian/storyboard.json \
  --remotion-dir ../remotion-renderer -o ../out/xian-illustrated.mp4 --tts kokoro --auto-subtitles on
```

- `--illustrate-model` (default `claude-opus-5-5`; `claude-sonnet-5-5` is about half the
  price), `--illustrate-effort` (default `medium`; `low` is cheaper but visibly rougher),
  `--illustrate-jobs` (pictures drawn at once, default 3), `--chrome PATH`.
- `vimmer illustrate STORYBOARD` writes `timeline.illustrated.json` next to the storyboard
  (`--timeline-out` to change it), or adds the pictures to an existing timeline with
  `--timeline FILE` (same scene count and order). `-o out.mp4` renders it too; every render
  option works. Scenes that already have an `image` keep it.
- One picture failing only affects its scene (it stays text only, with a warning). When the
  API cannot be used at all (no key, rejected key, no connection, no SDK) or Chrome cannot
  rasterize, Vimmer warns and makes the video without pictures; no money is spent once that
  is known.

How it works:

1. **One style for the whole video.** A first, cheap call (structured output) reads the title
   and every scene's `image_prompt` and returns a style guide: 5–7 hex colours, background,
   line and shape rules, composition, mood, recurring motifs. Every scene request carries that
   guide, so the pictures share palette, line work and layout.
2. **One SVG per scene**, up to 3 at once. The prompt fixes the canvas (`viewBox` 1920x1080 or
   1080x1920 like the timeline), keeps the main subject in the upper ~55 % and the lower ~45 %
   calm and dark where the scene text and subtitles go, keeps a 6 % margin for the camera
   motion, and forbids text in the picture.
3. **Sanitize** (`vimmer-media/src/svg.rs`). The SVG is untrusted. A strict XML parser (no
   DOCTYPE/entities, 512 KB, 8000 elements, depth 48) rebuilds the document from an allow-list
   of drawing elements: `script`, `foreignObject`, `image`, `a`, `iframe`, animation elements and
   anything unknown are dropped with their content; `on*` attributes, `href`/`xlink:href` that
   are not `#id`, `xml:base`, foreign namespaces, `javascript:`/`data:` values and every
   `url(...)` not pointing at `#id` are removed; CSS loses `@import` and external `url()`, and
   CSS with escapes or `image-set()` is dropped. A `viewBox` with the canvas aspect ratio is
   required. A rejected SVG goes back to Claude once with the reason; if the second one fails
   too, the scene stays text only.
4. **Rasterize** with the headless Chrome Remotion already downloads
   (`remotion-renderer/node_modules/.remotion/`, fetched with `remotion browser ensure` when
   missing), network sent to a closed proxy port, into a PNG of the frame size; a PNG that is
   one flat colour counts as a failed picture. The PNG then takes the normal image path
   (normalization, Ken Burns motion). Scenes on a picture get a soft dark gradient under the
   text; text-only scenes look exactly as before.
5. **Cache**: `remotion-renderer/public/generated/illustrations/` (git-ignored) holds
   `<hash>.svg` and `<hash>.png`, the hash covering model, effort, a cache version, the style
   guide, the picture prompt and the canvas size; the style guide is cached as
   `style-<hash>.json`. Running again with the same storyboard makes no API call (it works
   even without a key). If rasterizing fails, the sanitized SVG is kept, so the next run only
   rasterizes.

`--save-dir` also keeps `illustrations.json` (style guide, per-scene tokens, cost, SVG size,
what the sanitizer removed, warnings) and the raw replies `illustration-*.json`.

Measured on 2026-11-13 (effort `medium`, landscape, every SVG accepted on the first try,
nothing removed by the sanitizer):

| Run | Scenes | Output tokens / picture | Cost / picture | SVG size | Style call | Total |
|---|---|---|---|---|---|---|
| zh-xian, `claude-opus-5-5` | 11 | 2,365–5,192 (avg 3,855) | $0.084 | 4.1–7.1 KB | $0.027 | $0.95 |
| ja-sakura, `claude-opus-5-5` | 10 | 1,988–3,846 (avg 2,679) | $0.061 | 3.7–7.2 KB | $0.027 | $0.63 |
| zh-xian, `claude-sonnet-5-5` | 11 | 1,396–1,901 (avg 1,662) | $0.020 | 2.5–3.4 KB | $0.013 | $0.23 |

Input is about 1,700 tokens per picture (prompt, style guide, scene). Opus draws richer
scenes (more buildings, lanterns, light, depth); Sonnet's are simpler and flatter but in the
same consistent style, at about a quarter of the cost. A correction call roughly doubles the
cost of that picture. The printed estimate assumes ~3,500 output tokens per picture at
`medium`.

Tests (no network): `cd rust-core && cargo test -p vimmer-storyboard --lib` (a fake helper), and
`~/.local/share/vimmer/llm-venv/bin/python -m unittest discover -s rust-core/crates/vimmer-storyboard/scripts/tests`
(recorded API replies).

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

Music that runs past the end is cut. The editor preview plays the same loop and fades (but
not the ducking below).

## Audio (soundtrack, sync and ducking)

Segments are rendered **without audio** (`remotion render --muted`). After they are joined,
ffmpeg builds the whole soundtrack once, from the timeline, and muxes it onto the video:

- Each scene's audio (its `audio` file or its generated narration) starts at the scene's
  start frame plus `offset_ms` rounded to a frame, and is cut at the scene's last frame:
  the same placement Remotion uses. Mono clips play on both channels at full level.
- Music loops from 0, with its volume and fades, ducked as below, and is mixed under the
  scene audio without normalization. The track is exactly as long as the video.
- One AAC encode (48 kHz, 320 kb/s) for the whole video.

Before this, every segment carried its own AAC track. Each one starts with about 43 ms of
encoder priming silence, and the `-c copy` join kept it, so the sound fell another ~43 ms
behind the picture at every seam (a 3-minute video ended ~250 ms out of lip sync with 30 s
segments). Now a chunked render sounds identical to a one-segment render.

**Ducking.** Background music automatically drops under every scene's audio (narration or
`audio`; the scheduler cannot tell speech from a sound effect, so all scene audio counts) and
comes back up between them. The envelope comes from the known clip times, not from the signal,
so the same timeline always sounds the same:

- fully down (`depth_db`, default −12 dB relative to the music volume) when the clip starts:
  the attack (default 200 ms) runs *before* the onset, so the first syllable is never masked;
- held while the clip plays (until the clip's real end, or its scene's end if it is cut);
- back up over the release (default 400 ms). Gaps shorter than attack + release stay down,
  so the music does not bob up between two lines. Ramps are linear in dB.

It is on by default. Tune or turn it off per timeline:

```json
"music": {
  "source": "assets/pad-loop.m4a", "volume": 0.3, "fade_in_ms": 500, "fade_out_ms": 1500,
  "duck": { "enabled": true, "depth_db": -12, "attack_ms": 200, "release_ms": 400 }
}
```

Every `duck` field is optional; `"duck": { "enabled": false }` mixes the music at a constant
level as before. Ranges: `depth_db` −40..0, `attack_ms` and `release_ms` 10..5000. Music with
no scene audio under it sounds exactly as before. `fixtures/music-ducking.json` demonstrates it
with `assets/pad-loop.m4a`, a short synthesized chord loop made for the fixture.

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

### Kokoro narration (optional, local)

[Kokoro-82M](https://huggingface.co/hexgrad/Kokoro-82M) is a small neural TTS that sounds much
more natural than `say`. Vimmer runs its ONNX build ([kokoro-onnx](https://github.com/thewh1teagle/kokoro-onnx))
locally through a Python helper; nothing is sent anywhere and nothing costs money. `say` stays
the default.

**Install** (once; about 730 MB of models, Python 3.10–3.12; works on Intel Macs, where the
PyTorch build of Kokoro has no wheels):

```bash
sh rust-core/crates/vimmer-media/scripts/install_kokoro.sh
```

It creates the venv `~/.local/share/vimmer/kokoro-venv` from
`rust-core/crates/vimmer-media/scripts/requirements.txt` (kokoro-onnx, onnxruntime, misaki[zh,ja],
unidic-lite, soundfile) and downloads into `~/.local/share/vimmer/models/kokoro/`:
`kokoro-v1.0.onnx` + `voices-v1.0.bin` (release `model-files-v1.0`), `kokoro-v1.1-zh.onnx` +
`voices-v1.1-zh.bin` (release `model-files-v1.1`) and `config-v1.1-zh.json` (from
`hexgrad/Kokoro-82M-v1.1-zh`).

**Switches** (a flag wins over the variable):

| What | Flag | Variable | Default |
|---|---|---|---|
| Engine | `--tts say\|kokoro` | `VIMMER_TTS` | `say` |
| Interpreter | `--kokoro-python` | `VIMMER_KOKORO_PYTHON` | `~/.local/share/vimmer/kokoro-venv/bin/python` |
| Models | `--kokoro-models` | `VIMMER_KOKORO_MODELS` | `~/.local/share/vimmer/models/kokoro` |

```bash
cd rust-core && cargo run -p vimmer-cli -- ../fixtures/word-subtitles-demo.json \
  -o /tmp/kokoro.mp4 --remotion-dir ../remotion-renderer --tts kokoro
```

The desktop app has no switch; it reads the same variables (`VIMMER_TTS=kokoro`), as long as
they reach the app's environment (e.g. `launchctl setenv VIMMER_TTS kokoro`, then restart it).

**Languages and voices.** The language comes from the text, by the same rule as recognition:
any kana → Japanese, other CJK ideographs → Chinese, anything else → English.

| Language | Model | G2P | Default voice | Others |
|---|---|---|---|---|
| zh | v1.1-zh | misaki `ZHG2P(version="1.1")` | `zf_001` | `zf_002`…, `zm_009`… (100 voices) |
| ja | v1.0 | misaki `JAG2P` | `jf_alpha` | `jf_gongitsune`, `jf_nezumi`, `jf_tebukuro`, `jm_kumo` |
| en | v1.0 | espeak-ng (bundled) | `af_heart` | `af_bella`, `am_michael`, `bf_emma`, `bm_george`, … |

Chinese uses the v1.1-zh model on purpose: v1.0's Chinese G2P runs syllables together
(「已有」 comes out as 「你有」). A scene's `voice` is used when it is a Kokoro voice for that
language (`zf_…`/`zm_…` for Chinese, `j…` for Japanese, `a…`/`b…` for English). A `say` voice
such as `Tingting`, or a Kokoro voice of another language, is ignored with a warning and the
default is used, so timelines written for `say` render with Kokoro unchanged. In `say` mode a
Kokoro voice name is likewise replaced by the `say` default with a warning.

**English inside Chinese** (「我们用 Python 制作视频」) is read in English: the English words go
through misaki's espeak fallback, which uses the espeak-ng library kokoro-onnx already ships, so
no spaCy model or other download is needed. It is an espeak pronunciation, plainer than the
English model's own G2P. Latin words inside Japanese text are left to misaki's Japanese G2P.

**Cache.** Kokoro files are named `<scene>-kokoro-<hash>.wav` in `public/generated/tts/`; the
hash covers the engine, the model files (name and size), voice, language, speed and text.
`say` files keep their old `<scene>-<hash>.wav` names and keys, so switching engines never
mixes the two and old caches still hit.

**Speed.** One helper run per render synthesizes every clip that is not cached, so the models
load once. On a 4-core Intel Mac: about 25 s of start-up (imports, jieba, models), then roughly
real time (a 5 s clip takes about 5 s). The run is killed after 180 s plus 1 s per character,
and when the render is cancelled.

**Fallback.** When Kokoro cannot run (interpreter or model file missing, the helper fails or
times out), the render prints one warning and narrates the whole video with `say`, so voices
never mix within a video. A cancel stops the helper and cancels the render.

**Limits.** Speed is fixed at 1.0 (no DSL field). Korean and other languages are read as
English. Word-by-word subtitles work unchanged with Kokoro audio (recognition and alignment
do not care which engine spoke).

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

Where things live:

- **`whisper-cli`**: the lookup order is `--whisper-cli`, then `$VIMMER_WHISPER_CLI`, then `whisper-cli`
  on `PATH`. A self-built copy at `~/.local/bin/whisper-cli` is found only when `~/.local/bin` is on
  `PATH`; otherwise pass it explicitly, e.g. `export VIMMER_WHISPER_CLI=$HOME/.local/bin/whisper-cli`.
- **Model**: `~/.local/share/vimmer/models/ggml-large-v3-turbo-q5_0.bin` (the default when
  `--whisper-model` and `$VIMMER_WHISPER_MODEL` are not set).
- **Intel Mac**: builds for x86_64 run whisper CPU-only (see Timing below). Expect it to be slow
  with the large model; use a smaller model for quick checks.

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
dimmed. After a word ends it stays yellow for 6 more frames (so short pauses do not flicker),
but never past the start of the next scene that does not open with a transition. Chunked
renders clip words with their cue: words outside a chunk become zero-length at its edge, so a
line split across two chunks reads the same in both, and since chunks only split at such scene
starts, every word has the same state on every frame as in a full render.

- **Edited cues.** `words` must spell the cue's `text` (whitespace aside). When they do not,
  e.g. the text was edited after timing, the renderer shows the cue as one plain line and the
  render times it again from the narration (with a warning); such a JSON is still valid. The
  Studio drops a cue's `words` as soon as its text, start or end is edited.
- **Which narration times a cue.** A cue that overlaps several narrated scenes (an imported
  SRT line that starts just before a scene ends, say) is timed from the scene whose script
  contains most of its text. A cue whose words would mostly be zero-length (most of its text is
  not in that narration) stays plain instead of showing a line that never lights up.

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

## For developers

- **Building a `SubtitleCue`**: use `SubtitleCue::new(start_ms, end_ms, text)` or
  `SubtitleCue { ..Default::default() }`. A struct literal with every field listed breaks
  whenever a field is added (as `words` was).
- **Building a `Scene` or `VideoTimeline`**: neither implements `Default`. Build them from JSON
  with `serde_json::from_value` (as `vimmer-storyboard` does) so that new DSL fields take their
  serde defaults. A literal would fail to compile when a field is added.

## Not yet built

- Studio: no direct render from the editor yet (export JSON, then run the CLI).
  Subtitle cues are not drag-movable on the timeline; edit them in the inspector.
