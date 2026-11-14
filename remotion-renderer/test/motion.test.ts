// Run with: npm run test:motion  (Node >= 22.6, type stripping; no bundler involved)
import assert from "node:assert/strict";
import { test } from "node:test";
import { IDENTITY_POSE, applyEasing, sampleKeyframes } from "../src/motion/keyframes.ts";
import { transitionPose } from "../src/motion/transitions.ts";
import { animationPose, layerFrames, layerStateAt, splitLayers } from "../src/motion/layers.ts";
import { barProgress, chartMax, countFrames, formatNumber, frameProgress, ringFraction } from "../src/motion/data.ts";
import { lowerThirdPose } from "../src/motion/lowerThird.ts";
import { clipFrameAt, clipFrames, clipSourceMs, clipTimeAt } from "../src/motion/video.ts";
import { cursorVisible, typedCount, typedSegments, typingUnits } from "../src/motion/typewriter.ts";
import { sfxFrame } from "../src/motion/sfx.ts";
import type { Easing, Keyframe, Scene, ShapeLayer, TransitionKind } from "../src/types/timeline.ts";

const kf = (at_ms: number, extra: Partial<Keyframe> = {}): Keyframe => ({ at_ms, ...extra });
const close = (actual: number, expected: number) => assert.ok(Math.abs(actual - expected) < 1e-9, `${actual} != ${expected}`);

test("empty keyframes give the identity pose", () => {
  assert.deepEqual(sampleKeyframes([], 1234), IDENTITY_POSE);
});

test("holds the first pose before the first keyframe and the last pose after the last", () => {
  const keys = [kf(1000, { scale: 1.5, x: 10, opacity: 0.5, rotation: 30 }), kf(3000, { scale: 2 })];
  assert.deepEqual(sampleKeyframes(keys, 0), { scale: 1.5, x: 10, y: 0, opacity: 0.5, rotation: 30 });
  assert.deepEqual(sampleKeyframes(keys, 1000), { scale: 1.5, x: 10, y: 0, opacity: 0.5, rotation: 30 });
  assert.deepEqual(sampleKeyframes(keys, 9000), { scale: 2, x: 0, y: 0, opacity: 1, rotation: 0 });
  assert.deepEqual(sampleKeyframes(keys, 3000), { scale: 2, x: 0, y: 0, opacity: 1, rotation: 0 });
});

test("omitted fields default to the identity pose", () => {
  const pose = sampleKeyframes([kf(0), kf(1000, { x: 20 })], 500);
  assert.equal(pose.scale, 1);
  assert.equal(pose.opacity, 1);
  close(pose.x, 10); // ease_in_out at t = 0.5 is exactly 0.5
});

test("linear segment interpolates every channel", () => {
  const keys = [
    kf(0, { scale: 1, x: 0, y: 0, opacity: 1, rotation: 0, ease: "linear" }),
    kf(2000, { scale: 2, x: 10, y: -20, opacity: 0, rotation: 90 }),
  ];
  const pose = sampleKeyframes(keys, 500);
  close(pose.scale, 1.25);
  close(pose.x, 2.5);
  close(pose.y, -5);
  close(pose.opacity, 0.75);
  close(pose.rotation, 22.5);
});

test("the easing of the earlier keyframe governs the segment that leaves it", () => {
  const keys = [kf(0, { ease: "ease_in" }), kf(1000, { scale: 2, ease: "linear" }), kf(2000, { scale: 3 })];
  // Segment 0->1 uses ease_in: at t = 0.5 progress is 0.25, so scale = 1 + 0.25.
  close(sampleKeyframes(keys, 500).scale, 1.25);
  // Segment 1->2 uses linear: at t = 0.5 scale is 2.5.
  close(sampleKeyframes(keys, 1500).scale, 2.5);
});

test("easing curves hit their endpoints and midpoint", () => {
  for (const ease of ["linear", "ease_in", "ease_out", "ease_in_out"] as const) {
    close(applyEasing(ease, 0), 0);
    close(applyEasing(ease, 1), 1);
  }
  close(applyEasing("ease_in", 0.5), 0.25);
  close(applyEasing("ease_out", 0.5), 0.75);
  close(applyEasing("ease_in_out", 0.5), 0.5);
  assert.ok(applyEasing("ease_in_out", 0.25) < 0.25);
  assert.ok(applyEasing("ease_in_out", 0.75) > 0.75);
});

const peak = (ease: Easing): number => {
  let m = -Infinity;
  for (let i = 0; i <= 1000; i++) m = Math.max(m, applyEasing(ease, i / 1000));
  return m;
};

test("springy easings start at 0, end at 1, and overshoot where expected", () => {
  for (const ease of ["back_out", "elastic_out", "bounce_out"] as const) {
    close(applyEasing(ease, 0), 0);
    close(applyEasing(ease, 1), 1);
    assert.equal(applyEasing(ease, -0.5), applyEasing(ease, 0)); // clamped
    assert.equal(applyEasing(ease, 1.5), applyEasing(ease, 1));
  }
  const back = peak("back_out");
  assert.ok(back > 1.09 && back < 1.11, `back_out peak ${back}`);
  const elastic = peak("elastic_out");
  assert.ok(elastic > 1.3 && elastic < 1.4, `elastic_out peak ${elastic}`);
  assert.ok(peak("bounce_out") <= 1 + 1e-9);
  close(applyEasing("bounce_out", 1 / 2.75), 1); // first landing
  assert.ok(applyEasing("bounce_out", 1.5 / 2.75) < 0.76); // top of the first bounce
  // Old curves are unchanged by the clamp inside [0, 1].
  close(applyEasing("ease_in_out", 0.3), 2 * 0.09);
});

test("a springy keyframe ease overshoots between keyframes", () => {
  const keys = [kf(0, { scale: 1, ease: "elastic_out" }), kf(1000, { scale: 2 })];
  assert.ok(Math.max(...[100, 150, 200].map((t) => sampleKeyframes(keys, t).scale)) > 2.2);
  close(sampleKeyframes(keys, 1000).scale, 2);
});

test("an enter/exit ease replaces the kind's curve; without one the pose is unchanged", () => {
  const W = 1920;
  const H = 1080;
  const springy = animationPose("pop", 0.15, "enter", W, H, (t) => applyEasing("elastic_out", t));
  assert.ok(springy.scale > 1, `elastic pop overshoots: ${springy.scale}`);
  assert.deepEqual(animationPose("slide_up", 0.4, "enter", W, H), animationPose("slide_up", 0.4, "enter", W, H, undefined));
  const shape: ShapeLayer = { id: "s", kind: "shape", x: 0, y: 0, width: 0.1, height: 0.1, fill: "#fff", enter: { kind: "slide_up", duration_ms: 1000 } };
  const plain = layerStateAt(shape, 10, 30, 90, W, H, undefined, applyEasing);
  const without = layerStateAt(shape, 10, 30, 90, W, H);
  assert.deepEqual(plain, without);
  const eased = layerStateAt({ ...shape, enter: { kind: "slide_up", duration_ms: 1000, ease: "linear" } }, 15, 30, 90, W, H, undefined, applyEasing);
  // Linear: halfway through the slide, half the distance is left and the opacity is 0.5.
  close(eased.opacity, 0.5);
});

test("exact keyframe times return that keyframe's values", () => {
  const keys = [kf(0, { scale: 1 }), kf(1000, { scale: 1.2 }), kf(4000, { scale: 1.1 })];
  close(sampleKeyframes(keys, 1000).scale, 1.2);
  close(sampleKeyframes(keys, 4000).scale, 1.1);
});

test("transition poses: progress 0 hides the incoming scene, progress 1 shows it in place", () => {
  const kinds: TransitionKind[] = ["fade", "slide_left", "slide_right", "slide_up", "slide_down", "zoom_in", "wipe_left"];
  for (const kind of kinds) {
    const end = transitionPose(kind, 1);
    assert.equal(end.opacity, 1, `${kind} ends opaque`);
    const restingTransform: Record<TransitionKind, string> = {
      fade: "none",
      slide_left: "translate3d(0%, 0, 0)",
      slide_right: "translate3d(0%, 0, 0)",
      slide_up: "translate3d(0, 0%, 0)",
      slide_down: "translate3d(0, 0%, 0)",
      zoom_in: "scale(1)",
      wipe_left: "none",
    };
    assert.equal(end.transform, restingTransform[kind], kind);
    assert.equal(end.clipPath, kind === "wipe_left" ? "inset(0 0 0 0%)" : "none");
  }
  assert.equal(transitionPose("fade", 0).opacity, 0);
  assert.equal(transitionPose("zoom_in", 0).opacity, 0);
  assert.equal(transitionPose("slide_left", 0).transform, "translate3d(100%, 0, 0)");
  assert.equal(transitionPose("slide_right", 0).transform, "translate3d(-100%, 0, 0)");
  assert.equal(transitionPose("slide_up", 0).transform, "translate3d(0, 100%, 0)");
  assert.equal(transitionPose("slide_down", 0).transform, "translate3d(0, -100%, 0)");
  assert.equal(transitionPose("wipe_left", 0).clipPath, "inset(0 0 0 100%)");
  assert.equal(transitionPose("fade", 0.5).opacity, 0.5);
});

test("transition progress is clamped", () => {
  assert.equal(transitionPose("fade", -1).opacity, 0);
  assert.equal(transitionPose("fade", 3).opacity, 1);
});

// ---- Layers (src/motion/layers.ts)

const shape = (extra: Partial<ShapeLayer> = {}): ShapeLayer => ({
  id: "s", kind: "shape", x: 0.25, y: 0.5, width: 0.5, height: 0.1, fill: "#fff", ...extra,
});

test("layer box: fractions of the frame, top-left anchor by default, center anchor shifts by half", () => {
  const s = layerStateAt(shape(), 0, 30, 90, 1920, 1080);
  assert.equal(s.visible, true);
  assert.deepEqual([s.left, s.top, s.width, s.height], [480, 540, 960, 108]);
  assert.equal(s.transform, "none");
  const c = layerStateAt(shape({ anchor: "center" }), 0, 30, 90, 1920, 1080);
  assert.equal(c.transform, "translate(-50%, -50%)");
});

test("layer timing: shown on [start, end) frames, default end is the scene end", () => {
  const l = shape({ start_ms: 1000, end_ms: 2000 });
  assert.deepEqual(layerFrames(l, 30, 90), { start: 30, end: 60 });
  assert.equal(layerStateAt(l, 29, 30, 90, 1920, 1080).visible, false);
  assert.equal(layerStateAt(l, 30, 30, 90, 1920, 1080).visible, true);
  assert.equal(layerStateAt(l, 59, 30, 90, 1920, 1080).visible, true);
  assert.equal(layerStateAt(l, 60, 30, 90, 1920, 1080).visible, false);
  assert.deepEqual(layerFrames(shape(), 30, 90), { start: 0, end: 90 });
});

test("enter starts hidden and ends at rest; exit ends hidden on the last frame", () => {
  const l = shape({ enter: { kind: "slide_up", duration_ms: 500 }, exit: { kind: "fade", duration_ms: 500 } });
  const first = layerStateAt(l, 0, 30, 90, 1920, 1080);
  assert.equal(first.opacity, 0);
  assert.match(first.transform, /translate\(0px, 64\.8px\)/); // 6% of 1080 below
  const rest = layerStateAt(l, 45, 30, 90, 1920, 1080);
  assert.equal(rest.opacity, 1);
  assert.equal(rest.transform, "none");
  assert.equal(layerStateAt(l, 89, 30, 90, 1920, 1080).opacity, 0);
  assert.ok(layerStateAt(l, 80, 30, 90, 1920, 1080).opacity > 0);
});

test("every animation kind enters from hidden to rest and exits from rest to hidden", () => {
  for (const kind of ["fade", "slide_up", "slide_down", "slide_left", "slide_right", "pop"] as const) {
    const done = animationPose(kind, 1, "enter", 1920, 1080);
    assert.deepEqual([done.dx, done.dy, done.opacity], [0, 0, 1], kind);
    close(done.scale, 1);
    assert.equal(animationPose(kind, 0, "enter", 1920, 1080).opacity, 0, kind);
    assert.deepEqual(animationPose(kind, 0, "exit", 1920, 1080), { dx: 0, dy: 0, scale: 1, opacity: 1 }, kind);
    assert.equal(animationPose(kind, 1, "exit", 1920, 1080).opacity, 0, kind);
  }
  assert.ok(animationPose("pop", 0.6, "enter", 1920, 1080).scale > 1, "pop overshoots");
});

test("keyframes offset in percent of the frame, scale and rotation compose with the layer", () => {
  const l = shape({ rotation: 10, opacity: 0.5, keyframes: [kf(0, { x: 10, y: -10, scale: 2, rotation: 5, opacity: 0.5 })] });
  const pose = sampleKeyframes(l.keyframes!, 0);
  const s = layerStateAt(l, 0, 30, 90, 1920, 1080, pose);
  assert.equal(s.transform, "translate(192px, -108px) rotate(15deg) scale(2)");
  close(s.opacity, 0.25);
});

test("stacking: z < 0 under the scene text, z >= 0 over it, list order for equal z", () => {
  const ids = (ls: { id: string }[]) => ls.map((l) => l.id);
  const { below, above } = splitLayers([
    shape({ id: "a", z: 1 }), shape({ id: "b" }), shape({ id: "c", z: -2 }), shape({ id: "d", z: -1 }), shape({ id: "e" }),
  ]);
  assert.deepEqual(ids(below), ["c", "d"]);
  assert.deepEqual(ids(above), ["b", "e", "a"]);
  assert.deepEqual(splitLayers(undefined), { below: [], above: [] });
});

test("a layer's state depends only on the scene frame (chunked renders match full ones)", () => {
  // Chunks start on scene cuts, so a scene's frames are the same numbers in both renders; the
  // state is a pure function of them. Spot-check determinism over every frame.
  const l = shape({ enter: { kind: "pop" }, exit: { kind: "slide_left" }, keyframes: [kf(0), kf(2000, { x: 20, ease: "linear" })] });
  for (let f = 0; f < 90; f++) {
    const a = layerStateAt(l, f, 30, 90, 1920, 1080, sampleKeyframes(l.keyframes!, (f * 1000) / 30));
    const b = layerStateAt(l, f, 30, 90, 1920, 1080, sampleKeyframes(l.keyframes!, (f * 1000) / 30));
    assert.deepEqual(a, b);
  }
});

test("typewriter units keep CJK characters, emoji, accents and flags whole", () => {
  assert.deepEqual(typingUnits("abc"), ["a", "b", "c"]);
  assert.deepEqual(typingUnits("你好，世界"), ["你", "好", "，", "世", "界"]);
  const s = "é👍🏽👨‍👩‍👧🇯🇵1️⃣!";
  const u = typingUnits(s);
  assert.equal(u.length, 6, JSON.stringify(u));
  assert.equal(u.join(""), s);
  assert.equal(u[3], "🇯🇵");
  assert.equal(typingUnits("🇯🇵🇫🇷").length, 2);
  // Astral CJK (surrogate pairs in UTF-16) is one unit per character.
  assert.deepEqual(typingUnits("𠮷野家"), ["𠮷", "野", "家"]);
});

test("typewriter count and cursor depend only on the layer frame", () => {
  assert.equal(typedCount(0, 30, 20, 10), 0);
  assert.equal(typedCount(3, 30, 20, 10), 2);
  assert.equal(typedCount(300, 30, 20, 10), 10);
  // 10 units at 20 cps are typed by frame 15; solid cursor until then, then blinking.
  assert.equal(cursorVisible(14, 30, 20, 10), true);
  assert.equal(cursorVisible(15, 30, 20, 10), true);
  assert.equal(cursorVisible(15 + 16, 30, 20, 10), false); // 533 ms after: off
  assert.equal(cursorVisible(15 + 32, 30, 20, 10), true);
});

test("typed segments never split a unit and put the cursor after the typed text", () => {
  const segs = typedSegments("Hi 👋🏽 世界", 3, (w) => w === "世界");
  assert.deepEqual(
    segs.map((s) => [s.shown, s.hidden, s.cursorAfter, s.highlight]),
    [
      ["Hi", "", false, false],
      [" ", "", true, false],
      ["", "👋🏽", false, false],
      ["", " ", false, false],
      ["", "世界", false, true],
    ],
  );
  const none = typedSegments("abc", 0, () => false);
  assert.equal(none[0].cursorAfter, true);
  assert.equal(none[0].shown, "");
});

test("lower third: built-in wipe starts hidden, rests fully shown, ends hidden; own enter/exit skip it", () => {
  const span = 120; // 4 s at 30 fps
  const first = lowerThirdPose(0, span, 30, true, true);
  assert.equal(first.reveal, 0);
  assert.equal(first.bar, 0);
  assert.equal(first.textOpacity, 0);
  const mid = lowerThirdPose(60, span, 30, true, true);
  assert.deepEqual(mid, { bar: 1, reveal: 1, textShift: 0, textOpacity: 1 });
  const last = lowerThirdPose(span - 1, span, 30, true, true);
  assert.equal(last.reveal, 0);
  assert.equal(last.textOpacity, 0);
  // With the layer's own enter and exit, frame 0 and the last frame are at rest.
  assert.deepEqual(lowerThirdPose(0, span, 30, false, false), mid);
  assert.deepEqual(lowerThirdPose(span - 1, span, 30, false, false), mid);
  // A short layer squeezes the built-in animation into 40 % / 30 % of its frames.
  const short = 10;
  assert.equal(lowerThirdPose(4, short, 30, true, true).reveal, 1);
  assert.equal(lowerThirdPose(short - 1, short, 30, true, true).reveal, 0);
});

test("numbers are formatted without the machine's locale", () => {
  assert.equal(formatNumber(1234567.891, 2, ",", ".", "$", ""), "$1,234,567.89");
  assert.equal(formatNumber(1234567, 0, " ", ",", "", " €"), "1 234 567 €");
  assert.equal(formatNumber(999, 0), "999");
  assert.equal(formatNumber(1000, 0), "1,000");
  assert.equal(formatNumber(-1500.26, 1, ".", ",", "", "%"), "-1.500,3%");
  assert.equal(formatNumber(-0.004, 2), "0.00");
  assert.equal(formatNumber(0, 0), "0");
  assert.equal(formatNumber(12345, 0, ""), "12345");
  assert.equal(formatNumber(1e15, 0), "1,000,000,000,000,000");
});

test("counts reach their end value on the last counting frame", () => {
  const counter = { id: "c", kind: "counter", x: 0, y: 0, width: 0.3, to: 100, exit: { kind: "fade", duration_ms: 500 } } as const;
  // 3 s layer at 30 fps with a 500 ms exit: 75 counting frames.
  assert.equal(countFrames(counter, undefined, 90, 30), 75);
  assert.equal(countFrames(counter, 1000, 90, 30), 30);
  assert.equal(frameProgress(0, 75), 0);
  assert.equal(frameProgress(74, 75), 1);
  assert.equal(frameProgress(200, 75), 1);
  assert.equal(frameProgress(0, 1), 1);
});

test("bars grow one after another and the chart scale is max or the largest value", () => {
  // 800 ms growth (24 frames), 120 ms stagger (4 frames) at 30 fps.
  assert.equal(barProgress(0, 0, 30, 800, 120), 0);
  assert.equal(barProgress(23, 0, 30, 800, 120), 1);
  assert.equal(barProgress(3, 1, 30, 800, 120), 0); // bar 1 has not started
  assert.equal(barProgress(4, 1, 30, 800, 120), 0);
  assert.equal(barProgress(27, 1, 30, 800, 120), 1);
  assert.ok(barProgress(10, 2, 30, 800, 120) < barProgress(10, 1, 30, 800, 120));
  assert.equal(chartMax([10, 40, 25], undefined), 40);
  assert.equal(chartMax([10, 40, 25], 100), 100);
  assert.equal(chartMax([0, 0], null), 1);
});

test("ring fraction goes from `from` to `value` and never past a full ring", () => {
  assert.equal(ringFraction(0.2, 0.8, 0), 0.2);
  close(ringFraction(0.2, 0.8, 1), 0.8);
  close(ringFraction(0, 0.5, 0.5), 0.25);
  assert.equal(ringFraction(0, 1, 1.37), 1); // elastic overshoot is clamped
  assert.equal(ringFraction(0.5, 0, 1.37), 0);
});

test("clip frames match the Rust clip_frames / clip_frame_at", () => {
  // Same cases as rust-core/crates/vimmer-dsl/src/layer.rs clip_frame_math.
  assert.equal(clipFrames(2000, 2, 30), 30);
  assert.equal(clipFrames(2000, 0.5, 30), 120);
  assert.equal(clipFrames(10, 4, 30), 1);
  assert.equal(clipFrameAt(0, 30, "freeze"), 0);
  assert.equal(clipFrameAt(29, 30, "freeze"), 29);
  assert.equal(clipFrameAt(75, 30, "freeze"), 29);
  assert.equal(clipFrameAt(75, 30, "loop"), 15);
  assert.equal(clipFrameAt(30, 30, "loop"), 0);
  assert.equal(clipSourceMs({ src: "a.mp4", trim_start_ms: 500 }, 6000), 5500);
  assert.equal(clipSourceMs({ src: "a.mp4", trim_start_ms: 500, trim_end_ms: 9000 }, 6000), 5500);
  assert.equal(clipSourceMs({ src: "a.mp4" }), null);
});

test("a normalized clip (trim 0..N frames, speed 1) shows exactly frame g of its file", () => {
  // The scheduler rewrites a 45-frame clip to trim_end_ms = round(45 * 1000 / 30) = 1500.
  for (const [n, fit] of [[45, "freeze"], [37, "loop"], [83, "freeze"]] as const) {
    const clip = { src: "generated/videos/k.mp4", trim_end_ms: Math.round((n * 1000) / 30), fit_time: fit };
    for (let f = 0; f < 120; f++) {
      const t = clipTimeAt(clip, f, 120, 30);
      const want = fit === "loop" ? f % n : Math.min(f, n - 1);
      assert.equal(t.clipFrame, want);
      assert.ok(Math.abs((t.fileMs * 30) / 1000 - want) < 1e-9, `${n} ${fit} ${f}`);
    }
  }
});

test("trim, speed and stretch map scene frames to file times", () => {
  // From 1.5 s at 2x: scene frame f shows file time 1.5 s + f * 2 / 30 s, held after 3 s of file.
  const clip = { src: "a.mp4", trim_start_ms: 1500, trim_end_ms: 4500, speed: 2 };
  close(clipTimeAt(clip, 10, 90, 30).fileMs, 1500 + (10 * 2000) / 30);
  assert.equal(clipTimeAt(clip, 80, 90, 30).clipFrame, 44); // 3 s / 2 = 45 frames, then frozen
  // Stretch: 3 s of file over a 60-frame (2 s) scene plays at 1.5x and ends on the last frame.
  const s = clipTimeAt({ ...clip, fit_time: "stretch" }, 59, 60, 30);
  close(s.speed, 1.5);
  assert.equal(s.clipFrame, 59);
  // Length unknown (editor, no trim_end, file length not loaded yet): plays straight on.
  assert.equal(clipTimeAt({ src: "a.mp4", fit_time: "loop" }, 200, 60, 30).clipFrame, 200);
  // Known file length: loops.
  assert.equal(clipTimeAt({ src: "a.mp4", fit_time: "loop" }, 70, 120, 30, 2000).clipFrame, 10);
});

// ---- Brand kit (src/brand.ts) -------------------------------------------------------------------

import { readFileSync } from "node:fs";
import { BASE_FONT_STACK, BUILTIN, brandColor, fontStack, highlightColors, logoBox, logoShownOn, withAlpha } from "../src/brand.ts";
import type { Brand } from "../src/types/timeline.ts";

test("without a brand every default is the original hardcoded value", () => {
  assert.equal(fontStack(null, "heading"), "Inter, 'Geist', system-ui, sans-serif");
  assert.equal(fontStack({ colors: {} }, "body"), BASE_FONT_STACK);
  assert.deepEqual(highlightColors(null, 0.5), { color: "#a5b4fc", textShadow: "0 0 12px rgba(99, 102, 241, 0.3)" });
  assert.equal(brandColor(undefined, "background", BUILTIN.background), "#09090b");
  assert.equal(brandColor(null, "subtitle_background", BUILTIN.subtitleBackground), "rgba(9, 9, 11, 0.72)");
  assert.equal(logoBox(null, 1920, 1080), null);
});

test("the built-in colors match the Rust DEFAULT_* constants", () => {
  const rust = readFileSync(new URL("../../rust-core/crates/vimmer-dsl/src/brand.rs", import.meta.url), "utf8");
  const def = (name: string) => new RegExp(`pub const DEFAULT_${name}: &str = "([^"]+)"`).exec(rust)?.[1];
  assert.equal(def("TEXT"), BUILTIN.text);
  assert.equal(def("LOWER_THIRD_SUBTITLE"), BUILTIN.lowerThirdSubtitle);
  assert.equal(def("CHART_LABEL"), BUILTIN.chartLabel);
  assert.equal(def("ACCENT"), BUILTIN.accent);
  assert.equal(def("PRIMARY"), BUILTIN.primary);
  assert.equal(def("BACKGROUND"), BUILTIN.background);
  // rgba(9, 9, 11, 0.72) is #09090bb8 (0xb8 / 255 = 0.7216).
  assert.equal(def("SUBTITLE_BACKGROUND"), "#09090bb8");
});

test("brand colors, fonts and the logo box", () => {
  const brand: Brand = {
    colors: { accent: "#ff8800", text: "#111" },
    fonts: { heading: { family: "Demo Heading", src: "assets/fonts/x.ttf" } },
    logo: { src: "assets/logo.png", corner: "bottom_left", size: 0.1, margin: 0.05, hide_on: ["intro"] },
  };
  assert.equal(fontStack(brand, "heading"), `"Demo Heading", ${BASE_FONT_STACK}`);
  assert.equal(fontStack(brand, "body"), BASE_FONT_STACK);
  assert.equal(brandColor(brand, "text", BUILTIN.text), "#111");
  assert.equal(brandColor(brand, "primary", BUILTIN.primary), BUILTIN.primary);
  assert.deepEqual(highlightColors(brand, 1), { color: "#ff8800", textShadow: "0 0 24px rgba(255, 136, 0, 0.6)" });
  assert.equal(withAlpha("#11223380", 0.5), "rgba(17, 34, 51, 0.251)");
  assert.equal(withAlpha("red", 0.5), "red");
  assert.deepEqual(logoBox(brand, 1920, 1080), { height: 108, opacity: 1, left: 96, bottom: 54 });
  assert.deepEqual(logoBox({ logo: { src: "l.png" } }, 1080, 1920), { height: 153.6, opacity: 1, right: 54, top: 96 });
  assert.equal(logoShownOn(brand, "intro"), false);
  assert.equal(logoShownOn(brand, "body"), true);
  assert.equal(logoShownOn({}, "body"), false);
});

test("sound effects play on the frames of their layer events (as SoundEffect::frame_in)", () => {
  const scene = {
    id: "b", duration_ms: 2000, text: null, highlights: [], image: null, motion: "static", audio: null,
    layers: [{ id: "t", kind: "text", x: 0.1, y: 0.1, width: 0.5, text: "hi", start_ms: 520, end_ms: 1800, exit: { kind: "fade", duration_ms: 300 } }],
  } as unknown as Scene;
  assert.equal(sfxFrame({ src: "a.wav", on: { layer: "t", event: "enter" } }, scene, 30), 16);
  assert.equal(sfxFrame({ src: "a.wav", on: { layer: "t", event: "exit" } }, scene, 30), 45);
  assert.equal(sfxFrame({ src: "a.wav", at_ms: 1990 }, scene, 30), 59, "rounds onto the cut: plays on the last frame");
  assert.equal(sfxFrame({ src: "a.wav", on: { layer: "nope", event: "enter" } }, scene, 30), null);
  // Without an exit animation the exit is the layer's end frame.
  const plain = { ...scene, layers: [{ ...scene.layers![0], exit: null }] } as Scene;
  assert.equal(sfxFrame({ src: "a.wav", on: { layer: "t", event: "exit" } }, plain, 30), 54);
});
