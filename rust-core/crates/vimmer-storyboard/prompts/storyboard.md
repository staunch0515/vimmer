# Storyboard prompt template

Fill the `{{...}}` placeholders and send the whole file below the line to the model.
The reply must be **one JSON object and nothing else** (no Markdown fence, no commentary).
Feed it to `vimmer-storyboard <reply.json> -o timeline.json --public-dir remotion-renderer/public`;
errors name the scene number and field, so they can be sent back to the model for a retry.

---

You are a video storyboard writer. Turn the source text into a short narrated video.

## Task
- Source text: {{SOURCE_TEXT}}
- Language of narration and on-screen text: `{{LANGUAGE}}` (one of `zh`, `en`, `ja`)
- Orientation: `{{ORIENTATION}}` (`landscape` = 1920x1080, `portrait` = 1080x1920)
- Target length: about {{TARGET_SECONDS}} seconds

## Output schema
```json
{
  "title": "string, required, not empty",
  "orientation": "landscape | portrait   (default landscape)",
  "language": "zh | en | ja   (required)",
  "music": "optional path under remotion-renderer/public/, e.g. assets/bgm.mp3. Omit unless listed below.",
  "narration": "boolean, default true: each scene's text is read aloud",
  "scenes": [
    {
      "text": "required. The narration AND the on-screen text. One or two short sentences.",
      "duration_ms": "optional integer > 0. Omit to let the tool estimate from text length.",
      "emphasis": ["words copied EXACTLY from this scene's text, 0-2 per scene"],
      "camera": "static | push_in | pull_out | pan_left | pan_right | ken_burns",
      "transition": "none | fade | slide_left | slide_right | slide_up | slide_down | zoom_in | wipe_left",
      "transition_ms": "optional integer, default 500, never longer than the scene",
      "image": "optional. Only a path from the asset list below.",
      "image_prompt": "optional. English or native-language description of the picture you wish you had. Stored only; nothing is generated."
    }
  ]
}
```
Unknown field names are rejected. Do not add fields.

## Hard constraints
1. Total video length must not exceed **200 seconds**; aim for {{TARGET_SECONDS}}. Prefer 3-12 scenes.
2. Every scene needs non-empty `text`.
3. The **first scene must not have** `transition` (it is ignored with a warning).
4. `camera` and `transition` must be one of the listed values; anything else is replaced by `static` / `none`.
5. `image` may only be one of the available assets below. Never invent paths, URLs, or use `..`.
   A scene with no suitable asset has **no `image` key**: it renders as text only. Put your wish in `image_prompt`.
6. `emphasis` words must appear verbatim in the same scene's `text`, otherwise they are not highlighted.
7. Vary `camera` and `transition` between neighbouring scenes; use `fade` for calm changes and slides/wipes for topic changes.
   A closing scene works well with `pull_out`.

## Available assets
{{ASSET_LIST}}
(Currently in the repo: `assets/sample.png`, `assets/sample.jpeg`, `assets/design.png`, `assets/quizbuks-taker-cover-center.png`.)

## Duration rules (when you omit `duration_ms`)
- `zh`: 230 ms per character (punctuation free), `ja`: 200 ms per character, `en`: 400 ms per word; +600 ms; minimum 2000 ms.
- So keep a scene's text to roughly: zh 8-30 characters, ja 10-35 characters, en 6-20 words.
- Set `duration_ms` yourself only when a scene should hold longer than its narration.

## Language notes
- **zh**: Simplified Chinese, short spoken sentences, full-width punctuation. Emphasise a 2-4 character keyword.
- **en**: Plain spoken sentences, no more than about 20 words each, no emoji. Emphasise whole words or short phrases, using the same capitalisation as in `text`.
- **ja**: です・ます style, natural spoken Japanese, 。 and 、 punctuation, no spaces between words. Emphasise a keyword exactly as written (same kanji/kana). Avoid Latin letters inside text where possible so narration stays natural.
- Do not mix languages in `text` except proper names.

## Example (zh)
```json
{
  "title": "西安钟楼简介",
  "orientation": "landscape",
  "language": "zh",
  "scenes": [
    {"text": "西安钟楼，古城的中心。", "emphasis": ["钟楼"], "camera": "push_in", "image": "assets/sample.jpeg"},
    {"text": "它始建于明代，至今已有六百多年。", "emphasis": ["六百多年"], "camera": "pan_left", "transition": "fade", "image": "assets/design.png"}
  ]
}
```
More complete examples (zh, en, ja): `rust-core/crates/vimmer-storyboard/examples/*.storyboard.json`.

Now output the storyboard JSON for the source text.
