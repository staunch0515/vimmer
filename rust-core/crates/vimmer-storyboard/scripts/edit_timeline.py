#!/usr/bin/env python3
"""Ask Claude for a timeline edit as a JSON Patch (see README "Editing assistant").

Used by `vimmer edit` and the editor's assistant panel. One run makes one Messages API call
and prints one JSON object on stdout:

    {"ok": true, "reply_json": "{\"status\": ..., \"explanation\": ..., \"patch\": [...]}",
     "model": "...", "stop_reason": "end_turn", "usage": {...}, "warnings": [...]}
    {"ok": false, "error": {"kind": "...", "message": "..."}, "usage": {...} | null, ...}

The request comes from a JSON file written by the Rust side (`--request FILE`):

    {"timeline": {...}, "instruction": "...", "scene_id": "scene-02" | null,
     "assets": ["assets/sample.png", ...], "model": .., "effort": .., "max_tokens": ..,
     "timeout_secs": .., "history": [{"assistant": "<earlier reply>", "feedback": "<error>"}]}

The patch is NOT trusted here: the Rust side applies it to a copy and validates it, and sends
any rejection back through `history`. The system prompt (prompts/edit.md) is the same for every
request, so it is cached. The API key is read from ANTHROPIC_API_KEY only (by the SDK) and is
never printed or written. Usage accounting, prices, the API client and error mapping are shared
with generate_storyboard.py, which sits in the same directory.
"""

import argparse
import json
import os
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
# Run with `python -I`, so the script directory is not on sys.path by default.
if HERE not in sys.path:
    sys.path.insert(0, HERE)

import generate_storyboard as gs  # noqa: E402

DEFAULT_MODEL = "claude-opus-5-5"
DEFAULT_EFFORT = "low"
DEFAULT_MAX_TOKENS = 16000
DEFAULT_PROMPT_FILE = os.path.join(HERE, "..", "prompts", "edit.md")
STATUSES = ("edited", "clarify", "refused")
OPS = ("add", "remove", "replace", "move", "copy", "test")

EDIT_SCHEMA = {
    "type": "object",
    "additionalProperties": False,
    "required": ["status", "explanation", "patch"],
    "properties": {
        "status": {"type": "string", "enum": list(STATUSES)},
        "explanation": {
            "type": "string",
            "description": "1-3 short sentences in the user's language.",
        },
        "patch": {
            "type": "array",
            "description": "RFC 6902 operations; empty unless status is edited.",
            "items": {
                "type": "object",
                "additionalProperties": False,
                "required": ["op", "path", "from", "value_json"],
                "properties": {
                    "op": {"type": "string", "enum": list(OPS)},
                    "path": {"type": "string", "description": "JSON Pointer, e.g. /scenes/1/duration_ms"},
                    "from": {"type": "string", "description": "Source pointer for move/copy, else empty."},
                    "value_json": {
                        "type": "string",
                        "description": "The value as JSON text for add/replace/test, else empty.",
                    },
                },
            },
        },
    },
}


def build_system_prompt(template):
    """The part of prompts/edit.md below the first `---` line."""
    marker = "\n---\n"
    if marker in template:
        template = template.split(marker, 1)[1]
    return template.strip() + "\n"


def _clip(text, n=60):
    text = " ".join(str(text).split())
    return text if len(text) <= n else text[: n - 1] + "…"


def scene_table(timeline):
    """One line per scene: number, id, start, length and the fields people talk about."""
    lines = []
    start = 0
    for i, s in enumerate(timeline.get("scenes") or []):
        dur = s.get("duration_ms") or 0
        parts = [
            f"{i + 1}. id={s.get('id')} path=/scenes/{i}",
            f"start={start / 1000:.2f}s",
            f"duration={dur}ms",
        ]
        tr = s.get("transition_in")
        if tr:
            parts.append(f"transition_in={tr.get('kind')} {tr.get('duration_ms')}ms")
        if s.get("narration") and not s.get("audio"):
            parts.append("narrated")
        if s.get("keyframes"):
            parts.append(f"keyframes={len(s['keyframes'])}")
        parts.append(f"text={json.dumps(_clip(s.get('text') or ''), ensure_ascii=False)}")
        lines.append("  ".join(parts))
        start += dur
    return lines, start


def build_user_message(req):
    timeline = req["timeline"]
    lines, total = scene_table(timeline)
    res = timeline.get("resolution") or {}
    out = [
        f"Format: {res.get('width')}x{res.get('height')}, {len(lines)} scenes, total {total} ms, "
        f"{len(timeline.get('subtitles') or [])} subtitle cues, music: "
        f"{'yes' if timeline.get('music') else 'none'}.",
        "Scenes (numbered from 1 as people say them; JSON indices start at 0):",
        *lines,
        "",
    ]
    scene_id = req.get("scene_id")
    if scene_id:
        out.append(f"Selected scene: {scene_id} (\"this scene\" means this one).")
    else:
        out.append("Selected scene: none.")
    assets = req.get("assets") or []
    if assets:
        out.append("Asset files available (use exactly these paths):")
        out.extend(f"- {a}" for a in assets)
    else:
        out.append("Asset files available: only the paths already used in the timeline.")
    out += [
        "",
        "<timeline>",
        json.dumps(timeline, ensure_ascii=False, indent=1),
        "</timeline>",
        "",
        "<instruction>",
        str(req["instruction"]).strip(),
        "</instruction>",
    ]
    return "\n".join(out)


def build_messages(req):
    messages = [{"role": "user", "content": build_user_message(req)}]
    for turn in req.get("history") or []:
        messages.append({"role": "assistant", "content": turn["assistant"]})
        messages.append(
            {
                "role": "user",
                "content": "The validator rejected that reply:\n"
                + turn["feedback"].strip()
                + "\n\nFix it and reply with the complete corrected JSON object. Patch ops "
                "apply to the original timeline above, not to your earlier attempt.",
            }
        )
    return messages


def request_params(req, system):
    params = {
        "model": req["model"],
        "max_tokens": req.get("max_tokens") or DEFAULT_MAX_TOKENS,
        # The prompt is the same for every edit: cache it (helps retries and repeated edits).
        "system": [{"type": "text", "text": system, "cache_control": {"type": "ephemeral"}}],
        "messages": build_messages(req),
        "thinking": {"type": "adaptive"},
        "output_config": {
            "effort": req["effort"],
            "format": {"type": "json_schema", "schema": EDIT_SCHEMA},
        },
    }
    if req["model"] in gs.FALLBACK_MODELS:
        params["betas"] = [gs.FALLBACK_BETA]
        params["fallbacks"] = "default"
    return params


def load_request(path):
    with open(path, encoding="utf-8") as f:
        req = json.load(f)
    req.setdefault("model", DEFAULT_MODEL)
    req.setdefault("effort", DEFAULT_EFFORT)
    if not isinstance(req.get("timeline"), dict):
        raise gs.Failure("usage", "the request needs a `timeline` object")
    if not str(req.get("instruction", "")).strip():
        raise gs.Failure("usage", "the instruction is empty")
    if req["effort"] not in gs.EFFORTS:
        raise gs.Failure("usage", f"effort must be one of {', '.join(gs.EFFORTS)}")
    return req


def run(req, prompt_file, client_factory=gs.make_client, record=None):
    if not os.environ.get("ANTHROPIC_API_KEY", "").strip():
        raise gs.Failure(
            "no_api_key",
            "ANTHROPIC_API_KEY is not set (export it in ~/.zshrc, then open a new shell)",
        )
    with open(prompt_file, encoding="utf-8") as f:
        system = build_system_prompt(f.read())
    params = request_params(req, system)
    client = client_factory(float(req.get("timeout_secs") or 300))
    attempt = len(req.get("history") or []) + 1
    print(
        f"asking {req['model']} for an edit (effort {req['effort']}, attempt {attempt}) ...",
        file=sys.stderr,
        flush=True,
    )
    message = gs.call_api(client, params)
    if record:
        with open(record, "w", encoding="utf-8") as f:
            json.dump(message, f, ensure_ascii=False, indent=1)
    result = gs.interpret(message, req["model"])
    result["reply_json"] = result.pop("storyboard_json")
    return result


def main(argv=None, client_factory=gs.make_client, out=None):
    out = out or sys.stdout
    p = argparse.ArgumentParser(description="Ask Claude for a timeline edit (JSON Patch).")
    p.add_argument("--request", required=True, help="request JSON written by vimmer")
    p.add_argument("--prompt-file", default=DEFAULT_PROMPT_FILE)
    p.add_argument("--record", help="also save the raw API reply (JSON) to this file")
    args = p.parse_args(sys.argv[1:] if argv is None else argv)
    try:
        result = run(load_request(args.request), args.prompt_file, client_factory, args.record)
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
