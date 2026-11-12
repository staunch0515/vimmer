# Vimmer — text-to-video generation

Self-hosted, modular pipeline that turns a JSON scene description into a short
(≤ 200 s) 1080p / 30 fps video.

```
repo_vimmer/
├── fixtures/                 # Shared JSON contract fixtures (Rust tests + Remotion props)
│   └── sample-timeline.json  # 15-second single-scene baseline
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

## Commands

```bash
# Rust: DSL tests (5 unit + 3 fixture tests)
cd rust-core && cargo test

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

## Not yet built

- TTS generation (duration tracking exists for existing audio files).
- Studio UI (dark multi-panel layout, timeline, inspector). The design spec is
  deferred until the frontend phase starts.
