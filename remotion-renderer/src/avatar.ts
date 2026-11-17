// The narrator avatar inset (数字人讲解人): where it sits and which clip frame it shows.
// Pure, so test/avatar.test.ts can check it without Remotion.
import type { AvatarClip } from "./types/timeline";

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
 * The clip time in seconds on frame `sceneFrame` of the scene (timeline rate `fps`), as
 * AvatarInset plays it (`trimBefore = offset_frames`). It depends only on the scene-relative
 * frame, and chunks always start at a scene cut, so a chunked render shows exactly the clip
 * times of a full render.
 */
export const avatarClipSeconds = (clip: AvatarClip, sceneFrame: number, fps: number): number =>
  (clip.offset_frames + sceneFrame) / fps;
