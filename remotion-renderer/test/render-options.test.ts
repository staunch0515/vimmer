import assert from "node:assert/strict";
import { test } from "node:test";
import {
  DEFAULT_OPTIONS,
  FALLBACK_DEFAULTS,
  blocksRender,
  findingSelection,
  hasOptions,
  loadOptions,
  optionsKey,
  parseOptions,
  pointerTokens,
  saveOptions,
  sortFindings,
  summaryText,
  toPanelArgs,
  type CheckFinding,
  type CheckReport,
} from "../editor/src/renderOptions.ts";
import type { VideoTimeline } from "../src/types/timeline.ts";

const timeline = {
  fps: 30,
  resolution: { width: 1920, height: 1080 },
  scenes: [
    { id: "intro", duration_ms: 3000 },
    { id: "body", duration_ms: 4000, layers: [{ id: "t" }, { id: "u" }], keyframes: [{ at_ms: 0 }] },
  ],
  subtitles: [{ start_ms: 0, end_ms: 1000, text: "hi" }],
  music: { source: "a.mp3" },
} as unknown as VideoTimeline;

const finding = (severity: CheckFinding["severity"], path?: string, extra: Partial<CheckFinding> = {}): CheckFinding => ({
  severity,
  code: "x.y",
  message: "m",
  suggestion: { text: "fix it", ...(path !== undefined ? { path } : {}) },
  ...extra,
});

const report = (findings: CheckFinding[]): CheckReport => ({
  schema: "vimmer-check/1",
  summary: {
    errors: findings.filter((f) => f.severity === "error").length,
    warnings: findings.filter((f) => f.severity === "warning").length,
    infos: findings.filter((f) => f.severity === "info").length,
  },
  findings,
});

test("a finding's pointer selects the layer, keyframe, scene, cue or music", () => {
  const sel = (path: string) => findingSelection(finding("error", path), timeline);
  assert.deepEqual(sel("/scenes/1/layers/1/x"), { kind: "layer", sceneIndex: 1, index: 1 });
  assert.deepEqual(sel("/scenes/1/layers/0/keyframes/0/at_ms"), { kind: "layer", sceneIndex: 1, index: 0 });
  assert.deepEqual(sel("/scenes/1/keyframes/0"), { kind: "keyframe", sceneIndex: 1, index: 0 });
  assert.deepEqual(sel("/scenes/0/duration_ms"), { kind: "scene", index: 0 });
  assert.deepEqual(sel("/scenes/1/template/params/title"), { kind: "scene", index: 1 });
  assert.deepEqual(sel("/scenes/1/layers/9/x"), { kind: "scene", index: 1 }, "a missing layer falls back to its scene");
  assert.deepEqual(sel("/subtitles/0/text"), { kind: "subtitle", index: 0 });
  assert.deepEqual(sel("/subtitles/0"), { kind: "subtitle", index: 0 });
  assert.deepEqual(sel("/music/volume"), { kind: "music" });
  assert.equal(sel("/brand/colors/accent"), null);
  assert.equal(sel("/scenes/7/duration_ms"), null);
});

test("without a pointer, the scene id or cue index decides", () => {
  assert.deepEqual(findingSelection(finding("warning", undefined, { scene_id: "body" }), timeline), { kind: "scene", index: 1 });
  assert.deepEqual(findingSelection(finding("warning", undefined, { cue_index: 0 }), timeline), { kind: "subtitle", index: 0 });
  assert.equal(findingSelection(finding("info", undefined, { scene_id: "gone" }), timeline), null);
  assert.equal(findingSelection(finding("info"), timeline), null);
});

test("pointer tokens are unescaped", () => {
  assert.deepEqual(pointerTokens("/a~1b/c~0d"), ["a/b", "c~d"]);
  assert.deepEqual(pointerTokens(""), []);
});

test("only errors block the render", () => {
  assert.equal(blocksRender(report([finding("warning"), finding("info")])), false);
  assert.equal(blocksRender(report([finding("info"), finding("error")])), true);
  assert.equal(blocksRender(report([])), false);
});

test("findings list errors first, keeping the report's order within a severity", () => {
  const a = finding("info", "/a");
  const b = finding("error", "/b");
  const c = finding("warning", "/c");
  const d = finding("error", "/d");
  assert.deepEqual(sortFindings([a, b, c, d]).map((f) => f.suggestion.path), ["/b", "/d", "/c", "/a"]);
  assert.equal(summaryText(report([a, b, c, d])), "2 errors · 1 warning · 1 info");
  assert.equal(summaryText(report([])), "No problems found");
});

test("options are remembered per project and bad stored values fall back", () => {
  const map = new Map<string, string>();
  const store = { getItem: (k: string) => map.get(k) ?? null, setItem: (k: string, v: string) => void map.set(k, v) };
  assert.deepEqual(loadOptions(store, "/p/a.json"), DEFAULT_OPTIONS);
  saveOptions(store, "/p/a.json", { ...DEFAULT_OPTIONS, tts: "kokoro", autoSubtitles: "on" });
  assert.equal(loadOptions(store, "/p/a.json").tts, "kokoro");
  assert.equal(loadOptions(store, "/p/b.json").tts, null, "another project keeps its own");
  assert.ok(map.has(optionsKey("/p/a.json")));
  assert.equal(hasOptions(store, "/p/a.json"), true);
  assert.equal(hasOptions(store, "/p/b.json"), false);
  assert.equal(hasOptions(null, null), false);
  assert.deepEqual(
    parseOptions('{"tts":"espeak","autoSubtitles":"some","chunkMs":5,"renderJobs":0,"wordSubtitles":"yes"}'),
    DEFAULT_OPTIONS,
  );
  assert.deepEqual(parseOptions("not json"), DEFAULT_OPTIONS);
  assert.deepEqual(loadOptions(null, null), DEFAULT_OPTIONS);
});

test("the panel sends the default engine when none was picked", () => {
  assert.deepEqual(toPanelArgs(DEFAULT_OPTIONS, { ...FALLBACK_DEFAULTS, tts: "kokoro" }), {
    tts: "kokoro",
    autoSubtitles: "off",
    wordSubtitles: true,
    chunkMs: null,
    preprocessJobs: null,
    renderJobs: null,
    whisperLanguage: null,
  });
  assert.equal(toPanelArgs({ ...DEFAULT_OPTIONS, tts: "say", whisperLanguage: " ja " }, { ...FALLBACK_DEFAULTS, tts: "kokoro" }).tts, "say");
  assert.equal(toPanelArgs({ ...DEFAULT_OPTIONS, whisperLanguage: " ja " }, FALLBACK_DEFAULTS).whisperLanguage, "ja");
});
