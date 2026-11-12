import { AbsoluteFill, interpolate, useCurrentFrame } from "remotion";

type Props = {
  text: string;
  highlights: string[];
};

const STAGGER_FRAMES = 3;
const WORD_FADE_FRAMES = 12;

const normalize = (word: string): string => word.toLowerCase().replace(/[^\p{L}\p{N}]/gu, "");

// Word-by-word entrance with keyword highlights. Text wraps via CSS, and the
// padding keeps content inside the safe area on phones.
export const AnimatedText: React.FC<Props> = ({ text, highlights }) => {
  const frame = useCurrentFrame();
  const highlightSet = new Set(highlights.map(normalize));
  const words = text.split(/\s+/).filter(Boolean);

  return (
    <AbsoluteFill
      style={{
        justifyContent: "flex-end",
        alignItems: "center",
        padding: "8% 8% 12%",
      }}
    >
      <div
        style={{
          maxWidth: "88%",
          textAlign: "center",
          fontFamily: "Inter, 'Geist', system-ui, sans-serif",
          fontSize: 92,
          fontWeight: 600,
          lineHeight: 1.2,
          letterSpacing: "-0.02em",
          color: "#fafafa",
          textShadow: "0 4px 32px rgba(0, 0, 0, 0.6)",
          overflowWrap: "break-word",
          textWrap: "balance",
        }}
      >
        {words.map((word, i) => {
          const start = i * STAGGER_FRAMES;
          const opacity = interpolate(frame, [start, start + WORD_FADE_FRAMES], [0, 1], {
            extrapolateLeft: "clamp",
            extrapolateRight: "clamp",
          });
          const rise = interpolate(frame, [start, start + WORD_FADE_FRAMES], [18, 0], {
            extrapolateLeft: "clamp",
            extrapolateRight: "clamp",
          });
          const isHighlight = highlightSet.has(normalize(word));
          const glow = isHighlight
            ? interpolate(frame, [start, start + 30], [0, 1], {
                extrapolateLeft: "clamp",
                extrapolateRight: "clamp",
              })
            : 0;

          return (
            <span
              key={`${word}-${i}`}
              style={{
                display: "inline-block",
                opacity,
                transform: `translateY(${rise}px)`,
                marginRight: "0.25em",
                ...(isHighlight
                  ? {
                      color: "#a5b4fc",
                      textShadow: `0 0 ${24 * glow}px rgba(99, 102, 241, ${0.6 * glow})`,
                    }
                  : {}),
              }}
            >
              {word}
            </span>
          );
        })}
      </div>
    </AbsoluteFill>
  );
};
