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
import type { Brand, TextBackground, TextLayout, TextPage, TextPosition, TextStyle } from "../types/timeline";
import { useBrand } from "./Brand";
import { PAGE_FADE_FRAMES, fontPx, isDefaultLayout, pageAt, resolveTextLayout, topInsetPx } from "../textPages";
import { msToFrames } from "../layout";
import { LINE_BREAK, highlightParts, splitWordsAndBreaks } from "../highlight";

type Props = {
  text: string;
  highlights: string[];
  // True when the scene has an image under the text: a soft dark gradient rises from the bottom
  // edge so the white text stays readable on bright pictures. Text-only scenes render without it.
  onImage?: boolean;
  // The scene's text_style (outline, box, shadow); each part falls back to the brand's.
  textStyle?: TextStyle | null;
  // The scene's text_layout (size, position, paging); each part falls back to the brand's.
  // Neither set: the text is drawn exactly as before text layouts existed.
  layout?: TextLayout | null;
  // The pages to show (src/textPages.ts `scenePages`); fewer than two: the whole text.
  pages?: TextPage[];
  // For a top-corner logo hidden on this scene (top / middle text starts below a shown logo).
  sceneId?: string;
};

// Behind the text (lower part of the frame) and the subtitle band.
const IMAGE_SCRIM =
  "linear-gradient(to top, rgba(9, 9, 11, 0.62) 0%, rgba(9, 9, 11, 0.45) 30%, rgba(9, 9, 11, 0) 62%)";
// The same behind text at the top, and a band behind text in the middle.
const POSITION_SCRIM: Record<TextPosition, string> = {
  bottom: IMAGE_SCRIM,
  top: "linear-gradient(to bottom, rgba(9, 9, 11, 0.62) 0%, rgba(9, 9, 11, 0.45) 30%, rgba(9, 9, 11, 0) 62%)",
  middle:
    "linear-gradient(to bottom, rgba(9, 9, 11, 0) 15%, rgba(9, 9, 11, 0.45) 35%, rgba(9, 9, 11, 0.45) 65%, rgba(9, 9, 11, 0) 85%)",
};
const JUSTIFY: Record<TextPosition, string> = { top: "flex-start", middle: "center", bottom: "flex-end" };

// The middle band moved to centre `y` (a fraction of the height) for text at a free point.
export const bandScrim = (y: number): string => {
  const pct = (d: number) => `${+((y + d) * 100).toFixed(2)}%`;
  return `linear-gradient(to bottom, rgba(9, 9, 11, 0) ${pct(-0.35)}, rgba(9, 9, 11, 0.45) ${pct(-0.15)}, rgba(9, 9, 11, 0.45) ${pct(0.15)}, rgba(9, 9, 11, 0) ${pct(0.35)})`;
};

const STAGGER_FRAMES = 3;
const WORD_FADE_FRAMES = 12;

export const normalize = (word: string): string => word.toLowerCase().replace(/[^\p{L}\p{N}]/gu, "");

// One inline-block span per word. With `animate` (the default) each word fades and rises in,
// STAGGER_FRAMES after the one before it, counting from `frame` 0; highlighted words glow in.
// Without it, every word is shown at rest. Shared by the scene text and text layers. `shadow` is a
// text style's shadow: highlighted words then keep it under their glow (without it they show only
// the glow, as they always did). With `highlights` (the raw list), a word that does not match as a
// whole may glow in part: a keyword inside CJK text (src/highlight.ts); its parts reveal together.
// A LINE_BREAK entry in `words` is a line break (src/highlight.ts `splitWordsAndBreaks`); it takes
// no reveal step.
export const AnimatedWords: React.FC<{
  words: string[];
  highlightSet: Set<string>;
  frame: number;
  animate?: boolean;
  shadow?: string;
  highlights?: readonly string[];
}> = ({ words, highlightSet, frame, animate = true, shadow, highlights }) => {
  const brand = useBrand();
  let step = -1;
  return (
  <>
    {words.map((word, i) => {
      if (word === LINE_BREAK) return <br key={`br-${i}`} />;
      step += 1;
      const start = step * STAGGER_FRAMES;
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
      const parts = !isHighlight && highlights?.length ? highlightParts(word, highlights) : null;
      const partial = parts !== null && parts.some((p) => p.highlight);
      const glow = isHighlight || partial
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
          {partial
            ? parts.map((p, k) =>
                p.highlight ? (
                  <span key={k} style={glowStyle(highlightStyle(glow, brand), shadow)}>
                    {p.text}
                  </span>
                ) : (
                  p.text
                ),
              )
            : word}
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
// padding keeps content inside the safe area on phones. A paged text shows one page at a time,
// each revealed word by word from its own start; the outgoing page fades and lifts away over
// PAGE_FADE_FRAMES while the next one comes in. Pages are a pure function of the props, so a
// chunk render draws every frame as the full render does.
export const AnimatedText: React.FC<Props> = ({ text, highlights, onImage = false, textStyle, layout, pages, sceneId = "" }) => {
  const frame = useCurrentFrame();
  const { width, height, fps } = useVideoConfig();
  const brand = useBrand();
  const highlightSet = highlightKeys(highlights);
  const style = resolveTextStyle(textStyle, brand);
  const styled = !isEmptyStyle(style);
  const scale = styleScale(width, height);
  const shadow = shadowCss(style.shadow, scale, SOFT_SHADOW);
  const plain = isDefaultLayout(layout, brand);
  const resolved = resolveTextLayout(layout, brand);
  const position = plain ? "bottom" : resolved.position;
  const at = plain ? null : resolved.at;

  // A free point: a full-width row (8% side padding) whose centre sits at y of the height,
  // shifted sideways so the text centre is at x of the width. Otherwise the padded text area.
  const area: React.CSSProperties = at
    ? {
        top: `${Math.fround(at.y) * 100}%`,
        bottom: "auto",
        height: "auto",
        transform: `translate(${(Math.fround(at.x ?? 0.5) - 0.5) * width}px, -50%)`,
        flexDirection: "row",
        justifyContent: "center",
        padding: "0 8%",
      }
    : {
        justifyContent: JUSTIFY[position],
        alignItems: "center",
        // Fixed bottom inset keeps scene text clear of the subtitle band.
        padding: position === "bottom" ? "8% 8% 260px" : `${topInsetPx(position, brand, sceneId, width, height)}px 8% 260px`,
      };

  // One page of words in the text area, revealed from `pageFrame` 0.
  const page = (words: string[], pageFrame: number, extra?: React.CSSProperties, key?: string) => (
    <AbsoluteFill
      key={key}
      style={{
        ...area,
        ...extra,
        // The outgoing page's lift adds to a free point's placement.
        ...(at && extra?.transform ? { transform: `${area.transform} ${extra.transform}` } : {}),
      }}
    >
      <div
        style={{
          maxWidth: "88%",
          textAlign: "center",
          fontFamily: fontStack(brand, "heading"),
          fontSize: plain ? 92 : fontPx(resolved.size, width, height),
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
            frame={pageFrame}
            box={style.background}
            highlights={highlights}
            scale={scale}
            shadow={style.shadow ? shadow : undefined}
          />
        ) : (
          <AnimatedWords
            words={words}
            highlightSet={highlightSet}
            highlights={highlights}
            frame={pageFrame}
            shadow={style.shadow ? shadow : undefined}
          />
        )}
      </div>
    </AbsoluteFill>
  );

  let body: React.ReactNode;
  if (!pages || pages.length < 2) {
    body = page(splitWordsAndBreaks(text), frame);
  } else {
    const starts = pages.map((p) => msToFrames(p.start_ms, fps));
    const k = pageAt(starts.map((start_ms) => ({ start_ms })), frame);
    const out = k > 0 && frame < starts[k] + PAGE_FADE_FRAMES ? k - 1 : -1;
    const fade = out >= 0 ? interpolate(frame, [starts[k], starts[k] + PAGE_FADE_FRAMES], [0, 1], { extrapolateLeft: "clamp", extrapolateRight: "clamp" }) : 0;
    body = (
      <>
        {out >= 0
          ? page(splitWordsAndBreaks(pages[out].text), frame - starts[out], { opacity: 1 - fade, transform: `translateY(${-24 * fade}px)` }, `page-${out}`)
          : null}
        {page(splitWordsAndBreaks(pages[k].text), frame - starts[k], undefined, `page-${k}`)}
      </>
    );
  }
  if (!onImage) return <>{body}</>;
  return (
    <>
      <AbsoluteFill style={{ background: at ? bandScrim(Math.fround(at.y)) : POSITION_SCRIM[position] }} />
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
  highlights?: readonly string[];
}> = ({ words, highlightSet, frame, box, scale, shadow, highlights }) => {
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
          <AnimatedWords words={words} highlightSet={highlightSet} highlights={highlights} frame={frame} shadow={shadow} />
        </span>
      </div>
    </>
  );
};
