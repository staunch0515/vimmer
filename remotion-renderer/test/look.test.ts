// Atmosphere look: inheritance, mood map, grade math and deterministic particles (motion/look.ts).
import assert from "node:assert/strict";
import { test } from "node:test";
import {
  GRADE_PRESETS,
  MAX_PARTICLES,
  applyMatrix,
  autoPreset,
  drawsAnything,
  effectiveLook,
  grainOpacity,
  grainSeed,
  gradeStyle,
  lookProblems,
  overlaySeed,
  particleCount,
  particlesAt,
  presetForMood,
  resolveAutoGrades,
} from "../src/motion/look.ts";
import type { Look, VideoTimeline } from "../src/types/timeline.ts";
import { readFileSync } from "node:fs";

const timeline = (p: Partial<VideoTimeline>): VideoTimeline =>
  ({ fps: 30, resolution: { width: 1920, height: 1080 }, scenes: [], subtitles: [], ...p }) as VideoTimeline;

test("a scene part replaces the default part; a missing part is inherited", () => {
  const def: Look = { grade: { preset: "warm", strength: 0.6 }, overlays: [{ kind: "dust" }, { kind: "film_grain" }] };
  assert.deepEqual(effectiveLook(def, undefined), { grade: def.grade, overlays: def.overlays, inherited: true });
  const g = effectiveLook(def, { grade: { preset: "noir" } });
  assert.equal(g.grade?.preset, "noir");
  assert.equal(g.overlays.length, 2);
  assert.ok(g.inherited);
  const o = effectiveLook(def, { overlays: [{ kind: "snow" }] });
  assert.equal(o.grade?.preset, "warm");
  assert.deepEqual(o.overlays, [{ kind: "snow" }]);
  assert.ok(!o.inherited);
  assert.deepEqual(effectiveLook(def, { overlays: [] }).overlays, [], "an empty list means none here");
  assert.deepEqual(effectiveLook(null, null), { grade: null, overlays: [], inherited: true });
});

test("the mood map covers every library mood and matches the Rust table", () => {
  const lib = JSON.parse(readFileSync(new URL("../public/assets/music/library/library.json", import.meta.url), "utf8"));
  for (const t of lib.tracks) for (const m of t.moods) assert.notEqual(presetForMood(m), "none", m);
  assert.equal(presetForMood(" Warm "), "warm");
  assert.equal(presetForMood("calm"), "vintage");
  assert.equal(presetForMood("tense"), "moody");
  assert.equal(presetForMood("upbeat"), "vivid");
  assert.equal(presetForMood("epic"), "tech_blue");
  assert.equal(presetForMood("polka"), "none");
  assert.equal(presetForMood(undefined), "none");
  assert.equal(autoPreset(timeline({ music: { pick: { mood: "happy" }, volume: 0.3, fade_in_ms: 0, fade_out_ms: 0 } })), "vivid");
  assert.equal(autoPreset(timeline({ music: { source: "m.mp3", volume: 0.3, fade_in_ms: 0, fade_out_ms: 0 } })), "none");
});

test("auto grades resolve before the pick is dropped; timelines without auto are untouched", () => {
  const plain = timeline({ scenes: [{ id: "a" } as never] });
  assert.equal(resolveAutoGrades(plain), plain, "same object");
  const t = timeline({
    look: { grade: { preset: "auto", strength: 0.5 }, overlays: [{ kind: "snow" }] },
    scenes: [{ id: "a", look: { grade: { preset: "auto" } } } as never, { id: "b", look: { grade: { preset: "noir" } } } as never],
    music: { pick: { mood: "tense" }, volume: 0.3, fade_in_ms: 0, fade_out_ms: 0 },
  });
  const r = resolveAutoGrades(t);
  assert.deepEqual(r.look, { grade: { preset: "moody", strength: 0.5 }, overlays: [{ kind: "snow" }] });
  assert.equal(r.scenes[0].look?.grade?.preset, "moody");
  assert.equal(r.scenes[1], t.scenes[1]);
  assert.equal(t.look?.grade?.preset, "auto", "the input is not changed");
});

test("grades: none, auto without a mood and strength 0 draw nothing; strength scales toward identity", () => {
  assert.equal(gradeStyle(null), null);
  assert.equal(gradeStyle({ preset: "none" }), null);
  assert.equal(gradeStyle({ preset: "auto" }), null);
  assert.equal(gradeStyle({ preset: "warm", strength: 0 }), null);
  assert.equal(gradeStyle({ preset: "bogus" }), null);
  assert.ok(gradeStyle({ preset: "auto" }, "warm"));
  for (const p of GRADE_PRESETS) {
    if (p === "none" || p === "auto") continue;
    const g = gradeStyle({ preset: p })!;
    assert.equal(g.matrix.length, 20, p);
    // Alpha passes through.
    assert.deepEqual(g.matrix.slice(15), [0, 0, 0, 1, 0], p);
  }
  // Noir: no color left.
  const [r, g, b] = applyMatrix(gradeStyle({ preset: "noir" })!.matrix, [0.8, 0.3, 0.2]);
  assert.ok(Math.abs(r - g) < 1e-6 && Math.abs(g - b) < 1e-6);
  // Warm: warmer than the input; tech_blue: cooler.
  const warm = applyMatrix(gradeStyle({ preset: "warm" })!.matrix, [0.5, 0.5, 0.5]);
  assert.ok(warm[0] > warm[2]);
  const cool = applyMatrix(gradeStyle({ preset: "tech_blue" })!.matrix, [0.5, 0.5, 0.5]);
  assert.ok(cool[2] > cool[0]);
  // Half strength sits halfway.
  const full = gradeStyle({ preset: "vivid" })!.matrix;
  const half = gradeStyle({ preset: "vivid", strength: 0.5 })!.matrix;
  const id = [1, 0, 0, 0, 0, 0, 1, 0, 0, 0, 0, 0, 1, 0, 0, 0, 0, 0, 1, 0];
  half.forEach((v, i) => assert.ok(Math.abs(v - (id[i] + full[i]) / 2) < 1e-5, `${i}`));
  assert.equal(gradeStyle({ preset: "noir", strength: 0.5 })!.vignette, gradeStyle({ preset: "noir" })!.vignette / 2);
});

test("drawsAnything is false for looks that change nothing (the scene keeps its old DOM)", () => {
  assert.ok(!drawsAnything(effectiveLook(null, null), "none"));
  assert.ok(!drawsAnything(effectiveLook({ grade: { preset: "none" }, overlays: [{ kind: "snow", intensity: 0 }] }, null), "none"));
  assert.ok(!drawsAnything(effectiveLook({ grade: { preset: "auto" } }, null), "none"));
  assert.ok(drawsAnything(effectiveLook({ grade: { preset: "auto" } }, null), "warm"));
  assert.ok(drawsAnything(effectiveLook(null, { overlays: [{ kind: "film_grain" }] }), "none"));
});

test("particles are a pure function of the absolute frame: same frame, same output; chunk offsets do not matter", () => {
  const seed = overlaySeed({ kind: "snow" }, 0, true, "s1");
  for (const kind of ["snow", "dust", "bokeh"] as const) {
    const a = particlesAt(kind, 0.6, seed, 437, 1920, 1080);
    const b = particlesAt(kind, 0.6, seed, 437, 1920, 1080);
    assert.deepEqual(a, b, kind);
    assert.equal(a.length, particleCount(kind, 0.6));
    // A chunk starting at frame 300 draws local frame 137 as absolute 300 + 137: the same picture.
    const chunkStart = 300;
    assert.deepEqual(particlesAt(kind, 0.6, seed, chunkStart + 137, 1920, 1080), a, `${kind} chunk`);
    // And it moves: another frame is another picture.
    assert.notDeepEqual(particlesAt(kind, 0.6, seed, 438, 1920, 1080), a, `${kind} moves`);
    for (const p of a) {
      assert.ok(Number.isFinite(p.x) && Number.isFinite(p.y) && p.r > 0 && p.opacity > 0 && p.opacity <= 1, kind);
    }
  }
  // Late frames stay finite and on (or just off) the frame.
  for (const p of particlesAt("snow", 1, seed, 5999, 1920, 1080)) assert.ok(p.x > -200 && p.x < 2120 && p.y > -200 && p.y < 1280);
  assert.equal(grainSeed(seed, 10), grainSeed(seed, 10));
  assert.notEqual(grainSeed(seed, 10), grainSeed(seed, 11));
});

test("particle counts are capped, zero at intensity 0; grain opacity grows with intensity", () => {
  assert.equal(particleCount("snow", 0), 0);
  assert.equal(particleCount("snow", 1), MAX_PARTICLES.snow);
  assert.equal(particleCount("dust", 5), MAX_PARTICLES.dust);
  assert.equal(particleCount("bokeh", 1), MAX_PARTICLES.bokeh);
  assert.equal(grainOpacity(0), 0);
  assert.ok(grainOpacity(1) > grainOpacity(0.3));
  assert.ok(grainOpacity(1) <= 0.25);
});

test("seeds: default overlays share one seed in every scene; a scene's own overlays get their own", () => {
  const o = { kind: "dust" as const };
  assert.equal(overlaySeed(o, 0, true, "a"), overlaySeed(o, 0, true, "b"));
  assert.notEqual(overlaySeed(o, 0, false, "a"), overlaySeed(o, 0, false, "b"));
  assert.notEqual(overlaySeed(o, 0, true, "a"), overlaySeed(o, 1, true, "a"));
});

test("look problems mirror the Rust validation", () => {
  assert.deepEqual(lookProblems({ grade: { preset: "auto", strength: 0 }, overlays: [{ kind: "dust", color: "#fff" }] }), []);
  const p = lookProblems({
    grade: { preset: "sepia", strength: 2 },
    overlays: [{ kind: "rain" as never }, { kind: "snow", intensity: -1 }, { kind: "snow" }, { kind: "film_grain", color: "#fff" }, { kind: "dust", color: "red" }],
  }).join("\n");
  for (const want of ["`sepia` is unknown", "strength 2", "5 overlays", "`rain` is unknown", "intensity -1", "`snow` is listed twice", "`film_grain` has no color", "color `red`"]) {
    assert.ok(p.includes(want), want);
  }
});
