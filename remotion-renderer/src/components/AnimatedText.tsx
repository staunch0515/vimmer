import { AbsoluteFill, interpolate, useCurrentFrame, useVideoConfig } from "remotion";
import { BUILTIN, brandColor, fontStack, highlightColors } from "../brand";
import {
  BORDER_LINE_GAP_EM,
  SOFT_SHADOW,
  TRANSPARENT,
  borderCss,
  borderPx,
  isEmptyStyle,
  outlineCss,
  resolveTextStyle,
  shadowCss,
  splitAlpha,
  styleScale,
  withGlow,
} from "../textStyle";
import type { Brand, TextBackground, TextStyle } from "../types/timeline";
import { useBrand } from "./Brand";

type Props = {
  text: string;
  highlights: string[];
  // True when the scene has an image under the text: a soft dark gradient rises from the bottom
  // edge so the white text stays readable on bright pictures. Text-only scenes render without it.
  onImage?: boolean;
  // The scene's text_style (outline, box, shadow); each part falls back to the brand's.
  textStyle?: TextStyle | null;
};

// Behind the text (lower part of the frame) and the subtitle band.
const IMAGE_SCRIM =
  "linear-gradient(to top, rgba(9, 9, 11, 0.62) 0%, rgba(9, 9, 11, 0.45) 30%, rgba(9, 9, 11, 0) 62%)";

const STAGGER_FRAMES = 3;
const WORD_FADE_FRAMES = 12;

export const normalize = (word: string): string => word.toLowerCase().replace(/[^\p{L}\p{N}]/gu, "");

// One inline-block span per word. With `animate` (the default) each word fades and rises in,
// STAGGER_FRAMES after the one before it, counting from `frame` 0; highlighted words glow in.
// Without it, every word is shown at rest. Shared by the scene text and text layers. `shadow` is a
// text style's shadow: highlighted words then keep it under their glow (without it they show only
// the glow, as they always did).
export const AnimatedWords: React.FC<{
  words: string[];
  highlightSet: Set<string>;
  frame: number;
  animate?: boolean;
  shadow?: string;
}> = ({ words, highlightSet, frame, animate = true, shadow }) => {
  const brand = useBrand();
  return (
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
            ...(isHighlight ? glowStyle(highlightStyle(glow, brand), shadow) : {}),
          }}
        >
          {word}
        </span>
      );
    })}
  </>
  );
};

// The accent treatment of a highlighted word at glow 0..1 (the brand accent when there is one).
export const highlightStyle = (glow: number, brand: Brand | null = null): React.CSSProperties => highlightColors(brand, glow);

const glowStyle = (h: React.CSSProperties, shadow: string | undefined): React.CSSProperties =>
  shadow === undefined ? h : { ...h, textShadow: withGlow(String(h.textShadow ?? ""), shadow) };

export const highlightKeys = (highlights: readonly string[]): Set<string> => new Set(highlights.map(normalize));
export const splitWords = (text: string): string[] => text.split(/\s+/).filter(Boolean);

// Word-by-word entrance with keyword highlights. Text wraps via CSS, and the
// padding keeps content inside the safe area on phones.
export const AnimatedText: React.FC<Props> = ({ text, highlights, onImage = false, textStyle }) => {
  const frame = useCurrentFrame();
  const { width, height } = useVideoConfig();
  const brand = useBrand();
  const highlightSet = highlightKeys(highlights);
  const words = splitWords(text);
  const style = resolveTextStyle(textStyle, brand);
  const styled = !isEmptyStyle(style);
  const scale = styleScale(width, height);
  const shadow = shadowCss(style.shadow, scale, SOFT_SHADOW);

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
          fontFamily: fontStack(brand, "heading"),
          fontSize: 92,
          fontWeight: 600,
          lineHeight: 1.2,
          letterSpacing: "-0.02em",
          color: brandColor(brand, "text", BUILTIN.text),
          textShadow: shadow,
          overflowWrap: "break-word",
          textWrap: "balance",
          ...(styled ? outlineCss(style.outline, scale) : {}),
          ...(style.background ? { display: "grid" } : {}),
        }}
      >
        {style.background ? (
          <BoxedWords
            words={words}
            highlightSet={highlightSet}
            frame={frame}
            box={style.background}
            scale={scale}
            shadow={style.shadow ? shadow : undefined}
          />
        ) : (
          <AnimatedWords words={words} highlightSet={highlightSet} frame={frame} shadow={style.shadow ? shadow : undefined} />
        )}
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

// The scene text over a box that hugs each line. Two copies of the words share one grid cell, so
// they wrap identically: below, the words in an inline span with the box (`box-decoration-break:
// clone` gives every line its own padded, rounded piece) and transparent glyphs; above, the
// visible words. The box fades in with the first word. The inline padding adds one word gap on the
// left to balance the gap after each line's last word.
//
// Without a border the padded line pieces overlap, so the box copy is drawn in the opaque color
// and the color's alpha is applied to the copy as a whole: overlaps stay one even color. With a
// border the lines move apart (line height + 2 x padding + border + BORDER_LINE_GAP_EM) so every
// line is its own framed piece, drawn in the box color as is (an absent color is no fill).
const BoxedWords: React.FC<{
  words: string[];
  highlightSet: Set<string>;
  frame: number;
  box: TextBackground;
  scale: number;
  shadow?: string;
}> = ({ words, highlightSet, frame, box, scale, shadow }) => {
  const pad = box.padding ?? 0.4;
  const border = borderPx(box, scale);
  const fade = interpolate(frame, [0, WORD_FADE_FRAMES], [0, 1], { extrapolateLeft: "clamp", extrapolateRight: "clamp" });
  const line: React.CSSProperties = {
    padding: `${pad}em ${pad}em ${pad}em ${pad + 0.25}em`,
    borderRadius: `${box.radius ?? 0.3}em`,
    WebkitBoxDecorationBreak: "clone",
    boxDecorationBreak: "clone",
    ...(border > 0 ? { borderWidth: border, borderStyle: "solid", borderColor: "transparent" } : {}),
  };
  const lineHeight = border > 0 ? `calc(${+(1.2 + 2 * pad + BORDER_LINE_GAP_EM).toFixed(4)}em + ${2 * border}px)` : undefined;
  const { solid, alpha } = splitAlpha(box.color ?? TRANSPARENT);
  const under: React.CSSProperties =
    border > 0
      ? { ...line, background: box.color ?? TRANSPARENT, ...borderCss(box, scale) }
      : { ...line, background: solid };
  return (
    <>
      <div
        aria-hidden
        style={{
          gridArea: "1 / 1",
          opacity: border > 0 ? fade : alpha * fade,
          color: "transparent",
          textShadow: "none",
          WebkitTextStroke: "0",
          paintOrder: "normal",
          lineHeight,
        }}
      >
        <span style={under}>
          <AnimatedWords words={words} highlightSet={new Set()} frame={frame} animate={false} />
        </span>
      </div>
      <div style={{ gridArea: "1 / 1", lineHeight }}>
        <span style={line}>
          <AnimatedWords words={words} highlightSet={highlightSet} frame={frame} shadow={shadow} />
        </span>
      </div>
    </>
  );
};
