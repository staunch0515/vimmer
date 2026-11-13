"""Tests for illustrate.py. No network: API replies are hand-made message dicts.

    ~/.local/share/vimmer/llm-venv/bin/python -m unittest discover -s \
        rust-core/crates/vimmer-storyboard/scripts/tests -v
"""

import io
import json
import os
import sys
import tempfile
import unittest
from unittest import mock

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, os.path.dirname(HERE))
import illustrate as il  # noqa: E402
from test_generate_storyboard import FAKE_KEY, FakeClient  # noqa: E402

STYLE = {
    "name": "Dusk",
    "palette": ["#141a2e", "#d9a441"],
    "background": "gradient",
    "line": "none",
    "shapes": "flat",
    "composition": "upper half",
    "mood": "calm",
    "motifs": "moon",
}


def message(text, model="claude-opus-5-5", stop="end_turn", out=3000):
    return {
        "model": model,
        "stop_reason": stop,
        "content": [{"type": "thinking", "thinking": ""}, {"type": "text", "text": text}],
        "usage": {"input_tokens": 1500, "output_tokens": out},
    }


def request(kind="scene", **extra):
    req = {
        "kind": kind,
        "model": "claude-opus-5-5",
        "effort": "medium",
        "title": "西安钟楼",
        "language": "zh",
        "orientation": "landscape",
        "width": 1920,
        "height": 1080,
    }
    if kind == "scene":
        req["style"] = STYLE
        req["scene"] = {"text": "西安钟楼，古城的中心。", "image_prompt": "黄昏时分的钟楼"}
    else:
        req["scenes"] = [{"text": "a", "image_prompt": "p1"}, {"text": "b", "image_prompt": ""}]
    req.update(extra)
    return req


def run_main(req, client, env_key=FAKE_KEY):
    with tempfile.TemporaryDirectory() as d:
        path = os.path.join(d, "req.json")
        with open(path, "w", encoding="utf-8") as f:
            json.dump(req, f, ensure_ascii=False)
        out = io.StringIO()
        env = {"ANTHROPIC_API_KEY": env_key} if env_key else {}
        with mock.patch.dict(os.environ, env, clear=True), mock.patch("sys.stderr", io.StringIO()):
            code = il.main(["--request", path], client_factory=lambda t: client, out=out)
    return code, json.loads(out.getvalue())


class Prompts(unittest.TestCase):
    def test_layout_bands_follow_the_canvas(self):
        land = il.layout({"width": 1920, "height": 1080})
        self.assertEqual(land["calm_y"], 594)
        self.assertEqual(land["text_band"], "45%")
        port = il.layout({"width": 1080, "height": 1920})
        self.assertEqual(port["calm_y"], 1114)
        self.assertEqual(port["margin"], 65)

    def test_scene_system_prompt_carries_canvas_style_and_rules(self):
        system = il.system_prompt(request())
        self.assertIn('viewBox="0 0 1920 1080"', system)
        self.assertIn("y > 594", system)
        self.assertIn("- palette: #141a2e, #d9a441", system)
        self.assertIn("- motifs: moon", system)
        for rule in ("script", "foreignObject", "#id", "No text"):
            self.assertIn(rule, system)
        self.assertNotIn("{", system.replace("{style}", ""))

    def test_style_prompt_lists_every_scene(self):
        req = request("style", orientation="portrait", width=1080, height=1920)
        system = il.system_prompt(req)
        self.assertIn("1080x1920 (portrait)", system)
        msgs = il.style_messages(req)
        self.assertIn("1. text: a\n   picture: p1", msgs[0]["content"])
        self.assertIn("picture: (none; picture the scene text)", msgs[0]["content"])

    def test_scene_prompt_falls_back_to_the_text_and_replays_history(self):
        req = request(
            scene={"text": "只有文字。", "image_prompt": " "},
            history=[{"assistant": "<svg>bad", "feedback": "no viewBox"}],
        )
        msgs = il.scene_messages(req)
        self.assertEqual([m["role"] for m in msgs], ["user", "assistant", "user"])
        self.assertIn("<picture>只有文字。</picture>", msgs[0]["content"])
        self.assertEqual(msgs[1]["content"], "<svg>bad")
        self.assertIn("no viewBox", msgs[2]["content"])

    def test_request_params(self):
        scene = il.request_params(request())
        self.assertEqual(scene["thinking"], {"type": "adaptive"})
        self.assertEqual(scene["output_config"], {"effort": "medium"})
        self.assertEqual(scene["max_tokens"], il.DEFAULT_MAX_TOKENS["scene"])
        self.assertEqual(scene["fallbacks"], "default")
        self.assertNotIn("temperature", scene)
        style = il.request_params(request("style", max_tokens=5000))
        self.assertEqual(style["output_config"]["format"]["type"], "json_schema")
        self.assertEqual(style["max_tokens"], 5000)
        haiku = il.request_params(request(model="claude-haiku-5-5"))
        self.assertNotIn("fallbacks", haiku)


class Replies(unittest.TestCase):
    def test_extract_svg_tolerates_fences_and_preambles(self):
        svg = '<svg viewBox="0 0 1 1"><rect/></svg>'
        self.assertEqual(il.extract_svg(svg), svg)
        self.assertEqual(il.extract_svg("Here:\n```svg\n" + svg + "\n```"), svg)
        self.assertIsNone(il.extract_svg("no picture"))
        self.assertIsNone(il.extract_svg('<svg viewBox="0 0 1 1"><rect/>'))

    def test_scene_reply_with_usage(self):
        result = il.interpret(message("```\n<svg viewBox='0 0 2 1'></svg>\n```"), request())
        self.assertTrue(result["ok"])
        self.assertEqual(result["svg"], "<svg viewBox='0 0 2 1'></svg>")
        self.assertAlmostEqual(result["usage"]["cost_usd"], (1500 * 4 + 3000 * 20) / 1e6)

    def test_incomplete_svg_is_returned_for_a_correction(self):
        result = il.interpret(message("<svg><rect"), request())
        self.assertEqual(result["svg"], "<svg><rect")

    def test_style_reply(self):
        result = il.interpret(message(json.dumps(STYLE)), request("style"))
        self.assertEqual(result["style"]["name"], "Dusk")
        with self.assertRaises(il.gs.Failure) as e:
            il.interpret(message("not json"), request("style"))
        self.assertEqual(e.exception.kind, "bad_reply")

    def test_refusal_and_max_tokens_keep_usage(self):
        refusal = message("", stop="refusal")
        refusal["stop_details"] = {"category": "cyber"}
        with self.assertRaises(il.gs.Failure) as e:
            il.interpret(refusal, request())
        self.assertEqual(e.exception.kind, "refusal")
        self.assertGreater(e.exception.usage["cost_usd"], 0)
        with self.assertRaises(il.gs.Failure) as e:
            il.interpret(message("<svg", stop="max_tokens"), request())
        self.assertEqual(e.exception.kind, "max_tokens")


class MainProgram(unittest.TestCase):
    def test_scene_success(self):
        client = FakeClient(message('<svg viewBox="0 0 1920 1080"><rect/></svg>'))
        code, result = run_main(request(), client)
        self.assertEqual(code, 0)
        self.assertEqual(result["kind"], "scene")
        self.assertTrue(result["svg"].startswith("<svg"))
        self.assertEqual(client.calls[0]["model"], "claude-opus-5-5")

    def test_sonnet_is_priced_as_sonnet(self):
        client = FakeClient(message("<svg></svg>", model="claude-sonnet-5-5"))
        _, result = run_main(request(model="claude-sonnet-5-5"), client)
        self.assertAlmostEqual(result["usage"]["cost_usd"], (1500 * 2 + 3000 * 10) / 1e6)

    def test_missing_key_fails_before_any_call(self):
        client = FakeClient(message("<svg></svg>"))
        code, result = run_main(request(), client, env_key=None)
        self.assertEqual(code, 1)
        self.assertEqual(result["error"]["kind"], "no_api_key")
        self.assertEqual(client.calls, [])

    def test_bad_requests(self):
        for bad in (
            request(kind="poster"),
            request(width=10),
            request(effort="extreme"),
            request(style=None),
            request("style", scenes=[]),
        ):
            code, result = run_main(bad, FakeClient(message("x")))
            self.assertEqual(code, 1, bad)
            self.assertEqual(result["error"]["kind"], "usage", bad)

    def test_output_never_contains_the_key(self):
        client = FakeClient(message("<svg></svg>"))
        with tempfile.TemporaryDirectory() as d:
            path = os.path.join(d, "req.json")
            rec = os.path.join(d, "rec.json")
            with open(path, "w") as f:
                json.dump(request(), f)
            out = io.StringIO()
            err = io.StringIO()
            with mock.patch.dict(os.environ, {"ANTHROPIC_API_KEY": FAKE_KEY}, clear=True), \
                    mock.patch("sys.stderr", err):
                il.main(["--request", path, "--record", rec], lambda t: client, out)
            with open(rec) as f:
                recorded = f.read()
        for text in (out.getvalue(), err.getvalue(), recorded):
            self.assertNotIn(FAKE_KEY, text)


if __name__ == "__main__":
    unittest.main()
