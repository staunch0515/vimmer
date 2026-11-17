// Narrator avatar inset (数字人讲解人): box math and chunk-safe clip frames.
import assert from "node:assert/strict";
import { test } from "node:test";
import { avatarBox, avatarClipSeconds } from "../src/avatar.ts";
import type { AvatarClip } from "../src/types/timeline.ts";

const clip = (over: Partial<AvatarClip> = {}): AvatarClip => ({
  src: "generated/avatar/k.mp4",
  offset_frames: 0,
  corner: "bottom_right",
  size: 0.3,
  margin: 0.04,
  shape: "circle",
  border_width: 0,
  border_color: "#FFFFFF",
  ...over,
});

test("the box sits in its corner with the logo's margin rule", () => {
  const b = avatarBox(clip(), 1920, 1080);
  assert.equal(b.size, 324);
  assert.equal(b.left, Math.round(1920 - 0.04 * 1920 - 324));
  assert.equal(b.top, Math.round(1080 - 0.04 * 1080 - 324));
  assert.equal(b.radius, 162);
  const tl = avatarBox(clip({ corner: "top_left", shape: "rounded", border_width: 0.025 }), 1920, 1080);
  assert.deepEqual([tl.left, tl.top], [77, 43]);
  assert.ok(Math.abs(tl.radius - 324 * 0.12) < 1e-9);
  assert.ok(Math.abs(tl.ring - 8.1) < 1e-9);
  // Portrait frames: still a square, sized by the height.
  const p = avatarBox(clip({ corner: "top_right" }), 1080, 1920);
  assert.equal(p.size, 576);
  assert.equal(p.left + p.size, Math.round(1080 - 0.04 * 1080));
});

test("clip time follows the scene offset", () => {
  assert.equal(avatarClipSeconds(clip(), 0, 25), 0);
  assert.equal(avatarClipSeconds(clip({ offset_frames: 50 }), 0, 25), 2);
  assert.equal(avatarClipSeconds(clip({ offset_frames: 258 }), 32, 30), 290 / 30);
});

test("a chunked render shows the same clip times as a full render", () => {
  // A run of two scenes at 30 fps: scene A frames 0..257, scene B from 258. The scheduler gives B
  // offset_frames 258; a chunk starting at B's cut renders B from its own frame 0.
  const fps = 30;
  const a = clip({ offset_frames: 0 });
  const b = clip({ offset_frames: 258 });
  for (let g = 0; g < 500; g++) {
    const t = g < 258 ? avatarClipSeconds(a, g, fps) : avatarClipSeconds(b, g - 258, fps);
    assert.equal(t, g / fps);
  }
});
