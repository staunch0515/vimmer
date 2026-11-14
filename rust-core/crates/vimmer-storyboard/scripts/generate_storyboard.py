#!/usr/bin/env python3
"""Ask Claude for a Vimmer storyboard (see README "Generate a video from text").

One run makes one Messages API call and prints one JSON object on stdout:

    {"ok": true,  "storyboard_json": "...", "model": "...", "stop_reason": "end_turn",
     "usage": {"input_tokens": .., "output_tokens": .., "cache_creation_input_tokens": ..,
               "cache_read_input_tokens": .., "cost_usd": ..}, "warnings": [...]}
    {"ok": false, "error": {"kind": "...", "message": "..."}, "usage": {...} | null, ...}

and exits 0 when ok, 1 otherwise. Progress goes to stderr. The API key is read from
ANTHROPIC_API_KEY only and is never printed or written anywhere.

Input comes either from flags (standalone use) or from a request JSON written by the Rust
side (`--request FILE`), which may also carry `history`: earlier replies with the validation
errors they produced, so Claude can correct them.

    generate_storyboard.py --text-file story.txt --lang zh > reply.json
"""

import argparse
import json
import os
import sys

DEFAULT_MODEL = "claude-opus-5-5"
DEFAULT_EFFORT = "medium"
EFFORTS = ("low", "medium", "high", "xhigh", "max")
LANGS = ("zh", "en", "ja")
ORIENTATIONS = ("landscape", "portrait")
# Streaming request, so a generous cap does not risk an HTTP timeout. Thinking counts too.
DEFAULT_MAX_TOKENS = 32000
FALLBACK_BETA = "server-side-fallback-2026-07-01"
# Models that accept the server-side refusal fallback (`fallbacks: "default"`).
# Claude Haiku 5.5 has none: a declined request stays declined.
FALLBACK_MODELS = {
    "claude-opus-5-5",
    "claude-opus-5",
    "claude-sonnet-5-5",
    "claude-fable-5-2",
    "claude-fable-5-1",
}

# USD per million tokens: input, output, cache read. Cache writes (5 minute) cost 1.25x input.
# Anthropic first-party API prices, as listed in the claude-api skill (2026-11-03).
PRICES = {
    "claude-fable-5-2": (8.00, 40.00, 0.20),
    "claude-fable-5-1": (10.00, 50.00, 1.00),
    "claude-opus-5-5": (4.00, 20.00, 0.20),
    "claude-opus-5": (5.00, 25.00, 0.50),
    "claude-opus-4-8": (5.00, 25.00, 0.50),
    "claude-sonnet-5-5": (2.00, 10.00, 0.10),
    "claude-sonnet-5": (2.00, 10.00, 0.20),
    "claude-haiku-5-5": (0.10, 0.50, 0.01),
}
# Claude Haiku 5.5 costs more for prompts above 100K tokens.
HAIKU_LONG_PROMPT = 100_000
HAIKU_LONG_PRICES = (0.50, 2.50, 0.05)

HERE = os.path.dirname(os.path.abspath(__file__))
DEFAULT_PROMPT_FILE = os.path.join(HERE, "..", "prompts", "storyboard.md")
DEFAULT_SCHEMA_FILE = os.path.join(HERE, "..", "schema", "storyboard.schema.json")

NO_ASSETS = (
    "None in this run. Do not set `image` (the schema has no such field): every "
    "scene renders as text only. Describe the picture you would want in `image_prompt`."
)
NO_MUSIC = "None in this run. Do not set `music` (the schema has no such field)."


def music_text(moods):
    """The Background music section: the moods on offer, or none."""
    if not moods:
        return NO_MUSIC
    listed = ", ".join("`%s`" % m for m in moods)
    return (
        "Choose the ONE mood that best fits the whole video from: " + listed + ". "
        "Write it as `\"music\": {\"mood\": \"...\"}`; a track with that mood is picked from "
        "the music library at render time and plays quietly under the narration. "
        "Optionally add `energy` from 0 (quiet, slow) to 1 (intense) when the text clearly calls "
        "for one end; otherwise leave it out. Serious or sad topics usually want `calm`; how-tos "
        "and product news `upbeat`; stories with stakes `tense` or `epic`."
    )


def music_schema(moods):
    """The `music` property offered to Claude: one of `moods`, optional energy."""
    return {
        "type": "object",
        "properties": {
            "mood": {"type": "string", "enum": list(moods)},
            "energy": {
                "type": "number",
                "description": "0 (quiet, slow) to 1 (intense). Optional.",
            },
        },
        "required": ["mood"],
        "additionalProperties": False,
    }


class Failure(Exception):
    def __init__(self, kind, message, usage=None, model=None, stop_reason=None):
        super().__init__(message)
        self.kind = kind
        self.message = message
        self.usage = usage
        self.model = model
        self.stop_reason = stop_reason


# ---------------------------------------------------------------------------------------------
# Prompt and schema


def build_system_prompt(template, lang, orientation, target_seconds, moods=None):
    """The part of prompts/storyboard.md below the first `---` line, filled in."""
    marker = "\n---\n"
    if marker in template:
        template = template.split(marker, 1)[1]
    lines = [
        line
        for line in template.splitlines()
        if not line.startswith("(Currently in the repo:")
    ]
    text = "\n".join(lines).strip() + "\n"
    target = (
        str(int(target_seconds))
        if target_seconds
        else "whatever suits the source text (usually 30-90)"
    )
    return (
        text.replace("{{SOURCE_TEXT}}", "the text inside <source> in the user message")
        .replace("{{LANGUAGE}}", lang)
        .replace("{{ORIENTATION}}", orientation)
        .replace("{{TARGET_SECONDS}}", target)
        .replace("{{ASSET_LIST}}", NO_ASSETS)
        .replace("{{MUSIC}}", music_text(moods))
    )


def build_schema(base, lang, orientation, moods=None):
    """The storyboard schema with `language` and `orientation` pinned to this request, and a
    `music` mood when moods are offered."""
    schema = json.loads(json.dumps(base))
    props = schema["properties"]
    if moods:
        props["music"] = music_schema(moods)
    props["language"]["enum"] = [lang]
    props["orientation"]["enum"] = [orientation]
    for key in ("language", "orientation"):
        if key not in schema["required"]:
            schema["required"].append(key)
    return schema


def build_messages(source_text, history):
    """First the source text, then each earlier reply followed by the errors it caused."""
    messages = [
        {
            "role": "user",
            "content": "<source>\n" + source_text.strip() + "\n</source>\n\n"
            "Write the storyboard JSON for this source text.",
        }
    ]
    for turn in history or []:
        messages.append({"role": "assistant", "content": turn["assistant"]})
        messages.append(
            {
                "role": "user",
                "content": "The storyboard was rejected by the validator:\n"
                + turn["feedback"].strip()
                + "\n\nFix these problems and output the complete corrected storyboard JSON.",
            }
        )
    return messages


def request_params(req, system, schema):
    """Keyword arguments for client.beta.messages.stream()."""
    params = {
        "model": req["model"],
        "max_tokens": req.get("max_tokens") or DEFAULT_MAX_TOKENS,
        "system": system,
        "messages": build_messages(req["text"], req.get("history")),
        "thinking": {"type": "adaptive"},
        "output_config": {
            "effort": req["effort"],
            "format": {"type": "json_schema", "schema": schema},
        },
    }
    if req["model"] in FALLBACK_MODELS:
        params["betas"] = [FALLBACK_BETA]
        params["fallbacks"] = "default"
    return params


# ---------------------------------------------------------------------------------------------
# Usage and cost


def prices_for(model, prompt_tokens):
    if model == "claude-haiku-5-5" and prompt_tokens > HAIKU_LONG_PROMPT:
        return HAIKU_LONG_PRICES
    return PRICES.get(model)


def _cost(model, inp, out, cache_write, cache_read):
    prices = prices_for(model, inp + cache_write + cache_read)
    if prices is None:
        return None
    p_in, p_out, p_read = prices
    return (inp * p_in + cache_write * p_in * 1.25 + cache_read * p_read + out * p_out) / 1e6


def summarize_usage(message, requested_model):
    """Tokens and cost of a message (a dict, as from Message.to_dict()).

    With a server-side fallback the top-level usage covers only the attempt that produced the
    message; `usage.iterations` lists every attempt with its own model, so it is used when
    present. Returns (usage dict, warnings)."""
    usage = message.get("usage") or {}
    served = message.get("model") or requested_model
    entries = usage.get("iterations") or [
        dict(usage, model=served, type="message")
    ]
    total = {
        "input_tokens": 0,
        "output_tokens": 0,
        "cache_creation_input_tokens": 0,
        "cache_read_input_tokens": 0,
        "cost_usd": 0.0,
    }
    warnings = []
    for entry in entries:
        model = entry.get("model") or requested_model
        inp = entry.get("input_tokens") or 0
        out = entry.get("output_tokens") or 0
        cw = entry.get("cache_creation_input_tokens") or 0
        cr = entry.get("cache_read_input_tokens") or 0
        total["input_tokens"] += inp
        total["output_tokens"] += out
        total["cache_creation_input_tokens"] += cw
        total["cache_read_input_tokens"] += cr
        cost = _cost(model, inp, out, cw, cr)
        if cost is None:
            warnings.append(f"no price known for model {model}; its cost is not counted")
        else:
            total["cost_usd"] += cost
        if entry.get("type") == "fallback_message":
            warnings.append(f"{requested_model} declined; served by fallback model {model}")
    total["cost_usd"] = round(total["cost_usd"], 6)
    return total, warnings


def interpret(message, requested_model):
    """Turns a final message (dict) into the helper's result, or raises Failure."""
    usage, warnings = summarize_usage(message, requested_model)
    model = message.get("model") or requested_model
    stop = message.get("stop_reason")
    if stop == "refusal":
        details = message.get("stop_details") or {}
        what = details.get("category") or "unspecified"
        explanation = details.get("explanation")
        text = f"Claude declined the request (refusal, category {what})"
        if explanation:
            text += f": {explanation}"
        raise Failure("refusal", text, usage, model, stop)
    if stop == "max_tokens":
        raise Failure(
            "max_tokens",
            "the reply hit max_tokens before the storyboard was complete; raise --max-tokens "
            "or lower the effort",
            usage,
            model,
            stop,
        )
    texts = [b.get("text", "") for b in message.get("content") or [] if b.get("type") == "text"]
    text = "".join(texts).strip()
    if not text:
        raise Failure("no_text", f"the reply had no text (stop_reason {stop})", usage, model, stop)
    return {
        "ok": True,
        "storyboard_json": text,
        "model": model,
        "stop_reason": stop,
        "usage": usage,
        "warnings": warnings,
    }


# ---------------------------------------------------------------------------------------------
# API call


def make_client(timeout_secs):
    try:
        import anthropic
    except ImportError as e:
        raise Failure(
            "sdk_missing",
            f"the anthropic Python SDK is not installed for {sys.executable} ({e}); run "
            "rust-core/crates/vimmer-storyboard/scripts/install_llm.sh",
        )
    key = os.environ.get("ANTHROPIC_API_KEY", "").strip()
    return anthropic.Anthropic(
        api_key=key,
        timeout=anthropic.Timeout(timeout_secs, connect=20.0),
        max_retries=2,
    )


def call_api(client, params):
    import anthropic

    try:
        with client.beta.messages.stream(**params) as stream:
            return stream.get_final_message().to_dict()
    except anthropic.AuthenticationError as e:
        raise Failure("auth", f"the API key in ANTHROPIC_API_KEY was rejected (401): {e.message}")
    except anthropic.PermissionDeniedError as e:
        raise Failure("permission", f"permission denied (403): {e.message}")
    except anthropic.NotFoundError as e:
        raise Failure("not_found", f"not found (404), check the model name: {e.message}")
    except anthropic.RateLimitError as e:
        raise Failure("rate_limit", f"rate limited (429) after retries: {e.message}")
    except anthropic.BadRequestError as e:
        raise Failure("bad_request", f"bad request (400): {e.message}")
    except anthropic.APIStatusError as e:
        raise Failure("api_error", f"API error ({e.status_code}): {e.message}")
    except anthropic.APITimeoutError:
        raise Failure("timeout", "the API request timed out")
    except anthropic.APIConnectionError as e:
        raise Failure("connection", f"could not reach the API: {e}")


def generate(req, prompt_file, schema_file, client_factory=make_client, record=None):
    if not os.environ.get("ANTHROPIC_API_KEY", "").strip():
        raise Failure(
            "no_api_key",
            "ANTHROPIC_API_KEY is not set (export it in ~/.zshrc, then open a new shell)",
        )
    with open(prompt_file, encoding="utf-8") as f:
        template = f.read()
    with open(schema_file, encoding="utf-8") as f:
        base_schema = json.load(f)
    moods = req.get("music_moods") or []
    system = build_system_prompt(
        template, req["language"], req["orientation"], req.get("target_seconds"), moods
    )
    schema = build_schema(base_schema, req["language"], req["orientation"], moods)
    params = request_params(req, system, schema)
    client = client_factory(float(req.get("timeout_secs") or 300))
    attempt = len(req.get("history") or []) + 1
    print(
        f"asking {req['model']} (effort {req['effort']}, attempt {attempt}) ...",
        file=sys.stderr,
        flush=True,
    )
    message = call_api(client, params)
    if record:
        # The raw reply (no key, no request headers), e.g. to make a test fixture.
        with open(record, "w", encoding="utf-8") as f:
            json.dump(message, f, ensure_ascii=False, indent=1)
    return interpret(message, req["model"])


# ---------------------------------------------------------------------------------------------
# Command line


def parse_args(argv):
    p = argparse.ArgumentParser(description="Generate a Vimmer storyboard with Claude.")
    p.add_argument("--request", help="request JSON written by vimmer (overrides the flags below)")
    p.add_argument("--text-file", help="source text file ('-' for stdin)")
    p.add_argument("--lang", choices=LANGS)
    p.add_argument("--orientation", choices=ORIENTATIONS, default="landscape")
    p.add_argument("--target-seconds", type=int)
    p.add_argument("--model", default=DEFAULT_MODEL)
    p.add_argument("--effort", choices=EFFORTS, default=DEFAULT_EFFORT)
    p.add_argument("--max-tokens", type=int, default=DEFAULT_MAX_TOKENS)
    p.add_argument("--timeout-secs", type=float, default=300.0)
    p.add_argument("--prompt-file", default=DEFAULT_PROMPT_FILE)
    p.add_argument("--schema-file", default=DEFAULT_SCHEMA_FILE)
    p.add_argument("--record", help="also save the raw API reply (JSON) to this file")
    return p.parse_args(argv)


def load_request(args):
    if args.request:
        with open(args.request, encoding="utf-8") as f:
            req = json.load(f)
    else:
        if not args.text_file or not args.lang:
            raise Failure("usage", "--text-file and --lang are required without --request")
        if args.text_file == "-":
            text = sys.stdin.read()
        else:
            with open(args.text_file, encoding="utf-8") as f:
                text = f.read()
        req = {
            "text": text,
            "language": args.lang,
            "orientation": args.orientation,
            "target_seconds": args.target_seconds,
            "model": args.model,
            "effort": args.effort,
            "max_tokens": args.max_tokens,
            "timeout_secs": args.timeout_secs,
        }
    req.setdefault("orientation", "landscape")
    req.setdefault("model", DEFAULT_MODEL)
    req.setdefault("effort", DEFAULT_EFFORT)
    if not str(req.get("text", "")).strip():
        raise Failure("usage", "the source text is empty")
    if req.get("language") not in LANGS:
        raise Failure("usage", f"language must be one of {', '.join(LANGS)}")
    if req["orientation"] not in ORIENTATIONS:
        raise Failure("usage", f"orientation must be one of {', '.join(ORIENTATIONS)}")
    if req["effort"] not in EFFORTS:
        raise Failure("usage", f"effort must be one of {', '.join(EFFORTS)}")
    return req


def main(argv=None, client_factory=make_client, out=None):
    out = out or sys.stdout
    args = parse_args(sys.argv[1:] if argv is None else argv)
    try:
        req = load_request(args)
        result = generate(
            req, args.prompt_file, args.schema_file, client_factory, args.record
        )
        code = 0
    except Failure as f:
        result = {
            "ok": False,
            "error": {"kind": f.kind, "message": f.message},
            "usage": f.usage,
            "model": f.model,
            "stop_reason": f.stop_reason,
        }
        code = 1
    except (OSError, ValueError) as e:
        result = {"ok": False, "error": {"kind": "input", "message": str(e)}, "usage": None}
        code = 1
    out.write(json.dumps(result, ensure_ascii=False) + "\n")
    out.flush()
    return code


if __name__ == "__main__":
    sys.exit(main())
