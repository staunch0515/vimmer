import { useContext } from "react";
import { OffthreadVideo, useVideoConfig } from "remotion";
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
      <OffthreadVideo
        src={url}
        muted
        trimBefore={clip.offset_frames > 0 ? clip.offset_frames : undefined}
        style={{ display: "block", width: "100%", height: "100%", objectFit: "cover", borderRadius: Math.max(0, box.radius - box.ring) }}
      />
    </div>
  );
};
