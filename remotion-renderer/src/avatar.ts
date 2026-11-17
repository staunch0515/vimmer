// The narrator avatar inset (数字人讲解人): where it sits and which clip frame it shows.
// Pure, so test/avatar.test.ts can check it without Remotion.
import type { AvatarClip } from "./types/timeline";

/** The generated clips run at this rate (vimmer-media/scripts/avatar/vimmer_avatar.py). */
export const AVATAR_CLIP_FPS = 25;

export type AvatarBox = { left: number; top: number; size: number; radius: number; ring: number };

/**
 * The square inset box in pixels. `size` is a fraction of the frame height; the margin is a
 * fraction of the width for the left/right edge and of the height for the top/bottom edge, as for
 * the logo (brand.ts `logoBox`).
 */
export const avatarBox = (clip: AvatarClip, width: number, height: number): AvatarBox => {
  const size = Math.round(Math.min(Math.max(clip.size, 0.05), 1) * height);
  const mx = clip.margin * width;
  const my = clip.margin * height;
  const left = clip.corner.endsWith("left") ? mx : width - mx - size;
  const top = clip.corner.startsWith("top") ? my : height - my - size;
  const radius = clip.shape === "rounded" ? size * 0.12 : size / 2;
  const ring = Math.max(0, clip.border_width) * size;
  return { left: Math.round(left), top: Math.round(top), size, radius, ring };
};

/**
 * The clip time in ms on frame `sceneFrame` of the scene (timeline rate `fps`): the middle of the
 * clip frame showing then, so the decoder lands on that frame whether it rounds or floors. It depends only on the scene-relative frame, and chunks always start at a
 * scene cut, so a chunked render shows exactly the frames of a full render.
 */
export const avatarClipMs = (clip: AvatarClip, sceneFrame: number, fps: number): number => {
  const clipFrame = Math.floor(((clip.offset_frames + sceneFrame) * AVATAR_CLIP_FPS) / fps + 1e-9);
  return ((clipFrame + 0.5) * 1000) / AVATAR_CLIP_FPS;
};
