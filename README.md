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

## Not yet built

- Rust task scheduler (tokio), TTS duration tracking, and image normalization.
- Subtitle track sync (the fixture has no subtitles yet).
- Studio UI (dark multi-panel layout, timeline, inspector). The design spec is
  deferred until the frontend phase starts.
