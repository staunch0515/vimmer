import { useContext, useId } from "react";
import { Img, useVideoConfig } from "remotion";
import { AssetUrlContext, resolveAsset } from "../assets";
import { strokePx } from "../motion/layers";
import { DEFAULT_TYPING_CPS, cursorVisible, typedCount, typedSegments, typingUnits } from "../motion/typewriter";
import type { IconLayer, ImageLayer, ShapeLayer, TextLayer } from "../types/timeline";
import { AnimatedWords, highlightKeys, highlightStyle, normalize, splitWords } from "./AnimatedText";

// What each layer kind draws inside its box. The box itself (position, size, transform, opacity,
// timing) is set by LayerStack. `frameHeight` is the composition height: font sizes, strokes and
// corner radii are fractions of it. `frame` counts from the layer's start.

type Ctx = { frameHeight: number; frame: number };

export const ImageLayerView: React.FC<{ layer: ImageLayer } & Ctx> = ({ layer, frameHeight }) => {
  const url = resolveAsset(layer.src, useContext(AssetUrlContext));
  return (
    <Img
      src={url}
      style={{
        display: "block",
        width: "100%",
        height: "100%",
        objectFit: layer.fit ?? "cover",
        borderRadius: (layer.corner_radius ?? 0) * frameHeight,
      }}
    />
  );
};

// Same face as the scene text, so text layers and scene text match.
export const LAYER_FONT_FAMILY = "Inter, 'Geist', system-ui, sans-serif";
export const LAYER_LETTER_SPACING_EM = -0.02;

export const TextLayerView: React.FC<{ layer: TextLayer; boxHeight: number | null } & Ctx> = ({
  layer,
  boxHeight,
  frameHeight,
  frame,
}) => {
  const fontSize = (layer.font_size ?? 0.06) * frameHeight;
  const align = layer.align ?? "center";
  const bg = layer.background;
  const words = splitWords(layer.text);
  const text = (
    <div
      style={{
        width: "100%",
        boxSizing: "border-box",
        fontFamily: LAYER_FONT_FAMILY,
        fontSize,
        fontWeight: layer.weight ?? 600,
        lineHeight: layer.line_height ?? 1.2,
        letterSpacing: `${LAYER_LETTER_SPACING_EM}em`,
        color: layer.color ?? "#fafafa",
        textAlign: align,
        overflowWrap: "break-word",
        ...(bg
          ? {
              backgroundColor: bg.color,
              padding: (bg.padding ?? 0.4) * fontSize,
              borderRadius: (bg.radius ?? 0.3) * fontSize,
            }
          : {}),
      }}
    >
      {layer.reveal === "typewriter" ? (
        <TypedText layer={layer} frame={frame} />
      ) : (
        <AnimatedWords
          words={words}
          highlightSet={highlightKeys(layer.highlights ?? [])}
          frame={frame}
          animate={layer.reveal === "words"}
        />
      )}
    </div>
  );
  if (boxHeight == null) return text;
  // A fixed box centers the text vertically.
  return <div style={{ display: "flex", alignItems: "center", width: "100%", height: "100%" }}>{text}</div>;
};

// Typewriter reveal (motion/typewriter.ts). The whole text is laid out from the first frame and the
// untyped part is only hidden, so lines never re-wrap while typing. The cursor is an absolutely
// placed bar inside an empty inline span: it takes no width and adds no line-break opportunity.
const TypedText: React.FC<{ layer: TextLayer; frame: number }> = ({ layer, frame }) => {
  const { fps } = useVideoConfig();
  const cps = layer.typing_cps ?? DEFAULT_TYPING_CPS;
  const total = typingUnits(layer.text).length;
  const count = typedCount(frame, fps, cps, total);
  const keys = highlightKeys(layer.highlights ?? []);
  const segments = typedSegments(layer.text, count, (w) => keys.has(normalize(w)));
  const showCursor = Boolean(layer.cursor) && cursorVisible(frame, fps, cps, total);
  return (
    <>
      {segments.map((seg, i) => (
        <span key={i} style={seg.highlight ? highlightStyle(1) : undefined}>
          {seg.shown}
          {seg.cursorAfter && showCursor ? (
            <span style={{ position: "relative" }}>
              <span
                style={{
                  position: "absolute",
                  left: "0.03em",
                  top: "0.08em",
                  bottom: "0.08em",
                  width: "0.07em",
                  background: layer.color ?? "#fafafa",
                }}
              />
            </span>
          ) : null}
          {seg.hidden ? <span style={{ visibility: "hidden" }}>{seg.hidden}</span> : null}
        </span>
      ))}
    </>
  );
};

export const ShapeLayerView: React.FC<{ layer: ShapeLayer; width: number; height: number } & Ctx> = ({
  layer,
  width,
  height,
  frameHeight,
}) => {
  const shape = layer.shape ?? "rect";
  const stroke = layer.stroke ?? undefined;
  const sw = strokePx(layer.stroke_width, Boolean(stroke), frameHeight);
  const fill = layer.fill ?? "none";
  const common = { fill, stroke: stroke ?? "none", strokeWidth: sw };
  if (shape === "line") {
    // An SVG with a zero width or height paints nothing, so the canvas is grown by the stroke on
    // every side and shifted back: the line still runs from the box's top-left to bottom-right.
    const pad = Math.max(sw, 1) / 2;
    return (
      <svg
        width={width + 2 * pad}
        height={height + 2 * pad}
        style={{ display: "block", position: "absolute", left: -pad, top: -pad, overflow: "visible" }}
      >
        <line x1={pad} y1={pad} x2={width + pad} y2={height + pad} strokeLinecap="round" {...common} fill="none" />
      </svg>
    );
  }
  return (
    <svg width={width} height={height} style={{ display: "block", overflow: "visible" }}>
      {shape === "ellipse" ? (
        <ellipse
          cx={width / 2}
          cy={height / 2}
          rx={Math.max(0, (width - sw) / 2)}
          ry={Math.max(0, (height - sw) / 2)}
          {...common}
        />
      ) : (
        <rect
          x={sw / 2}
          y={sw / 2}
          width={Math.max(0, width - sw)}
          height={Math.max(0, height - sw)}
          rx={Math.min((layer.corner_radius ?? 0) * frameHeight, width / 2, height / 2)}
          {...common}
        />
      )}
    </svg>
  );
};

// The SVG is drawn through an SVG filter that floods it with `color`, keeping only its alpha, so
// any single- or multi-color SVG becomes a silhouette in that color. <Img> makes the renderer wait
// for the file, and the filter is in the same DOM, so the first frame is already tinted.
export const IconLayerView: React.FC<{ layer: IconLayer } & Ctx> = ({ layer }) => {
  const url = resolveAsset(layer.src, useContext(AssetUrlContext));
  const filterId = `vimmer-icon-${useId().replace(/[^a-zA-Z0-9_-]/g, "")}`;
  return (
    <>
      <svg width={0} height={0} style={{ position: "absolute" }} aria-hidden>
        <filter id={filterId} colorInterpolationFilters="sRGB">
          <feFlood floodColor={layer.color ?? "#fafafa"} />
          <feComposite in2="SourceAlpha" operator="in" />
        </filter>
      </svg>
      <Img
        src={url}
        style={{ display: "block", width: "100%", height: "100%", objectFit: "contain", filter: `url(#${filterId})` }}
      />
    </>
  );
};
