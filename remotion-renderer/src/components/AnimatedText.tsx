import { AbsoluteFill, interpolate, useCurrentFrame } from "remotion";

type Props = {
  text: string;
  highlights: string[];
  // True when the scene has an image under the text: a soft dark gradient rises from the bottom
  // edge so the white text stays readable on bright pictures. Text-only scenes render without it.
  onImage?: boolean;
};

// Behind the text (lower part of the frame) and the subtitle band.
const IMAGE_SCRIM =
  "linear-gradient(to top, rgba(9, 9, 11, 0.62) 0%, rgba(9, 9, 11, 0.45) 30%, rgba(9, 9, 11, 0) 62%)";

const STAGGER_FRAMES = 3;
const WORD_FADE_FRAMES = 12;

export const normalize = (word: string): string => word.toLowerCase().replace(/[^\p{L}\p{N}]/gu, "");

// One inline-block span per word. With `animate` (the default) each word fades and rises in,
// STAGGER_FRAMES after the one before it, counting from `frame` 0; highlighted words glow in.
// Without it, every word is shown at rest. Shared by the scene text and text layers.
export const AnimatedWords: React.FC<{
  words: string[];
  highlightSet: Set<string>;
  frame: number;
  animate?: boolean;
}> = ({ words, highlightSet, frame, animate = true }) => (
  <>
    {words.map((word, i) => {
      const start = i * STAGGER_FRAMES;
      const opacity = animate
        ? interpolate(frame, [start, start + WORD_FADE_FRAMES], [0, 1], {
            extrapolateLeft: "clamp",
            extrapolateRight: "clamp",
          })
        : 1;
      const rise = animate
        ? interpolate(frame, [start, start + WORD_FADE_FRAMES], [18, 0], {
            extrapolateLeft: "clamp",
            extrapolateRight: "clamp",
          })
        : 0;
      const isHighlight = highlightSet.has(normalize(word));
      const glow = isHighlight
        ? animate
          ? interpolate(frame, [start, start + 30], [0, 1], {
              extrapolateLeft: "clamp",
              extrapolateRight: "clamp",
            })
          : 1
        : 0;

      return (
        <span
          key={`${word}-${i}`}
          style={{
            display: "inline-block",
            opacity,
            transform: `translateY(${rise}px)`,
            marginRight: "0.25em",
            ...(isHighlight ? highlightStyle(glow) : {}),
          }}
        >
          {word}
        </span>
      );
    })}
  </>
);

// The accent treatment of a highlighted word at glow 0..1.
export const highlightStyle = (glow: number): React.CSSProperties => ({
  color: "#a5b4fc",
  textShadow: `0 0 ${24 * glow}px rgba(99, 102, 241, ${0.6 * glow})`,
});

export const highlightKeys = (highlights: readonly string[]): Set<string> => new Set(highlights.map(normalize));
export const splitWords = (text: string): string[] => text.split(/\s+/).filter(Boolean);

// Word-by-word entrance with keyword highlights. Text wraps via CSS, and the
// padding keeps content inside the safe area on phones.
export const AnimatedText: React.FC<Props> = ({ text, highlights, onImage = false }) => {
  const frame = useCurrentFrame();
  const highlightSet = highlightKeys(highlights);
  const words = splitWords(text);

  const body = (
    <AbsoluteFill
      style={{
        justifyContent: "flex-end",
        alignItems: "center",
        // Fixed bottom inset keeps scene text clear of the subtitle band.
        padding: "8% 8% 260px",
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
        <AnimatedWords words={words} highlightSet={highlightSet} frame={frame} />
      </div>
    </AbsoluteFill>
  );
  if (!onImage) return body;
  return (
    <>
      <AbsoluteFill style={{ background: IMAGE_SCRIM }} />
      {body}
    </>
  );
};
