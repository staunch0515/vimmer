"""Tests for edit_timeline.py. No network: API replies are hand-made message dicts.

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
import edit_timeline as et  # noqa: E402
from test_generate_storyboard import FakeClient  # noqa: E402

# Not shaped like a real key on purpose.
TEST_KEY = "test-key-not-real"
PROMPT = os.path.join(os.path.dirname(HERE), "..", "prompts", "edit.md")
EXAMPLE = os.path.join(os.path.dirname(HERE), "..", "examples", "zh-xian.timeline.json")

REPLY = {
    "status": "edited",
    "explanation": "第二段缩短到 3 秒。",
    "patch": [{"op": "replace", "path": "/scenes/1/duration_ms", "from": "", "value_json": "3000"}],
}


def message(text, model="claude-opus-5-5", stop="end_turn"):
    return {
        "model": model,
        "stop_reason": stop,
        "content": [{"type": "thinking", "thinking": ""}, {"type": "text", "text": text}],
        "usage": {
            "input_tokens": 300,
            "output_tokens": 200,
            "cache_creation_input_tokens": 0,
            "cache_read_input_tokens": 2500,
        },
    }


def timeline():
    with open(EXAMPLE, encoding="utf-8") as f:
        return json.load(f)


def request(**extra):
    req = {
        "timeline": timeline(),
        "instruction": "第二段节奏快一点",
        "scene_id": None,
        "assets": ["assets/sample.png", "assets/design.png"],
        "model": "claude-opus-5-5",
        "effort": "low",
    }
    req.update(extra)
    return req


def run_main(req, client, env_key=TEST_KEY, record=None):
    out = io.StringIO()
    with tempfile.TemporaryDirectory() as d:
        path = os.path.join(d, "req.json")
        with open(path, "w", encoding="utf-8") as f:
            json.dump(req, f, ensure_ascii=False)
        argv = ["--request", path, "--prompt-file", PROMPT]
        if record:
            argv += ["--record", os.path.join(d, record)]
        env = {"ANTHROPIC_API_KEY": env_key} if env_key else {}
        with mock.patch.dict(os.environ, env, clear=True), mock.patch("sys.stderr", io.StringIO()):
            code = et.main(argv, client_factory=lambda timeout: client, out=out)
        recorded = None
        if record:
            with open(os.path.join(d, record), encoding="utf-8") as f:
                recorded = json.load(f)
    return code, json.loads(out.getvalue()), recorded


class EditHelperTest(unittest.TestCase):
    def test_ok_reply_is_passed_through_with_usage_and_cost(self):
        client = FakeClient(message(json.dumps(REPLY, ensure_ascii=False)))
        code, result, recorded = run_main(request(), client, record="r.json")
        self.assertEqual(code, 0)
        self.assertTrue(result["ok"])
        self.assertEqual(json.loads(result["reply_json"]), REPLY)
        self.assertNotIn("storyboard_json", result)
        # 300 in * 4 + 2500 cache read * 0.2 + 200 out * 20, per million.
        self.assertAlmostEqual(result["usage"]["cost_usd"], (1200 + 500 + 4000) / 1e6, places=7)
        self.assertEqual(recorded["model"], "claude-opus-5-5")

    def test_request_shape(self):
        client = FakeClient(message(json.dumps(REPLY)))
        run_main(request(scene_id="scene-02"), client)
        params = client.calls[0]
        self.assertEqual(params["model"], "claude-opus-5-5")
        self.assertEqual(params["thinking"], {"type": "adaptive"})
        self.assertEqual(params["output_config"]["effort"], "low")
        self.assertEqual(params["output_config"]["format"]["schema"], et.EDIT_SCHEMA)
        self.assertEqual(params["fallbacks"], "default")
        system = params["system"]
        self.assertEqual(system[0]["cache_control"], {"type": "ephemeral"})
        self.assertNotIn("# Timeline editing prompt", system[0]["text"])
        self.assertIn("JSON Patch", system[0]["text"])
        user = params["messages"][0]["content"]
        self.assertIn("Selected scene: scene-02", user)
        self.assertIn("2. id=scene-02 path=/scenes/1", user)
        self.assertIn("- assets/design.png", user)
        self.assertIn("<instruction>\n第二段节奏快一点\n</instruction>", user)
        self.assertIn('"id": "scene-01"', user)

    def test_scene_table_lists_layers_by_index(self):
        t = timeline()
        t["scenes"][1]["layers"] = [
            {"id": "logo", "kind": "icon", "src": "assets/icons/star.svg", "x": 0, "y": 0, "width": 0.1, "height": 0.1},
            {"id": "note", "kind": "text", "text": "hi", "x": 0, "y": 0, "width": 0.5},
        ]
        lines, _ = et.scene_table(t)
        self.assertIn("layers=[0:logo(icon),1:note(text)]", lines[1])
        self.assertNotIn("layers=", lines[0])

    def test_scene_table_names_a_background_video(self):
        t = timeline()
        t["scenes"][0]["image"] = None
        t["scenes"][0]["video"] = {"src": "assets/clips/counter-6s.mp4", "speed": 2}
        lines, _ = et.scene_table(t)
        self.assertIn("video=assets/clips/counter-6s.mp4", lines[0])
        self.assertNotIn("video=", lines[1])

    def test_system_prompt_is_identical_across_requests(self):
        a, b = FakeClient(message("{}")), FakeClient(message("{}"))
        run_main(request(), a)
        run_main(request(instruction="switch to vertical", scene_id="scene-03"), b)
        self.assertEqual(a.calls[0]["system"], b.calls[0]["system"])

    def test_history_turns_carry_the_validator_error(self):
        client = FakeClient(message(json.dumps(REPLY)))
        hist = [{"assistant": '{"status":"edited"}', "feedback": "op 0 (add /x): `x` is not a field"}]
        run_main(request(history=hist), client)
        msgs = client.calls[0]["messages"]
        self.assertEqual([m["role"] for m in msgs], ["user", "assistant", "user"])
        self.assertIn("`x` is not a field", msgs[2]["content"])
        self.assertIn("original timeline", msgs[2]["content"])

    def test_haiku_has_no_fallback(self):
        client = FakeClient(message(json.dumps(REPLY), model="claude-haiku-5-5"))
        run_main(request(model="claude-haiku-5-5"), client)
        self.assertNotIn("fallbacks", client.calls[0])

    def test_no_key_and_bad_requests(self):
        code, result, _ = run_main(request(), FakeClient(message("{}")), env_key=None)
        self.assertEqual((code, result["error"]["kind"]), (1, "no_api_key"))
        code, result, _ = run_main(request(instruction="  "), FakeClient(message("{}")))
        self.assertEqual(result["error"]["kind"], "usage")
        code, result, _ = run_main(request(effort="huge"), FakeClient(message("{}")))
        self.assertEqual(result["error"]["kind"], "usage")
        req = request()
        del req["timeline"]
        code, result, _ = run_main(req, FakeClient(message("{}")))
        self.assertEqual(result["error"]["kind"], "usage")

    def test_refusal_and_max_tokens_are_failures(self):
        refusal = message("", stop="refusal")
        refusal["stop_details"] = {"category": "cyber"}
        code, result, _ = run_main(request(), FakeClient(refusal))
        self.assertEqual((code, result["error"]["kind"]), (1, "refusal"))
        code, result, _ = run_main(request(), FakeClient(message("{", stop="max_tokens")))
        self.assertEqual(result["error"]["kind"], "max_tokens")

    def test_key_never_appears_in_output(self):
        client = FakeClient(message(json.dumps(REPLY)))
        _, result, _ = run_main(request(), client)
        self.assertNotIn(TEST_KEY, json.dumps(result))

    def test_schema_is_strict(self):
        def walk(node):
            if node.get("type") == "object":
                self.assertIs(node.get("additionalProperties"), False)
                self.assertEqual(sorted(node["required"]), sorted(node["properties"]))
                for child in node["properties"].values():
                    walk(child)
            if node.get("type") == "array":
                walk(node["items"])

        walk(et.EDIT_SCHEMA)


if __name__ == "__main__":
    unittest.main()
