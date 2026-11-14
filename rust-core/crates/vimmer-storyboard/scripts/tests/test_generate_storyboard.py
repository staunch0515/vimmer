"""Tests for generate_storyboard.py. No network: API replies come from recorded fixtures.

    ~/.local/share/vimmer/llm-venv/bin/python -m unittest discover -s \
        rust-core/crates/vimmer-storyboard/scripts/tests -v
"""

import importlib.util
import io
import json
import os
import sys
import tempfile
import unittest
from unittest import mock

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, os.path.dirname(HERE))
import generate_storyboard as gs  # noqa: E402

HAS_SDK = importlib.util.find_spec("anthropic") is not None
FAKE_KEY = "sk-ant-test-not-a-real-key"


def fixture(name):
    with open(os.path.join(HERE, "fixtures", name), encoding="utf-8") as f:
        return json.load(f)


class FakeMessage:
    def __init__(self, data):
        self.data = data

    def to_dict(self):
        return self.data


class FakeStream:
    def __init__(self, data):
        self.data = data

    def __enter__(self):
        return self

    def __exit__(self, *exc):
        return False

    def get_final_message(self):
        return FakeMessage(self.data)


class FakeClient:
    """Records the stream() kwargs and replays a fixture (or raises `error`)."""

    def __init__(self, data=None, error=None):
        self.calls = []
        self.data = data
        self.error = error
        self.beta = self
        self.messages = self

    def stream(self, **params):
        self.calls.append(params)
        if self.error:
            raise self.error
        return FakeStream(self.data)


def run_main(argv, client, env_key=FAKE_KEY):
    out = io.StringIO()
    env = {"ANTHROPIC_API_KEY": env_key} if env_key else {}
    with mock.patch.dict(os.environ, env, clear=True), mock.patch("sys.stderr", io.StringIO()):
        code = gs.main(argv, client_factory=lambda timeout: client, out=out)
    return code, json.loads(out.getvalue())


class TextFile:
    def __enter__(self):
        self.dir = tempfile.TemporaryDirectory()
        self.path = os.path.join(self.dir.name, "story.txt")
        with open(self.path, "w", encoding="utf-8") as f:
            f.write("西安钟楼位于古城中心，始建于明代。")
        return self.path

    def __exit__(self, *exc):
        self.dir.cleanup()


class PromptAndSchema(unittest.TestCase):
    def test_system_prompt_is_filled_and_offers_no_assets(self):
        with open(gs.DEFAULT_PROMPT_FILE, encoding="utf-8") as f:
            template = f.read()
        system = gs.build_system_prompt(template, "ja", "portrait", 45)
        self.assertNotIn("{{", system)
        self.assertNotIn("Fill the `{{...}}` placeholders", system)  # header above --- dropped
        self.assertNotIn("Currently in the repo", system)
        self.assertIn("`ja`", system)
        self.assertIn("`portrait`", system)
        self.assertIn("about 45 seconds", system)
        self.assertIn("Do not set `image`", system)
        self.assertIn("<source>", system)
        no_target = gs.build_system_prompt(template, "zh", "landscape", None)
        self.assertIn("whatever suits the source text", no_target)

    def test_offered_moods_go_into_prompt_and_schema(self):
        with open(gs.DEFAULT_PROMPT_FILE, encoding="utf-8") as f:
            template = f.read()
        no_music = gs.build_system_prompt(template, "en", "landscape", None)
        self.assertIn("Do not set `music`", no_music)
        system = gs.build_system_prompt(template, "en", "landscape", None, ["calm", "upbeat"])
        self.assertNotIn("{{", system)
        self.assertIn("`calm`, `upbeat`", system)
        self.assertIn('"music": {"mood": "..."}', system)
        self.assertNotIn("Do not set `music`", system)
        base = {"properties": {"language": {}, "orientation": {}}, "required": []}
        self.assertNotIn("music", gs.build_schema(base, "en", "landscape")["properties"])
        music = gs.build_schema(base, "en", "landscape", ["calm", "upbeat"])["properties"]["music"]
        self.assertEqual(music["properties"]["mood"]["enum"], ["calm", "upbeat"])
        self.assertEqual(music["required"], ["mood"])
        self.assertFalse(music["additionalProperties"])
        self.assertNotIn("music", base["properties"])

    def test_request_moods_reach_the_api_call(self):
        req = {"text": "x", "language": "en", "orientation": "landscape", "model": "m",
               "effort": "low", "music_moods": ["epic"]}
        with open(gs.DEFAULT_PROMPT_FILE, encoding="utf-8") as f:
            system = gs.build_system_prompt(f.read(), "en", "landscape", None, req["music_moods"])
        with open(gs.DEFAULT_SCHEMA_FILE, encoding="utf-8") as f:
            schema = gs.build_schema(json.load(f), "en", "landscape", req["music_moods"])
        params = gs.request_params(req, system, schema)
        fmt = params["output_config"]["format"]["schema"]
        self.assertEqual(fmt["properties"]["music"]["properties"]["mood"]["enum"], ["epic"])
        self.assertIn("`epic`", params["system"])

    def test_schema_pins_language_and_orientation_without_touching_the_base(self):
        with open(gs.DEFAULT_SCHEMA_FILE, encoding="utf-8") as f:
            base = json.load(f)
        schema = gs.build_schema(base, "en", "portrait")
        self.assertEqual(schema["properties"]["language"]["enum"], ["en"])
        self.assertEqual(schema["properties"]["orientation"]["enum"], ["portrait"])
        self.assertIn("orientation", schema["required"])
        self.assertEqual(base["properties"]["language"]["enum"], ["zh", "en", "ja"])
        scene = schema["properties"]["scenes"]["items"]
        self.assertNotIn("image", scene["properties"])
        self.assertFalse(scene["additionalProperties"])
        self.assertFalse(schema["additionalProperties"])

    def test_history_becomes_alternating_turns_ending_with_the_errors(self):
        msgs = gs.build_messages(
            " text ", [{"assistant": '{"a":1}', "feedback": "scene 2: field `text`: empty"}]
        )
        self.assertEqual([m["role"] for m in msgs], ["user", "assistant", "user"])
        self.assertIn("<source>\ntext\n</source>", msgs[0]["content"])
        self.assertEqual(msgs[1]["content"], '{"a":1}')
        self.assertIn("scene 2: field `text`: empty", msgs[2]["content"])

    def test_request_params_use_structured_output_adaptive_thinking_and_fallback(self):
        req = {"text": "x", "model": "claude-opus-5-5", "effort": "high"}
        p = gs.request_params(req, "SYS", {"type": "object"})
        self.assertEqual(p["thinking"], {"type": "adaptive"})
        self.assertEqual(p["output_config"]["effort"], "high")
        self.assertEqual(
            p["output_config"]["format"], {"type": "json_schema", "schema": {"type": "object"}}
        )
        self.assertEqual(p["fallbacks"], "default")
        self.assertEqual(p["betas"], ["server-side-fallback-2026-07-01"])
        self.assertEqual(p["max_tokens"], gs.DEFAULT_MAX_TOKENS)
        self.assertNotIn("temperature", p)
        sonnet = gs.request_params(dict(req, model="claude-sonnet-5-5"), "S", {})
        self.assertEqual(sonnet["fallbacks"], "default")
        # Claude Haiku 5.5 has no server-side fallback.
        haiku = gs.request_params(dict(req, model="claude-haiku-5-5"), "S", {})
        self.assertNotIn("fallbacks", haiku)
        self.assertNotIn("betas", haiku)


class UsageAndCost(unittest.TestCase):
    def test_plain_reply_cost(self):
        result = gs.interpret(fixture("opus_ok.json"), "claude-opus-5-5")
        self.assertTrue(result["ok"])
        self.assertEqual(result["usage"]["input_tokens"], 2100)
        self.assertEqual(result["usage"]["output_tokens"], 1500)
        # 2100 * $4/M + 1500 * $20/M
        self.assertAlmostEqual(result["usage"]["cost_usd"], 0.0084 + 0.03)
        sb = json.loads(result["storyboard_json"])
        self.assertEqual(sb["language"], "zh")
        self.assertEqual(result["warnings"], [])

    def test_recorded_real_reply(self):
        # A real claude-opus-5-5 reply (2026-11-13), saved with --record. Its usage.iterations
        # entry has model null, which falls back to the requested model's price.
        data = fixture("recorded_opus_zh.json")
        self.assertIsNone(data["usage"]["iterations"][0]["model"])
        result = gs.interpret(data, "claude-opus-5-5")
        self.assertEqual(result["stop_reason"], "end_turn")
        self.assertEqual(result["usage"]["input_tokens"], 2756)
        self.assertEqual(result["usage"]["output_tokens"], 959)
        self.assertAlmostEqual(result["usage"]["cost_usd"], 2756 * 4e-6 + 959 * 20e-6)
        sb = json.loads(result["storyboard_json"])
        self.assertEqual(sb["language"], "zh")
        self.assertEqual(len(sb["scenes"]), 10)
        self.assertTrue(all("image" not in s for s in sb["scenes"]))

    def test_fallback_counts_every_attempt_at_its_own_price(self):
        result = gs.interpret(fixture("opus_fallback.json"), "claude-opus-5-5")
        self.assertEqual(result["model"], "claude-opus-4-8")
        self.assertEqual(result["usage"]["input_tokens"], 4000)
        self.assertEqual(result["usage"]["output_tokens"], 800)
        expected = 2000 * 4e-6 + 2000 * 5e-6 + 800 * 25e-6
        self.assertAlmostEqual(result["usage"]["cost_usd"], expected)
        self.assertTrue(any("fallback" in w for w in result["warnings"]))

    def test_cache_tokens_and_unknown_model(self):
        msg = {
            "model": "claude-sonnet-5-5",
            "stop_reason": "end_turn",
            "content": [{"type": "text", "text": "{}"}],
            "usage": {
                "input_tokens": 1000,
                "output_tokens": 100,
                "cache_creation_input_tokens": 1000,
                "cache_read_input_tokens": 10000,
            },
        }
        usage, _ = gs.summarize_usage(msg, "claude-sonnet-5-5")
        expected = (1000 * 2 + 1000 * 2 * 1.25 + 10000 * 0.10 + 100 * 10) / 1e6
        self.assertAlmostEqual(usage["cost_usd"], expected)
        usage, warnings = gs.summarize_usage(dict(msg, model="claude-mystery"), "claude-mystery")
        self.assertEqual(usage["cost_usd"], 0)
        self.assertIn("no price known", warnings[0])

    def test_haiku_long_prompt_price(self):
        self.assertEqual(gs.prices_for("claude-haiku-5-5", 1000), (0.10, 0.50, 0.01))
        self.assertEqual(gs.prices_for("claude-haiku-5-5", 150_000), gs.HAIKU_LONG_PRICES)

    def test_refusal_and_max_tokens_fail_but_keep_usage(self):
        with self.assertRaises(gs.Failure) as ctx:
            gs.interpret(fixture("refusal.json"), "claude-opus-5-5")
        self.assertEqual(ctx.exception.kind, "refusal")
        self.assertIn("cyber", ctx.exception.message)
        self.assertIn("fixture explanation", ctx.exception.message)
        self.assertEqual(ctx.exception.usage["input_tokens"], 1900)
        with self.assertRaises(gs.Failure) as ctx:
            gs.interpret(fixture("max_tokens.json"), "claude-haiku-5-5")
        self.assertEqual(ctx.exception.kind, "max_tokens")
        self.assertEqual(ctx.exception.usage["output_tokens"], 32000)
        self.assertAlmostEqual(ctx.exception.usage["cost_usd"], (1800 * 0.1 + 32000 * 0.5) / 1e6)

    def test_reply_without_text(self):
        msg = {"model": "m", "stop_reason": "end_turn", "content": [], "usage": {}}
        with self.assertRaises(gs.Failure) as ctx:
            gs.interpret(msg, "claude-opus-5-5")
        self.assertEqual(ctx.exception.kind, "no_text")


class MainProgram(unittest.TestCase):
    @unittest.skipUnless(HAS_SDK, "anthropic SDK not installed")
    def test_success_prints_one_json_object(self):
        client = FakeClient(fixture("opus_ok.json"))
        with TextFile() as path:
            code, result = run_main(["--text-file", path, "--lang", "zh"], client)
        self.assertEqual(code, 0)
        self.assertTrue(result["ok"])
        self.assertEqual(len(client.calls), 1)
        params = client.calls[0]
        self.assertEqual(params["model"], "claude-opus-5-5")
        self.assertEqual(params["output_config"]["effort"], "medium")
        schema = params["output_config"]["format"]["schema"]
        self.assertEqual(schema["properties"]["language"]["enum"], ["zh"])
        self.assertIn("西安钟楼", params["messages"][0]["content"])

    @unittest.skipUnless(HAS_SDK, "anthropic SDK not installed")
    def test_request_file_with_history(self):
        client = FakeClient(fixture("opus_ok.json"))
        with tempfile.TemporaryDirectory() as d:
            req = os.path.join(d, "req.json")
            record = os.path.join(d, "raw.json")
            with open(req, "w", encoding="utf-8") as f:
                json.dump(
                    {
                        "text": "Coffee.",
                        "language": "en",
                        "orientation": "portrait",
                        "model": "claude-sonnet-5-5",
                        "effort": "low",
                        "history": [{"assistant": "{}", "feedback": "field `title`: empty"}],
                    },
                    f,
                )
            code, result = run_main(["--request", req, "--record", record], client)
            self.assertEqual(code, 0, result)
            with open(record, encoding="utf-8") as f:
                self.assertEqual(json.load(f)["id"], "msg_fixture_ok")
        params = client.calls[0]
        self.assertEqual(len(params["messages"]), 3)
        self.assertEqual(params["output_config"]["effort"], "low")

    def test_missing_key_fails_before_any_call(self):
        client = FakeClient(fixture("opus_ok.json"))
        with TextFile() as path:
            code, result = run_main(["--text-file", path, "--lang", "zh"], client, env_key=None)
        self.assertEqual(code, 1)
        self.assertEqual(result["error"]["kind"], "no_api_key")
        self.assertEqual(client.calls, [])

    @unittest.skipUnless(HAS_SDK, "anthropic SDK not installed")
    def test_failures_report_kind_and_usage(self):
        with TextFile() as path:
            code, result = run_main(
                ["--text-file", path, "--lang", "zh"], FakeClient(fixture("refusal.json"))
            )
        self.assertEqual(code, 1)
        self.assertEqual(result["error"]["kind"], "refusal")
        self.assertEqual(result["usage"]["input_tokens"], 1900)
        self.assertEqual(result["stop_reason"], "refusal")

    def test_bad_input(self):
        code, result = run_main(["--lang", "zh"], FakeClient())
        self.assertEqual((code, result["error"]["kind"]), (1, "usage"))
        with TextFile() as path:
            code, result = run_main(["--text-file", path + ".missing", "--lang", "zh"], FakeClient())
        self.assertEqual((code, result["error"]["kind"]), (1, "input"))

    @unittest.skipUnless(HAS_SDK, "anthropic SDK not installed")
    def test_api_errors_map_to_kinds_and_never_echo_the_key(self):
        import anthropic
        import httpx2 as httpx

        request = httpx.Request("POST", "https://api.anthropic.com/v1/messages")

        def status_error(cls, code):
            response = httpx.Response(code, request=request, json={"error": {"message": "nope"}})
            return cls("nope", response=response, body=None)

        cases = [
            (status_error(anthropic.AuthenticationError, 401), "auth"),
            (status_error(anthropic.RateLimitError, 429), "rate_limit"),
            (status_error(anthropic.BadRequestError, 400), "bad_request"),
            (status_error(anthropic.InternalServerError, 500), "api_error"),
            (anthropic.APITimeoutError(request=request), "timeout"),
            (anthropic.APIConnectionError(request=request), "connection"),
        ]
        for error, kind in cases:
            with TextFile() as path:
                code, result = run_main(
                    ["--text-file", path, "--lang", "zh"], FakeClient(error=error)
                )
            self.assertEqual((code, result["error"]["kind"]), (1, kind))
            self.assertNotIn(FAKE_KEY, json.dumps(result))

    @unittest.skipUnless(HAS_SDK, "anthropic SDK not installed")
    def test_real_client_is_built_from_the_env_key_only(self):
        with mock.patch.dict(os.environ, {"ANTHROPIC_API_KEY": FAKE_KEY}, clear=True):
            client = gs.make_client(12.0)
        self.assertEqual(client.api_key, FAKE_KEY)
        self.assertEqual(client.max_retries, 2)


if __name__ == "__main__":
    unittest.main()
