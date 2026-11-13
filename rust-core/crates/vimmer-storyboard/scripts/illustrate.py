#!/usr/bin/env python3
"""Ask Claude for a video's illustration style, or for one scene's SVG illustration.

Used by `vimmer generate --illustrate` and `vimmer illustrate` (see README "Illustrations").
One run makes one Messages API call and prints one JSON object on stdout:

    {"ok": true, "kind": "style", "style": {...}, "model": .., "usage": {..}, "warnings": [..]}
    {"ok": true, "kind": "scene", "svg": "<svg ...>...</svg>", "model": .., "usage": {..}, ..}
    {"ok": false, "error": {"kind": "...", "message": "..."}, "usage": {...} | null, ...}

The request comes from a JSON file written by the Rust side (`--request FILE`):

    {"kind": "style", "model": .., "effort": .., "max_tokens": .., "timeout_secs": ..,
     "title": .., "language": .., "orientation": .., "width": 1920, "height": 1080,
     "scenes": [{"text": .., "image_prompt": ..}, ...]}
    {"kind": "scene", ... same header ..., "style": {...}, "scene": {"text": .., "image_prompt": ..},
     "history": [{"assistant": "<svg ...", "feedback": "..."}]}

The SVG is NOT trusted here or anywhere: the Rust side sanitizes it before it is stored or drawn.
The API key is read from ANTHROPIC_API_KEY only (by the SDK) and is never printed or written.
Usage accounting, prices, the API client and error mapping are shared with
generate_storyboard.py, which sits in the same directory.
"""

import argparse
import json
import os
import re
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
# Run with `python -I`, so the script directory is not on sys.path by default.
if HERE not in sys.path:
    sys.path.insert(0, HERE)

import generate_storyboard as gs  # noqa: E402

DEFAULT_MODEL = "claude-opus-5-5"
DEFAULT_EFFORT = "low"
KINDS = ("style", "scene")
DEFAULT_MAX_TOKENS = {"style": 8000, "scene": 32000}

STYLE_SCHEMA = {
    "type": "object",
    "additionalProperties": False,
    "required": [
        "name",
        "palette",
        "background",
        "line",
        "shapes",
        "composition",
        "mood",
        "motifs",
    ],
    "properties": {
        "name": {"type": "string", "description": "Two to five words naming the style."},
        "palette": {
            "type": "array",
            "items": {"type": "string"},
            "description": "5-7 hex colours (#rrggbb) used by every illustration, darkest first.",
        },
        "background": {
            "type": "string",
            "description": "How backgrounds are painted: colours (hex from the palette) and gradient.",
        },
        "line": {"type": "string", "description": "Outline / stroke rules: width, colour, caps."},
        "shapes": {
            "type": "string",
            "description": "Shape language: geometric or organic, corner radius, shading, texture.",
        },
        "composition": {
            "type": "string",
            "description": "Where subjects sit, horizon line, how the text zone is kept calm.",
        },
        "mood": {"type": "string", "description": "Lighting and atmosphere in one sentence."},
        "motifs": {
            "type": "string",
            "description": "Recurring elements that tie the scenes together.",
        },
    },
}

STYLE_SYSTEM = """You are the art director of a short narrated video. Every scene gets one flat \
vector illustration (an SVG drawn by another call), and white scene text plus subtitles are laid \
over the lower part of each picture. Define ONE illustration style that all scenes share, so the \
video looks like a single, consistent piece.

Requirements for the style:
- Flat illustration / infographic look: simple geometric shapes, limited palette, at most soft \
linear or radial gradients. No photorealism, no text or lettering in the pictures.
- A fairly dark, low-contrast background family, so white text stays readable on top.
- The palette has 5-7 colours, darkest first, chosen to suit the subject and its culture.
- Composition: the main subject sits in the upper {subject_band}; the lower {text_band} is a calm \
area (background, ground, water, soft shapes) with no bright details, because the text goes there.
- Keep it achievable in hand-written SVG of moderate size.

Canvas: {width}x{height} ({orientation}). Language of the video: {language}.
Fill in every field of the JSON schema. Be concrete (hex colours, stroke widths in px)."""

SCENE_SYSTEM = """You draw one illustration of a narrated video as a single hand-written SVG.

Output ONLY the SVG document: start with `<svg` and end with `</svg>`. No Markdown fence, no \
commentary.

Canvas and format:
- Root element: <svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 {width} {height}" \
width="{width}" height="{height}">. Fill the whole canvas with the background first.
- Use only basic SVG: g, path, rect, circle, ellipse, line, polyline, polygon, defs, \
linearGradient, radialGradient, stop, clipPath, mask, use, symbol, pattern, and simple filters \
(feGaussianBlur, feOffset, feBlend, feColorMatrix, feFlood, feComposite, feMerge, feDropShadow).
- Forbidden (they are stripped or rejected): script, foreignObject, image, a, iframe, animation \
elements, event attributes (onload, onclick, ...), any external link or URL. A `href` or `url()` \
may only point inside the document (`#id`). No CSS @import, no web fonts.
- No text, letters, digits or logos inside the picture: the scene text is overlaid separately.
- Keep the file under 40 KB: prefer a few dozen well-chosen shapes over thousands of points. \
Round coordinates to whole numbers.

Layout (the scene text, about two lines of large white type, and a subtitle line are drawn over \
the picture by the video renderer):
- Main subject in the upper {subject_band} of the canvas, clearly readable at a glance.
- The lower {text_band} (y > {calm_y}) must stay calm and fairly dark: background, ground, water \
or soft shapes only, no bright highlights or busy detail.
- Keep important content at least {margin}px from every edge (the camera slowly zooms and pans).

Style guide (follow it exactly; every scene of this video uses it):
{style}"""

SVG_RE = re.compile(r"<svg\b.*</svg\s*>", re.S | re.I)


# ---------------------------------------------------------------------------------------------
# Prompts


def layout(req):
    """Bands of the canvas: where the subject goes and where the overlaid text needs calm."""
    w, h = int(req["width"]), int(req["height"])
    portrait = h > w
    # The renderer's scene text is bottom-aligned 260 px above the bottom edge, 92 px type, and
    # subtitles sit below it; portrait text wraps to more lines, so it needs a taller calm band.
    calm_fraction = 0.42 if portrait else 0.45
    return {
        "width": w,
        "height": h,
        "subject_band": f"{round((1 - calm_fraction) * 100)}%",
        "text_band": f"{round(calm_fraction * 100)}%",
        "calm_y": round(h * (1 - calm_fraction)),
        "margin": round(min(w, h) * 0.06),
    }


def format_style(style):
    lines = []
    for key in ("name", "palette", "background", "line", "shapes", "composition", "mood", "motifs"):
        value = style.get(key)
        if value is None:
            continue
        if isinstance(value, list):
            value = ", ".join(str(v) for v in value)
        lines.append(f"- {key}: {value}")
    return "\n".join(lines)


def style_messages(req):
    scenes = []
    for i, s in enumerate(req.get("scenes") or [], 1):
        prompt = (s.get("image_prompt") or "").strip() or "(none; picture the scene text)"
        scenes.append(f"{i}. text: {s.get('text', '').strip()}\n   picture: {prompt}")
    content = (
        f"<video title>{req.get('title', '').strip()}</video title>\n"
        "<scenes>\n" + "\n".join(scenes) + "\n</scenes>\n\n"
        "Define the shared illustration style for these scenes."
    )
    return [{"role": "user", "content": content}]


def scene_messages(req):
    scene = req["scene"]
    prompt = (scene.get("image_prompt") or "").strip() or scene.get("text", "").strip()
    content = (
        f"<video title>{req.get('title', '').strip()}</video title>\n"
        f"<scene text>{scene.get('text', '').strip()}</scene text>\n"
        f"<picture>{prompt}</picture>\n\n"
        "Draw the SVG illustration for this scene."
    )
    messages = [{"role": "user", "content": content}]
    for turn in req.get("history") or []:
        messages.append({"role": "assistant", "content": turn["assistant"]})
        messages.append(
            {
                "role": "user",
                "content": "The SVG was rejected by the checker:\n"
                + turn["feedback"].strip()
                + "\n\nFix these problems and output the complete corrected SVG only.",
            }
        )
    return messages


def system_prompt(req):
    bands = layout(req)
    if req["kind"] == "style":
        return STYLE_SYSTEM.format(
            orientation=req.get("orientation", "landscape"),
            language=req.get("language", "en"),
            **bands,
        )
    return SCENE_SYSTEM.format(style=format_style(req["style"]), **bands)


def request_params(req):
    """Keyword arguments for client.beta.messages.stream()."""
    kind = req["kind"]
    params = {
        "model": req["model"],
        "max_tokens": req.get("max_tokens") or DEFAULT_MAX_TOKENS[kind],
        "system": system_prompt(req),
        "messages": style_messages(req) if kind == "style" else scene_messages(req),
        "thinking": {"type": "adaptive"},
        "output_config": {"effort": req["effort"]},
    }
    if kind == "style":
        params["output_config"]["format"] = {"type": "json_schema", "schema": STYLE_SCHEMA}
    if req["model"] in gs.FALLBACK_MODELS:
        params["betas"] = [gs.FALLBACK_BETA]
        params["fallbacks"] = "default"
    return params


# ---------------------------------------------------------------------------------------------
# Replies


def extract_svg(text):
    """The `<svg ...>...</svg>` part of a reply (tolerates a Markdown fence or a preamble)."""
    match = SVG_RE.search(text or "")
    return match.group(0).strip() if match else None


def interpret(message, req):
    """Turns a final message (dict) into the helper's result, or raises gs.Failure."""
    usage, warnings = gs.summarize_usage(message, req["model"])
    model = message.get("model") or req["model"]
    stop = message.get("stop_reason")
    if stop == "refusal":
        details = message.get("stop_details") or {}
        what = details.get("category") or "unspecified"
        raise gs.Failure(
            "refusal", f"Claude declined the request (refusal, category {what})", usage, model, stop
        )
    texts = [b.get("text", "") for b in message.get("content") or [] if b.get("type") == "text"]
    text = "".join(texts).strip()
    if stop == "max_tokens":
        raise gs.Failure(
            "max_tokens",
            "the reply hit max_tokens before it was complete; lower the effort or raise the cap",
            usage,
            model,
            stop,
        )
    result = {
        "ok": True,
        "kind": req["kind"],
        "model": model,
        "stop_reason": stop,
        "usage": usage,
        "warnings": warnings,
    }
    if req["kind"] == "style":
        try:
            style = json.loads(text)
        except ValueError as e:
            raise gs.Failure("bad_reply", f"the style reply is not JSON: {e}", usage, model, stop)
        if not isinstance(style, dict):
            raise gs.Failure("bad_reply", "the style reply is not an object", usage, model, stop)
        result["style"] = style
        return result
    svg = extract_svg(text)
    # A reply without a complete <svg> element is still returned, so the Rust side can send it
    # back with the checker's complaint and ask for a correction.
    result["svg"] = svg if svg is not None else text
    return result


# ---------------------------------------------------------------------------------------------
# Command line


def load_request(path):
    with open(path, encoding="utf-8") as f:
        req = json.load(f)
    if req.get("kind") not in KINDS:
        raise gs.Failure("usage", f"kind must be one of {', '.join(KINDS)}")
    req.setdefault("model", DEFAULT_MODEL)
    req.setdefault("effort", DEFAULT_EFFORT)
    req.setdefault("orientation", "landscape")
    if req["effort"] not in gs.EFFORTS:
        raise gs.Failure("usage", f"effort must be one of {', '.join(gs.EFFORTS)}")
    for key in ("width", "height"):
        if not isinstance(req.get(key), int) or not 16 <= req[key] <= 8192:
            raise gs.Failure("usage", f"{key} must be an integer between 16 and 8192")
    if req["kind"] == "scene":
        if not isinstance(req.get("style"), dict):
            raise gs.Failure("usage", "a scene request needs a `style` object")
        if not isinstance(req.get("scene"), dict):
            raise gs.Failure("usage", "a scene request needs a `scene` object")
    elif not req.get("scenes"):
        raise gs.Failure("usage", "a style request needs `scenes`")
    return req


def run(req, client_factory=gs.make_client, record=None):
    if not os.environ.get("ANTHROPIC_API_KEY", "").strip():
        raise gs.Failure(
            "no_api_key",
            "ANTHROPIC_API_KEY is not set (export it in ~/.zshrc, then open a new shell)",
        )
    params = request_params(req)
    client = client_factory(float(req.get("timeout_secs") or 300))
    what = "a style" if req["kind"] == "style" else "an illustration"
    print(
        f"asking {req['model']} for {what} (effort {req['effort']}) ...",
        file=sys.stderr,
        flush=True,
    )
    message = gs.call_api(client, params)
    if record:
        with open(record, "w", encoding="utf-8") as f:
            json.dump(message, f, ensure_ascii=False, indent=1)
    return interpret(message, req)


def main(argv=None, client_factory=gs.make_client, out=None):
    out = out or sys.stdout
    p = argparse.ArgumentParser(description="Ask Claude for an illustration style or an SVG.")
    p.add_argument("--request", required=True, help="request JSON written by vimmer")
    p.add_argument("--record", help="also save the raw API reply (JSON) to this file")
    args = p.parse_args(sys.argv[1:] if argv is None else argv)
    try:
        result = run(load_request(args.request), client_factory, args.record)
        code = 0
    except gs.Failure as f:
        result = {
            "ok": False,
            "error": {"kind": f.kind, "message": f.message},
            "usage": f.usage,
            "model": f.model,
            "stop_reason": f.stop_reason,
        }
        code = 1
    except (OSError, ValueError, KeyError) as e:
        result = {"ok": False, "error": {"kind": "input", "message": str(e)}, "usage": None}
        code = 1
    out.write(json.dumps(result, ensure_ascii=False) + "\n")
    out.flush()
    return code


if __name__ == "__main__":
    sys.exit(main())
