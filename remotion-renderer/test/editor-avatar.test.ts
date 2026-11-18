// 数字人讲解人 in the editor (editor/src/avatarUi.ts): the edits and their undo steps, which avatar
// a scene shows, the status lines, progress parsing, the preview fallback and the render panel's
// warning. Also the render options' 「必须带数字人」.
import assert from "node:assert/strict";
import { test } from "node:test";
import {
  aboutTime,
  avatarRenderWarning,
  overTextFix,
  parseProgress,
  progressFraction,
  sceneAvatar,
  statusLines,
  withAvatar,
  withPreviewAvatars,
  withSceneAvatar,
  type AvatarStatus,
} from "../editor/src/avatarUi.ts";
import { record, undo, type Undoable } from "../editor/src/history.ts";
import { DEFAULT_OPTIONS, FALLBACK_DEFAULTS, parseOptions, toPanelArgs, type CheckFinding } from "../editor/src/renderOptions.ts";
import type { Scene, VideoTimeline } from "../src/types/timeline.ts";

const scene = (id: string, p: Partial<Scene> = {}): Scene =>
  ({ id, duration_ms: 2000, text: "Hi", highlights: [], image: null, motion: "static", audio: null, ...p }) as Scene;
const tl = (p: Partial<VideoTimeline> = {}): VideoTimeline =>
  ({ fps: 30, resolution: { width: 1920, height: 1080 }, scenes: [scene("a"), scene("b"), scene("c")], subtitles: [], ...p }) as VideoTimeline;

// What the reducer does for one action: edit, then record one history step.
type Doc = Undoable<VideoTimeline>;
const doc = (t: VideoTimeline): Doc => ({ timeline: t, history: [], future: [], merge: null });
const step = (d: Doc, next: VideoTimeline): Doc => record(d, { ...d, timeline: next }, null, (a, b) => JSON.stringify(a) === JSON.stringify(b));

test("picking a photo, then a corner: each one undo step; removing gives the old JSON back", () => {
  const t0 = tl();
  const d1 = step(doc(t0), withAvatar(t0, { photo: "assets/me.png" }));
  assert.deepEqual(d1.timeline.avatar, { photo: "assets/me.png" });
  const d2 = step(d1, withAvatar(d1.timeline, { corner: "top_left", size: 0.25 }));
  assert.deepEqual(d2.timeline.avatar, { photo: "assets/me.png", corner: "top_left", size: 0.25 });
  assert.equal(d2.history.length, 2);
  assert.deepEqual(undo(d2)!.timeline, d1.timeline);
  // Turning the border off drops the key.
  const bordered = withAvatar(d2.timeline, { border: { color: "#FFFFFF", width: 0.02 } });
  assert.deepEqual(withAvatar(bordered, { border: undefined }).avatar, d2.timeline.avatar);
  const d3 = step(d2, withAvatar(d2.timeline, null));
  assert.equal(JSON.stringify(d3.timeline), JSON.stringify(t0));
  // An empty photo removes it too.
  assert.equal(withAvatar(d2.timeline, { photo: "" }).avatar, undefined);
});

test("a look change starts from the brand's avatar, keeping its photo", () => {
  const t = tl({ brand: { avatar: { photo: "brand.png", size: 0.4 } } });
  assert.deepEqual(withAvatar(t, { corner: "top_right" }).avatar, { photo: "brand.png", size: 0.4, corner: "top_right" });
  assert.deepEqual(t.brand?.avatar, { photo: "brand.png", size: 0.4 }, "the brand is untouched");
});

test("scene settings: off, a corner, another photo; each one step; empty means none", () => {
  const t0 = tl({ avatar: { photo: "me.png" } });
  const d1 = step(doc(t0), withSceneAvatar(t0, 1, { enabled: false }));
  assert.deepEqual(d1.timeline.scenes[1].avatar, { enabled: false });
  assert.equal(sceneAvatar(d1.timeline, 1), null);
  assert.equal(sceneAvatar(d1.timeline, 0)?.corner, "bottom_right");
  const d2 = step(d1, withSceneAvatar(d1.timeline, 1, { enabled: true, corner: "bottom_left" }));
  assert.equal(sceneAvatar(d2.timeline, 1)?.corner, "bottom_left");
  assert.equal(d2.history.length, 2);
  // Back to the default corner and no own photo: the scene's avatar block goes away.
  const d3 = withSceneAvatar(withSceneAvatar(d2.timeline, 1, { corner: undefined, enabled: undefined }), 1, { photo: "" });
  assert.equal("avatar" in d3.scenes[1], false);
  assert.equal(JSON.stringify(withSceneAvatar(d2.timeline, 1, null)), JSON.stringify(t0));
  // Another photo for one scene.
  assert.equal(sceneAvatar(withSceneAvatar(t0, 2, { photo: "other.png" }), 2)?.photo, "other.png");
});

test("which avatar a scene shows follows the Rust resolve", () => {
  // No default: only a scene with its own photo shows one.
  const t = tl({ scenes: [scene("a", { avatar: { photo: "x.png" } }), scene("b", { avatar: { corner: "top_left" } })] });
  assert.equal(sceneAvatar(t, 0)?.photo, "x.png");
  assert.equal(sceneAvatar(t, 1), null);
  // A default switched off, turned on by one scene.
  const off = tl({ avatar: { photo: "me.png", enabled: false }, scenes: [scene("a"), scene("b", { avatar: { enabled: true } })] });
  assert.equal(sceneAvatar(off, 0), null);
  assert.equal(sceneAvatar(off, 1)?.photo, "me.png");
  // The timeline's replaces the brand's; defaults as in the DSL.
  const both = tl({ avatar: { photo: "t.png" }, brand: { avatar: { photo: "b.png" } } });
  assert.deepEqual(sceneAvatar(both, 0), { photo: "t.png", corner: "bottom_right", size: 0.3, margin: 0.04, shape: "circle", borderWidth: 0, borderColor: "#FFFFFF" });
});

test("the over-text fix reads both suggestion forms", () => {
  const f = (path: string, value: unknown): CheckFinding => ({ severity: "warning", code: "avatar.over_text", message: "", suggestion: { text: "", path, value } });
  assert.deepEqual(overTextFix(f("/scenes/2/avatar", { corner: "bottom_left" })), { index: 2, corner: "bottom_left" });
  assert.deepEqual(overTextFix(f("/scenes/0/avatar/corner", "top_left")), { index: 0, corner: "top_left" });
  assert.equal(overTextFix(f("/scenes/0/avatar", { enabled: false })), null);
  assert.equal(overTextFix({ ...f("/scenes/0/avatar/corner", "top_left"), code: "avatar.over_subtitles" }), null);
});

const status = (p: Partial<Extract<AvatarStatus, { ok: true }>>): AvatarStatus => ({ ok: true, known: true, runs: [], missing_estimate_s: 0, speaking_estimate_s: 0, ...p });
const run = (scenes: string[], ready: boolean, estimate_s = 366) => ({ scenes, photo: "me.png", clip: `generated/avatar/${scenes[0]}.mp4`, ready, duration_ms: 31000, estimate_s });

test("status lines: per run, or one line when this timeline was never generated", () => {
  assert.deepEqual(statusLines(null), []);
  assert.deepEqual(statusLines(status({ known: false, missing_estimate_s: 69 })), [{ ready: false, label: "○ 未生成 · 约 69 秒", scenes: [] }]);
  assert.deepEqual(
    statusLines(status({ runs: [run(["a", "b"], true), run(["c"], false)] })).map((l) => l.label),
    ["✓ 已生成", "○ 未生成 · 约 6 分钟"],
  );
  assert.deepEqual(statusLines({ ok: false, error: "x" }), []);
  assert.equal(aboutTime(366), "约 6 分钟");
});

test("progress: frames per run, the drift log line does not reset it", () => {
  let p = parseProgress(null, "Synthesizing narration");
  assert.equal(progressFraction(p), 0);
  p = parseProgress(p, "Narrator avatar 1/2: preparing speech (31 s, about 6 min (约 6 分钟))");
  assert.deepEqual([p.run, p.runs, p.done], [1, 2, 0]);
  p = parseProgress(p, "Narrator avatar 1/2: 400/800 frames, about 3 min (约 3 分钟) left");
  assert.deepEqual([p.done, p.total], [400, 800]);
  assert.equal(progressFraction(p), 0.25);
  p = parseProgress(p, "Narrator avatar 1/2: head pose pitch [-0.5, 0.4]° raw, [-0.7, 0.3]° after the drift fix (800 frames)");
  assert.equal(p.done, 400);
  p = parseProgress(p, "Narrator avatar 2/2: render");
  assert.deepEqual([p.run, p.done, p.total], [2, 0, 0]);
  assert.equal(progressFraction(p), 0.5);
});

test("preview: the dimmed photo with a badge before generation, the clip at its offset after", () => {
  const t = tl({ avatar: { photo: "me.png", corner: "top_left", border: { width: 0.03 } }, scenes: [scene("a"), scene("b"), scene("c", { avatar: { enabled: false } })] });
  // Never generated: every shown scene draws the photo.
  const before = withPreviewAvatars(t, status({ known: false, missing_estimate_s: 69 }));
  const a = before.scenes[0].avatar_clip!;
  assert.equal(a.src, "");
  assert.equal(a.preview_photo, "me.png");
  assert.equal(a.preview_badge, "未生成 · 约 69 秒");
  assert.deepEqual([a.corner, a.size, a.border_width, a.border_color], ["top_left", 0.3, 0.03, "#FFFFFF"]);
  assert.equal(before.scenes[2].avatar_clip, undefined, "a scene with the avatar off draws nothing");
  // The browser editor (no status): still the photo, plain badge.
  assert.equal(withPreviewAvatars(t, null).scenes[1].avatar_clip!.preview_badge, "未生成");
  // Generated run a+b: b plays from 2 s into the clip (60 frames at 30 fps).
  const after = withPreviewAvatars(t, status({ runs: [run(["a", "b"], true)] }));
  assert.deepEqual([after.scenes[0].avatar_clip!.src, after.scenes[0].avatar_clip!.offset_frames], ["generated/avatar/a.mp4", 0]);
  assert.equal(after.scenes[1].avatar_clip!.offset_frames, 60);
  assert.equal(after.scenes[1].avatar_clip!.preview_photo, undefined);
  // A run not ready yet shows its own estimate.
  assert.equal(withPreviewAvatars(t, status({ runs: [run(["a", "b"], false, 120)] })).scenes[0].avatar_clip!.preview_badge, "未生成 · 约 2 分钟");
  // No avatar: the very same timeline object.
  const plain = tl();
  assert.equal(withPreviewAvatars(plain, null), plain);
});

test("render panel: warn when clips are missing, block when they are required", () => {
  const t = tl({ avatar: { photo: "me.png" } });
  const missing = status({ known: false, missing_estimate_s: 69 });
  assert.deepEqual(avatarRenderWarning(t, missing, false), { level: "warn", text: "数字人还没生成，这次渲染不会显示讲解人" });
  assert.equal(avatarRenderWarning(t, missing, true)?.level, "block");
  assert.equal(avatarRenderWarning(t, status({ runs: [run(["a"], true)] }), true), null);
  assert.equal(avatarRenderWarning(tl(), missing, false), null, "no avatar, no warning");
  assert.equal(avatarRenderWarning(t, null, false), null, "status unknown (browser): no warning");
});

test("「必须带数字人」 maps to --avatar require; 「不带数字人渲染」 to off; old options stay as they were", () => {
  assert.equal("avatar" in toPanelArgs(DEFAULT_OPTIONS, FALLBACK_DEFAULTS), false);
  assert.equal(toPanelArgs({ ...DEFAULT_OPTIONS, requireAvatar: true }, FALLBACK_DEFAULTS).avatar, "require");
  assert.equal(toPanelArgs({ ...DEFAULT_OPTIONS, requireAvatar: true }, FALLBACK_DEFAULTS, true).avatar, "off");
  assert.equal(parseOptions(JSON.stringify({ tts: "say" })).requireAvatar, false);
  assert.equal(parseOptions(JSON.stringify({ requireAvatar: true })).requireAvatar, true);
});
