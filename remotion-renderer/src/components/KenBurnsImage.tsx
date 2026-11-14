import { AbsoluteFill, Img, interpolate, useCurrentFrame, useVideoConfig } from "remotion";
import { useContext } from "react";
import { AssetUrlContext, resolveAsset } from "../assets";
import { sampleKeyframes, type KeyframePose } from "../motion/keyframes";
import type { Keyframe, MotionType, VideoClip } from "../types/timeline";
import { ClipVideo } from "./ClipVideo";

type Props = {
  src: string;
  motion: MotionType;
  // When non-empty, these replace `motion`.
  keyframes?: Keyframe[];
  durationInFrames: number;
};

type Transform = { scale: number; x: number; y: number };

// Pure function of progress (0..1) so the motion is frame-exact and deterministic.
const transformFor = (motion: MotionType, p: number): Transform => {
  switch (motion) {
    case "zoom_in":
      return { scale: interpolate(p, [0, 1], [1, 1.12]), x: 0, y: 0 };
    case "zoom_out":
      return { scale: interpolate(p, [0, 1], [1.12, 1]), x: 0, y: 0 };
    case "pan_left":
      return { scale: 1.08, x: interpolate(p, [0, 1], [3, -3]), y: 0 };
    case "pan_right":
      return { scale: 1.08, x: interpolate(p, [0, 1], [-3, 3]), y: 0 };
    case "static":
      return { scale: 1, x: 0, y: 0 };
  }
};

// The background's pose on scene frame `frame`: keyframes when there are any, else `motion`.
const backgroundPose = (
  motion: MotionType,
  keyframes: Keyframe[] | undefined,
  frame: number,
  fps: number,
  durationInFrames: number,
): KeyframePose => {
  const p = interpolate(frame, [0, Math.max(1, durationInFrames - 1)], [0, 1], {
    extrapolateLeft: "clamp",
    extrapolateRight: "clamp",
  });
  return keyframes && keyframes.length > 0
    ? sampleKeyframes(keyframes, (frame * 1000) / fps)
    : { ...transformFor(motion, p), opacity: 1, rotation: 0 };
};

const poseTransform = (pose: KeyframePose): string =>
  `translate3d(${pose.x}%, ${pose.y}%, 0) rotate(${pose.rotation}deg) scale(${pose.scale})`;

// Image with a blurred copy of itself behind it, so mismatched aspect ratios
// are padded with a soft background instead of flat bars. With keyframes, the whole
// image layer (both copies) takes the keyframed opacity, so a fade-out reveals the background.
export const KenBurnsImage: React.FC<Props> = ({ src, motion, keyframes, durationInFrames }) => {
  const frame = useCurrentFrame();
  const { fps } = useVideoConfig();
  const url = resolveAsset(src, useContext(AssetUrlContext));
  const pose = backgroundPose(motion, keyframes, frame, fps, durationInFrames);

  return (
    <AbsoluteFill style={{ overflow: "hidden", backgroundColor: "#09090b", opacity: pose.opacity }}>
      <Img
        src={url}
        style={{
          position: "absolute",
          inset: 0,
          width: "100%",
          height: "100%",
          objectFit: "cover",
          filter: "blur(48px) brightness(0.45) saturate(1.2)",
          transform: "scale(1.2)",
        }}
      />
      <Img
        src={url}
        style={{
          position: "absolute",
          inset: 0,
          width: "100%",
          height: "100%",
          objectFit: "contain",
          transform: poseTransform(pose),
          willChange: "transform",
        }}
      />
    </AbsoluteFill>
  );
};

// A video clip as the scene background, in the image's place: the same motion / keyframes move it.
// One decoded copy only (no blurred padding, which would decode the clip twice): `fit: contain`
// leaves the scene's dark background around it. Scene frame n shows clip frame n (motion/video.ts).
export const KenBurnsVideo: React.FC<{
  clip: VideoClip;
  motion: MotionType;
  keyframes?: Keyframe[];
  durationInFrames: number;
}> = ({ clip, motion, keyframes, durationInFrames }) => {
  const frame = useCurrentFrame();
  const { fps } = useVideoConfig();
  const pose = backgroundPose(motion, keyframes, frame, fps, durationInFrames);
  return (
    <AbsoluteFill style={{ overflow: "hidden", backgroundColor: "#09090b", opacity: pose.opacity }}>
      <AbsoluteFill style={{ transform: poseTransform(pose), willChange: "transform" }}>
        <ClipVideo clip={clip} frame={frame} span={durationInFrames} />
      </AbsoluteFill>
    </AbsoluteFill>
  );
};
