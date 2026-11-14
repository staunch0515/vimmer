import { createContext, useContext, type CSSProperties } from "react";
import { Freeze, OffthreadVideo, useVideoConfig } from "remotion";
import { AssetUrlContext, resolveAsset } from "../assets";
import { clipTimeAt } from "../motion/video";
import type { VideoClip } from "../types/timeline";

// File lengths in ms by src, when the host knows them (the editor learns them from the browser, so
// a preview of an untrimmed clip can loop and freeze). The renderer needs none: the scheduler gives
// every clip a trim_end_ms.
export type ClipLength = (src: string) => number | null;
export const ClipLengthContext = createContext<ClipLength | null>(null);

// A frame number that is a whole frame up to float noise (g * 1000 / fps * fps / 1000) is made
// exact, so a normalized clip is asked for exactly its frame g.
const snapFrame = (at: number): number => (Math.abs(at - Math.round(at)) < 1e-6 ? Math.round(at) : at);

// One video clip, showing the file frame for `frame` of its `span` frames (motion/video.ts). The
// player never runs the clip on its own clock: the frame is chosen here and held with <Freeze>, so
// a chunk shows exactly what a full render shows. Always muted: clip sound is mixed by the Rust core.
export const ClipVideo: React.FC<{ clip: VideoClip; frame: number; span: number; style?: CSSProperties }> = ({
  clip,
  frame,
  span,
  style,
}) => {
  const { fps } = useVideoConfig();
  const url = resolveAsset(clip.src, useContext(AssetUrlContext));
  const fileMs = useContext(ClipLengthContext)?.(clip.src) ?? null;
  const t = clipTimeAt(clip, frame, span, fps, fileMs);
  return (
    <Freeze frame={snapFrame((t.fileMs * fps) / 1000)}>
      <OffthreadVideo
        src={url}
        muted
        style={{ display: "block", width: "100%", height: "100%", objectFit: clip.fit ?? "cover", ...style }}
      />
    </Freeze>
  );
};
