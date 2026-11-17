import { useContext } from "react";
import { Freeze, OffthreadVideo, useCurrentFrame, useVideoConfig } from "remotion";
import { AssetUrlContext, resolveAsset } from "../assets";
import { avatarBox, avatarClipMs } from "../avatar";
import type { AvatarClip } from "../types/timeline";

// The narrator's talking head in its corner, inside the scene's own Sequence (frames count from
// the scene start). The clip frame is chosen here and held with <Freeze>, like ClipVideo, so a
// chunk shows exactly what a full render shows. Always muted: the speech is already in the mix.
export const AvatarInset: React.FC<{ clip: AvatarClip }> = ({ clip }) => {
  const frame = useCurrentFrame();
  const { fps, width, height } = useVideoConfig();
  const url = resolveAsset(clip.src, useContext(AssetUrlContext));
  const box = avatarBox(clip, width, height);
  const ms = avatarClipMs(clip, frame, fps);
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
      <Freeze frame={(ms * fps) / 1000}>
        <OffthreadVideo
          src={url}
          muted
          style={{ display: "block", width: "100%", height: "100%", objectFit: "cover", borderRadius: Math.max(0, box.radius - box.ring) }}
        />
      </Freeze>
    </div>
  );
};
