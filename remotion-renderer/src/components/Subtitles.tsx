import { AbsoluteFill, interpolate, useCurrentFrame, useVideoConfig } from "remotion";
import { msToFrames } from "../layout";
import type { SubtitleCue } from "../types/timeline";

const FADE_FRAMES = 6;

type Props = {
  cues: SubtitleCue[];
};

// Shows the cue whose frame range contains the current frame. Frame ranges come from the
// same rounding as the Rust side, so a cue starts on the exact frame the DSL specifies.
export const Subtitles: React.FC<Props> = ({ cues }) => {
  const frame = useCurrentFrame();
  const { fps, width, height } = useVideoConfig();

  const active = cues.find(
    (c) => frame >= msToFrames(c.start_ms, fps) && frame < msToFrames(c.end_ms, fps),
  );
  if (!active) {
    return null;
  }

  const start = msToFrames(active.start_ms, fps);
  const end = msToFrames(active.end_ms, fps);
  const fadeIn = interpolate(frame, [start, start + FADE_FRAMES], [0, 1], {
    extrapolateLeft: "clamp",
    extrapolateRight: "clamp",
  });
  const fadeOut = interpolate(frame, [end - FADE_FRAMES, end], [1, 0], {
    extrapolateLeft: "clamp",
    extrapolateRight: "clamp",
  });

  // Scale with the shorter side so the size reads the same in landscape and portrait.
  const fontSize = Math.round(Math.min(width, height) * 0.05);

  return (
    <AbsoluteFill
      style={{ justifyContent: "flex-end", alignItems: "center", paddingBottom: "4%", pointerEvents: "none" }}
    >
      <div
        style={{
          opacity: Math.min(fadeIn, fadeOut),
          maxWidth: "86%",
          padding: `${Math.round(fontSize * 0.3)}px ${Math.round(fontSize * 0.6)}px`,
          borderRadius: fontSize * 0.4,
          backgroundColor: "rgba(9, 9, 11, 0.72)",
          border: "1px solid rgba(255, 255, 255, 0.08)",
          color: "#fafafa",
          fontFamily: "Inter, 'Geist', system-ui, sans-serif",
          fontSize,
          fontWeight: 500,
          lineHeight: 1.3,
          textAlign: "center",
          textWrap: "balance",
          overflowWrap: "break-word",
        }}
      >
        {active.text}
      </div>
    </AbsoluteFill>
  );
};
