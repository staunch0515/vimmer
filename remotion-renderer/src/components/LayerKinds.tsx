import { useContext, useId } from "react";
import { Img, useVideoConfig } from "remotion";
import { AssetUrlContext, resolveAsset } from "../assets";
import { strokePx } from "../motion/layers";
import { DEFAULT_TYPING_CPS, cursorVisible, typedCount, typedSegments, typingUnits } from "../motion/typewriter";
import { applyEasing } from "../motion/keyframes";
import {
  BAR_FILL,
  HBAR_LABEL_FRAC,
  HBAR_VALUE_FRAC,
  VBAR_LABEL_EM,
  VBAR_VALUE_EM,
  barProgress,
  chartMax,
  countFrames,
  formatNumber,
  frameProgress,
  lerpValue,
  ringFraction,
} from "../motion/data";
import { lowerThirdPose } from "../motion/lowerThird";
import type { BarChartLayer, CounterLayer, ProgressRingLayer, IconLayer, ImageLayer, LowerThirdLayer, ShapeLayer, TextLayer } from "../types/timeline";
import { AnimatedWords, highlightKeys, highlightStyle, normalize, splitWords } from "./AnimatedText";

// What each layer kind draws inside its box. The box itself (position, size, transform, opacity,
// timing) is set by LayerStack. `frameHeight` is the composition height: font sizes, strokes and
// corner radii are fractions of it. `frame` counts from the layer's start, `span` is the layer's
// length in frames. Everything is computed from these, so chunked renders match full ones.

type Ctx = { frameHeight: number; frame: number; span: number; fps: number };

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

// Accent bar, then a box with the title over the subtitle. The box hugs its text up to the layer's
// width; a fixed height centers it vertically. Sizes are in em of the title font.
export const LOWER_THIRD_BAR_EM = 0.16;
export const LOWER_THIRD_PAD_V_EM = 0.38;
export const LOWER_THIRD_PAD_H_EM = 0.7;
export const LOWER_THIRD_SUBTITLE_SCALE = 0.62;

export const LowerThirdView: React.FC<{ layer: LowerThirdLayer; boxHeight: number | null } & Ctx> = ({
  layer,
  boxHeight,
  frameHeight,
  frame,
  span,
  fps,
}) => {
  const fontSize = (layer.font_size ?? 0.045) * frameHeight;
  const pose = lowerThirdPose(frame, span, fps, !layer.enter, !layer.exit);
  const bg = layer.background_color === undefined ? "#09090bd9" : layer.background_color;
  const textStyle = { transform: `translateX(${pose.textShift}em)`, opacity: pose.textOpacity };
  return (
    <div
      style={{
        display: "flex",
        alignItems: "stretch",
        width: "100%",
        height: boxHeight == null ? undefined : "100%",
        fontFamily: LAYER_FONT_FAMILY,
        fontSize,
        letterSpacing: `${LAYER_LETTER_SPACING_EM}em`,
      }}
    >
      <div
        style={{
          flex: "0 0 auto",
          width: `${LOWER_THIRD_BAR_EM}em`,
          background: layer.accent_color ?? "#6366f1",
          transform: `scaleY(${pose.bar})`,
          transformOrigin: "bottom",
        }}
      />
      <div
        style={{
          flex: "0 1 auto",
          minWidth: 0,
          display: "flex",
          flexDirection: "column",
          justifyContent: "center",
          padding: `${LOWER_THIRD_PAD_V_EM}em ${LOWER_THIRD_PAD_H_EM}em`,
          background: bg ?? undefined,
          clipPath: pose.reveal >= 1 ? undefined : `inset(0 ${(1 - pose.reveal) * 100}% 0 0)`,
          overflowWrap: "break-word",
        }}
      >
        <div style={{ ...textStyle, color: layer.color ?? "#fafafa", fontWeight: 700, lineHeight: 1.15 }}>{layer.title}</div>
        {layer.subtitle ? (
          <div
            style={{
              ...textStyle,
              color: layer.subtitle_color ?? "#d4d4d8",
              fontWeight: 500,
              fontSize: `${LOWER_THIRD_SUBTITLE_SCALE}em`,
              lineHeight: 1.3,
              marginTop: "0.25em",
            }}
          >
            {layer.subtitle}
          </div>
        ) : null}
      </div>
    </div>
  );
};

// One line, tabular digits so the width does not jitter while counting. A fixed height centers it.
export const CounterView: React.FC<{ layer: CounterLayer; boxHeight: number | null } & Ctx> = ({
  layer,
  boxHeight,
  frameHeight,
  frame,
  span,
  fps,
}) => {
  const n = countFrames(layer, layer.duration_ms, span, fps);
  const value = lerpValue(layer.from ?? 0, layer.to, applyEasing(layer.ease ?? "ease_out", frameProgress(frame, n)));
  const text = formatNumber(value, layer.decimals ?? 0, layer.separator ?? ",", layer.decimal_mark ?? ".", layer.prefix ?? "", layer.suffix ?? "");
  return (
    <div
      style={{
        display: "flex",
        alignItems: "center",
        justifyContent: { left: "flex-start", center: "center", right: "flex-end" }[layer.align ?? "center"],
        width: "100%",
        height: boxHeight == null ? undefined : "100%",
        fontFamily: LAYER_FONT_FAMILY,
        fontSize: (layer.font_size ?? 0.12) * frameHeight,
        fontWeight: layer.weight ?? 700,
        letterSpacing: `${LAYER_LETTER_SPACING_EM}em`,
        lineHeight: 1.2,
        color: layer.color ?? "#fafafa",
        fontVariantNumeric: "tabular-nums",
        whiteSpace: "nowrap",
      }}
    >
      {text}
    </div>
  );
};

// Bars drawn with divs in the layer's box (no chart library). Each bar's length is its value over
// the chart max times its eased grow progress (may overshoot with springy easings); its value label
// counts up with the unovershot progress and fades in at the start of its growth.
export const BarChartView: React.FC<{ layer: BarChartLayer; width: number; height: number } & Ctx> = ({
  layer,
  width,
  height,
  frameHeight,
  frame,
  fps,
}) => {
  const f = (layer.font_size ?? 0.032) * frameHeight;
  const bars = layer.bars.slice(0, 12);
  const n = Math.max(1, bars.length);
  const max = chartMax(bars.map((b) => b.value), layer.max);
  const showValues = layer.show_values ?? true;
  const ease = layer.ease ?? "ease_out";
  const labelColor = layer.label_color ?? "#e4e4e7";
  const text = (extra: React.CSSProperties): React.CSSProperties => ({
    position: "absolute",
    fontFamily: LAYER_FONT_FAMILY,
    fontSize: f,
    lineHeight: 1.2,
    letterSpacing: `${LAYER_LETTER_SPACING_EM}em`,
    color: labelColor,
    whiteSpace: "nowrap",
    fontVariantNumeric: "tabular-nums",
    ...extra,
  });
  const grown = bars.map((b, i) => {
    const p = barProgress(frame, i, fps, layer.grow_ms ?? 800, layer.stagger_ms ?? 120);
    const e = applyEasing(ease, p);
    return {
      bar: b,
      frac: Math.max(0, (b.value / max) * e),
      label: formatNumber(b.value * Math.min(1, e), layer.decimals ?? 0, ",", ".", layer.prefix ?? "", layer.suffix ?? ""),
      fade: Math.min(1, p * 4),
    };
  });
  const color = (b: (typeof bars)[number]) => b.color ?? layer.color ?? "#6366f1";
  const axis = { position: "absolute" as const, background: labelColor, opacity: 0.35 };

  if (layer.orientation === "horizontal") {
    const labelW = width * HBAR_LABEL_FRAC;
    const valueW = showValues ? width * HBAR_VALUE_FRAC : 0;
    const plotW = Math.max(0, width - labelW - valueW);
    const row = height / n;
    const thick = row * BAR_FILL;
    return (
      <div style={{ position: "relative", width, height }}>
        <div style={{ ...axis, left: labelW - 1, top: 0, width: 2, height }} />
        {grown.map(({ bar, frac, label, fade }, i) => {
          const cy = row * i + row / 2;
          const len = plotW * frac;
          return (
            <div key={i}>
              <div style={text({ right: width - labelW + 0.4 * f, top: cy - 0.6 * f, textAlign: "right" })}>{bar.label}</div>
              <div
                style={{
                  position: "absolute",
                  left: labelW,
                  top: cy - thick / 2,
                  width: len,
                  height: thick,
                  background: color(bar),
                  borderRadius: `0 ${Math.min(0.2 * f, thick / 2)}px ${Math.min(0.2 * f, thick / 2)}px 0`,
                }}
              />
              {showValues ? (
                <div style={text({ left: labelW + len + 0.3 * f, top: cy - 0.6 * f, opacity: fade, fontWeight: 600 })}>{label}</div>
              ) : null}
            </div>
          );
        })}
      </div>
    );
  }

  const valueH = showValues ? VBAR_VALUE_EM * f : 0;
  const labelH = VBAR_LABEL_EM * f;
  const plotH = Math.max(0, height - valueH - labelH);
  const baseY = valueH + plotH;
  const slot = width / n;
  const thick = slot * BAR_FILL;
  return (
    <div style={{ position: "relative", width, height }}>
      <div style={{ ...axis, left: 0, top: baseY - 1, width, height: 2 }} />
      {grown.map(({ bar, frac, label, fade }, i) => {
        const cx = slot * i + slot / 2;
        const len = plotH * frac;
        return (
          <div key={i}>
            <div
              style={{
                position: "absolute",
                left: cx - thick / 2,
                top: baseY - len,
                width: thick,
                height: len,
                background: color(bar),
                borderRadius: `${Math.min(0.2 * f, thick / 2)}px ${Math.min(0.2 * f, thick / 2)}px 0 0`,
              }}
            />
            {showValues ? (
              <div style={text({ left: cx - slot / 2, width: slot, top: baseY - len - 1.3 * f, textAlign: "center", opacity: fade, fontWeight: 600 })}>
                {label}
              </div>
            ) : null}
            <div style={text({ left: cx - slot / 2, width: slot, top: baseY + 0.3 * f, textAlign: "center" })}>{bar.label}</div>
          </div>
        );
      })}
    </div>
  );
};

// Center label size, as a fraction of the ring's diameter (percentage / custom text).
export const RING_PERCENT_SCALE = 0.22;
export const RING_LABEL_SCALE = 0.15;

// The largest circle that fits the box, centered: a full track and an arc from 12 o'clock,
// clockwise, drawn with stroke-dasharray. The fill fraction is clamped to 0..1, so a springy ease
// may overshoot `value` but never draws past a full ring. Nothing is drawn for an empty arc (a round cap would show a dot).
export const ProgressRingView: React.FC<{ layer: ProgressRingLayer; width: number; height: number } & Ctx> = ({
  layer,
  width,
  height,
  frame,
  span,
  fps,
}) => {
  const d = Math.min(width, height);
  const stroke = (layer.thickness ?? 0.12) * d;
  const r = Math.max(0, (d - stroke) / 2);
  const c = 2 * Math.PI * r;
  const n = countFrames(layer, layer.duration_ms, span, fps);
  const e = applyEasing(layer.ease ?? "ease_out", frameProgress(frame, n));
  const frac = ringFraction(layer.from ?? 0, layer.value, e);
  const cx = width / 2;
  const cy = height / 2;
  const custom = layer.label ?? null;
  // The percentage follows the fill but does not overshoot with springy easings (like chart values).
  const label = custom ?? formatNumber(ringFraction(layer.from ?? 0, layer.value, Math.min(1, e)) * 100, layer.decimals ?? 0, ",", ".", "", "%");
  return (
    <div style={{ position: "relative", width, height }}>
      <svg width={width} height={height} style={{ display: "block", position: "absolute", left: 0, top: 0 }}>
        <circle cx={cx} cy={cy} r={r} fill="none" stroke={layer.track_color ?? "#ffffff26"} strokeWidth={stroke} />
        {frac > 0 ? (
          <circle
            cx={cx}
            cy={cy}
            r={r}
            fill="none"
            stroke={layer.color ?? "#6366f1"}
            strokeWidth={stroke}
            strokeLinecap={frac >= 1 ? "butt" : "round"}
            strokeDasharray={`${frac * c} ${c}`}
            transform={`rotate(-90 ${cx} ${cy})`}
          />
        ) : null}
      </svg>
      {(layer.show_label ?? true) ? (
        <div
          style={{
            position: "absolute",
            inset: 0,
            display: "flex",
            alignItems: "center",
            justifyContent: "center",
            fontFamily: LAYER_FONT_FAMILY,
            fontSize: d * (custom != null ? RING_LABEL_SCALE : RING_PERCENT_SCALE),
            fontWeight: 700,
            letterSpacing: `${LAYER_LETTER_SPACING_EM}em`,
            color: layer.label_color ?? "#fafafa",
            fontVariantNumeric: "tabular-nums",
            whiteSpace: "nowrap",
          }}
        >
          {label}
        </div>
      ) : null}
    </div>
  );
};
