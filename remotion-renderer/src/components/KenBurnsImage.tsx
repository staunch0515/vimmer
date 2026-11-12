import { AbsoluteFill, Img, interpolate, useCurrentFrame } from "remotion";
import { resolveAsset } from "../assets";
import type { MotionType } from "../types/timeline";

type Props = {
  src: string;
  motion: MotionType;
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

// Image with a blurred copy of itself behind it, so mismatched aspect ratios
// are padded with a soft background instead of flat bars.
export const KenBurnsImage: React.FC<Props> = ({ src, motion, durationInFrames }) => {
  const frame = useCurrentFrame();
  const url = resolveAsset(src);
  const p = interpolate(frame, [0, Math.max(1, durationInFrames - 1)], [0, 1], {
    extrapolateLeft: "clamp",
    extrapolateRight: "clamp",
  });
  const { scale, x, y } = transformFor(motion, p);

  return (
    <AbsoluteFill style={{ overflow: "hidden", backgroundColor: "#09090b" }}>
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
          transform: `translate3d(${x}%, ${y}%, 0) scale(${scale})`,
          willChange: "transform",
        }}
      />
    </AbsoluteFill>
  );
};
