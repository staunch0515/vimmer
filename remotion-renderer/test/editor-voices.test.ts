import assert from "node:assert/strict";
import { test } from "node:test";
import * as v from "../editor/src/voices.ts";
import { withVoice } from "../editor/src/narration.ts";
import { record, undo, type Undoable } from "../editor/src/history.ts";
import { FALLBACK_DEFAULTS, TTS_ENGINES } from "../editor/src/renderOptions.ts";
import type { Scene, VideoTimeline } from "../src/types/timeline.ts";

test("say voices and Kokoro voices are told apart like the render does", () => {
  for (const say of ["Samantha", "Tingting", "Kyoko", " Alex "]) assert.equal(v.isSayVoice(say), true, say);
  for (const k of ["zf_001", "af_heart", "jm_kumo", "", "  ", null, undefined]) assert.equal(v.isSayVoice(k), false, String(k));
  assert.equal(v.looksLikeKokoroVoice("zf_001"), true);
  assert.equal(v.looksLikeKokoroVoice("Zf_001"), false);
});

test("换成 Kokoro maps an old say voice by language and gender", () => {
  assert.equal(v.sayToKokoro("Tingting", "你好"), "zf_001");
  assert.equal(v.sayToKokoro("Li-mu", "你好"), "zm_009");
  assert.equal(v.sayToKokoro("Kyoko", "こんにちは"), "jf_alpha");
  assert.equal(v.sayToKokoro("Otoya", null), "jm_kumo");
  assert.equal(v.sayToKokoro("Samantha", "Hello"), "af_heart");
  assert.equal(v.sayToKokoro("Alex", "Hello"), "am_michael");
  assert.equal(v.sayToKokoro("Daniel", "Hello"), "bm_george");
  assert.equal(v.sayToKokoro("Serena", "Hello"), "bf_emma");
  // Unknown say voice: the default voice of the language the scene speaks.
  assert.equal(v.sayToKokoro("Zarvox", "西安钟楼"), "zf_001");
  assert.equal(v.sayToKokoro("Zarvox", "桜が咲きました"), "jf_alpha");
  assert.equal(v.sayToKokoro("Zarvox", "Hi"), "af_heart");
  // Every mapped voice is one the picker lists, and every listed voice is shaped like Kokoro's.
  for (const name of ["Tingting", "Li-mu", "Kyoko", "Otoya", "Samantha", "Alex", "Daniel", "Serena"]) {
    assert.ok(v.kokoroVoice(v.sayToKokoro(name, "")), name);
  }
  assert.ok(v.KOKORO_VOICES.every((k) => v.looksLikeKokoroVoice(k.id) && k.id[0] === { zh: "z", en: k.id[0] === "b" ? "b" : "a", ja: "j" }[k.lang]));
});

test("the picker groups voices by language; samples use one cached scene per voice", () => {
  const langs = new Set(v.KOKORO_VOICES.map((k) => k.lang));
  assert.deepEqual([...langs].sort(), ["en", "ja", "zh"]);
  for (const l of langs) assert.ok(v.KOKORO_VOICES.some((k) => k.lang === l && k.gender === "m") && v.KOKORO_VOICES.some((k) => k.lang === l && k.gender === "f"), l);
  for (const l of ["zh", "en", "ja"] as const) assert.ok(v.kokoroVoice(v.DEFAULT_KOKORO[l]), l);
  const t = v.sampleTimeline(v.kokoroVoice("zm_010")!);
  assert.equal(t.scenes[0].id, "voice-sample-zm-010");
  assert.equal(t.scenes[0].narration?.voice, "zm_010");
  assert.equal(v.textLang(t.scenes[0].text), "zh");
  assert.equal(v.textLang(v.SAMPLE_TEXT.ja), "ja");
  assert.equal(v.textLang(v.SAMPLE_TEXT.en), "en");
});

test("Kokoro is the editor's default engine", () => {
  assert.equal(FALLBACK_DEFAULTS.tts, "kokoro");
  assert.equal(TTS_ENGINES[0].value, "kokoro");
});

test("换成 Kokoro is one undo step and keeps the spoken line", () => {
  type S = Undoable<VideoTimeline>;
  const same = (a: VideoTimeline, b: VideoTimeline) => JSON.stringify(a) === JSON.stringify(b);
  const scene: Scene = {
    id: "a", duration_ms: 3000, text: "Hello", highlights: [], image: null, motion: "static", audio: null,
    narration: { voice: "Samantha", text: "Said line" },
  };
  const t: VideoTimeline = { fps: 30, resolution: { width: 1920, height: 1080 }, scenes: [scene], subtitles: [] };
  assert.deepEqual(v.sayVoiceScenes(t), [0]);
  const n = withVoice(scene.narration!, v.sayToKokoro("Samantha", "Said line"));
  const next: VideoTimeline = { ...t, scenes: [{ ...scene, narration: n }] };
  assert.deepEqual(next.scenes[0].narration, { voice: "af_heart", text: "Said line" });
  assert.deepEqual(v.sayVoiceScenes(next), []);
  let s: S = { timeline: t, history: [], future: [] };
  s = record(s, { ...s, timeline: next }, null, same);
  assert.equal(s.history.length, 1);
  assert.deepEqual(undo(s)!.timeline, t);
});
