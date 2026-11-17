import { useContext } from "react";
import { Img, OffthreadVideo, useVideoConfig } from "remotion";
import { AssetUrlContext, resolveAsset } from "../assets";
import { avatarBox } from "../avatar";
import type { AvatarClip } from "../types/timeline";

// The narrator's talking head in its corner, inside the scene's own Sequence (frames count from
// the scene start). The clip covers a run of scenes, so the scene starts `offset_frames` into it
// (`trimBefore`): its time on scene frame f is (offset_frames + f) / fps (src/avatar.ts), the same
// in a chunk, which always starts at a scene cut, as in a full render. Not held with <Freeze>
// like ClipVideo: a frozen frame past the end of a chunk's composition shows the wrong clip time.
// Always muted: the speech is already in the mix.
export const AvatarInset: React.FC<{ clip: AvatarClip }> = ({ clip }) => {
  const { width, height } = useVideoConfig();
  const url = resolveAsset(clip.src, useContext(AssetUrlContext));
  const box = avatarBox(clip, width, height);
  return (
    <div
      style={{
        position: "absolute",
        left: box.left,
        top: box.top,
        width: box.size,
        height: box.size,
        borderRadius: box.radius,
        overflow: "hidden",
        boxSizing: "border-box",
        border: box.ring > 0 ? `${box.ring}px solid ${clip.border_color}` : undefined,
        backgroundColor: box.ring > 0 ? clip.border_color : undefined,
      }}
    >
      {clip.preview_photo !== undefined && !clip.src ? (
        <AvatarStill clip={clip} radius={Math.max(0, box.radius - box.ring)} />
      ) : (
      <OffthreadVideo
        src={url}
        muted
        trimBefore={clip.offset_frames > 0 ? clip.offset_frames : undefined}
        style={{ display: "block", width: "100%", height: "100%", objectFit: "cover", borderRadius: Math.max(0, box.radius - box.ring) }}
      />
      )}
    </div>
  );
};

// The editor preview before the clip is generated: the still photo, dimmed, with its badge.
const AvatarStill: React.FC<{ clip: AvatarClip; radius: number }> = ({ clip, radius }) => {
  const { height } = useVideoConfig();
  const url = resolveAsset(clip.preview_photo ?? "", useContext(AssetUrlContext));
  return (
    <div data-avatar-still style={{ position: "relative", width: "100%", height: "100%", borderRadius: radius, overflow: "hidden", background: "#3f3f46" }}>
      {clip.preview_photo ? (
        <Img src={url} style={{ display: "block", width: "100%", height: "100%", objectFit: "cover", opacity: 0.55, filter: "grayscale(0.35)" }} />
      ) : null}
      {clip.preview_badge ? (
        <div
          style={{
            position: "absolute",
            left: "50%",
            bottom: "12%",
            transform: "translateX(-50%)",
            whiteSpace: "nowrap",
            padding: `${Math.round(height * 0.006)}px ${Math.round(height * 0.012)}px`,
            borderRadius: height,
            background: "rgba(9,9,11,0.78)",
            color: "#fff",
            fontSize: Math.max(10, Math.round(height * 0.02)),
            fontFamily: "sans-serif",
          }}
        >
          {clip.preview_badge}
        </div>
      ) : null}
    </div>
  );
};
