import assert from "node:assert/strict";
import { test } from "node:test";
import { mkdtempSync, readdirSync, readFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import * as rec from "../editor/src/recording.ts";
import * as vo from "../editor/src/voiceover.ts";
import { recordingName, safeStem, saveTake, takeProblem } from "../editor/assistant-server.ts";
import type { Scene, VideoTimeline } from "../src/types/timeline.ts";

const scene = (id: string, ms: number, p: Partial<Scene> = {}): Scene => ({
  id, duration_ms: ms, text: `${id} text`, highlights: [], image: null, motion: "static", audio: null, ...p,
});

test("a take is a 16-bit mono 48 kHz WAV of the samples", () => {
  const samples = new Float32Array([0, 0.5, -0.5, 1, -1, 2, -2]);
  const wav = rec.encodeWav(samples);
  assert.equal(wav.length, 44 + samples.length * 2);
  assert.deepEqual(rec.wavInfo(wav), { rate: 48000, channels: 1, bits: 16, ms: 0 });
  const v = new DataView(wav.buffer);
  assert.deepEqual([1, 2, 3, 4, 5, 6].map((i) => v.getInt16(44 + i * 2, true)), [16384, -16384, 32767, -32768, 32767, -32768]);
  const second = rec.encodeWav(new Float32Array(48000));
  assert.equal(rec.wavInfo(second)?.ms, 1000);
  assert.equal(rec.wavInfo(new Uint8Array(10)), null);
  assert.equal(takeProblem(Buffer.from(second)), null, "the dev server accepts what the editor makes");
});

test("resampling keeps the length in time; levels and the meter", () => {
  const tone = new Float32Array(44100).map((_, i) => Math.sin((2 * Math.PI * 440 * i) / 44100) * 0.5);
  const out = rec.resample(tone, 44100, 48000);
  assert.equal(out.length, 48000);
  assert.ok(Math.abs(Math.max(...out) - 0.5) < 0.01);
  assert.equal(rec.resample(tone, 48000, 48000), tone);
  const lv = rec.levels(tone);
  assert.ok(Math.abs(lv.peakDb - -6.02) < 0.05, `${lv.peakDb}`);
  assert.ok(Math.abs(lv.rmsDb - -9.03) < 0.05, `${lv.rmsDb}`);
  assert.equal(rec.levels(new Float32Array(10)).peakDb, Number.NEGATIVE_INFINITY);
  assert.equal(rec.meterFill(Number.NEGATIVE_INFINITY), 0);
  assert.equal(rec.meterFill(-30), 0.5);
  assert.equal(rec.meterFill(3), 1);
  assert.deepEqual(rec.joinBlocks([new Float32Array([1, 2]), new Float32Array([3])]), new Float32Array([1, 2, 3]));
});

test("names and scripts", () => {
  assert.equal(rec.recordingStem("scene 03"), "scene-03");
  assert.equal(rec.recordingStem("开场"), "take");
  assert.equal(safeStem("../x"), "x");
  assert.equal(recordingName("a", (n) => n === "a-1.wav"), "a-2.wav");
  assert.equal(rec.scriptOf(scene("a", 1000)), "a text");
  assert.equal(rec.scriptOf(scene("a", 1000, { narration: { voice: null, text: " 台词 " } })), "台词");
  assert.equal(rec.scriptOf(scene("a", 1000, { text: null })), "");
});

test("the teleprompter's lines and the line being read", () => {
  const t: VideoTimeline = { fps: 30, resolution: { width: 1920, height: 1080 }, subtitles: [], scenes: [scene("a", 2000), scene("b", 3000), scene("c", 1000)] };
  const all = rec.prompterLines(t);
  assert.deepEqual(all.map((l) => [l.id, l.startMs, l.endMs]), [["a", 0, 2000], ["b", 2000, 5000], ["c", 5000, 6000]]);
  const later = rec.prompterLines(t, 2500);
  assert.deepEqual(later.map((l) => l.id), ["b", "c"]);
  assert.equal(rec.currentLine(all, 0, 0), 0);
  assert.equal(rec.currentLine(all, 0, 2000), 1);
  assert.equal(rec.currentLine(later, 2500, 2600), 1);
  assert.equal(rec.currentLine(all, 0, 99_000), 2);
});

test("keeping a teleprompter take: placed at its start, overlaps refused or replaced", () => {
  const t: VideoTimeline = {
    fps: 30, resolution: { width: 1920, height: 1080 }, subtitles: [], scenes: [scene("a", 5000), scene("b", 5000)],
    voiceover: [{ id: "vo-1", source: "old.wav", start_ms: 6000 }],
  };
  const L = { "old.wav": 2000 };
  const fits = vo.placeTake(t, "assets/recordings/voiceover-1.wav", 0, 5000, L, false);
  assert.ok(!("error" in fits));
  assert.deepEqual(fits.timeline.voiceover?.map((c) => [c.id, c.start_ms]), [["vo-2", 0], ["vo-1", 6000]]);
  const refused = vo.placeTake(t, "new.wav", 1000, 6000, L, false);
  assert.ok("error" in refused && refused.error.includes("“vo-1”"));
  const replaced = vo.placeTake(t, "new.wav", 1000, 6000, L, true);
  assert.ok(!("error" in replaced));
  assert.deepEqual(replaced.timeline.voiceover, [{ id: "vo-2", source: "new.wav", start_ms: 1000 }]);
  assert.deepEqual(replaced.replaced, ["vo-1"]);
  assert.ok("error" in vo.placeTake(t, "x.wav", 10_000, 100, L, false));
});

test("the dev server stores takes without overwriting and refuses other formats", () => {
  const dir = mkdtempSync(join(tmpdir(), "vimmer-rec-"));
  try {
    const wav = Buffer.from(rec.encodeWav(new Float32Array(4800)));
    assert.deepEqual(saveTake(dir, "intro", wav), { path: "assets/recordings/intro-1.wav" });
    assert.deepEqual(saveTake(dir, "intro", wav), { path: "assets/recordings/intro-2.wav" });
    assert.deepEqual(readFileSync(join(dir, "assets/recordings/intro-1.wav")), wav);
    const other = Buffer.from(rec.encodeWav(new Float32Array(4800), 44100));
    assert.deepEqual(saveTake(dir, "x", other), { error: "The recording must be 16-bit PCM, mono, 48 kHz." });
    assert.deepEqual(saveTake(dir, "x", Buffer.from("nope")), { error: "The recording is not a WAV file." });
    assert.deepEqual(readdirSync(join(dir, "assets/recordings")).sort(), ["intro-1.wav", "intro-2.wav"]);
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
});
