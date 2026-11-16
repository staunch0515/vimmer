import { AbsoluteFill, useCurrentFrame, useVideoConfig } from "remotion";
import {
  DEFAULT_INTENSITY,
  OVERLAY_KINDS,
  grainOpacity,
  grainSeed,
  gradeStyle,
  hashString,
  overlayColor,
  overlaySeed,
  particlesAt,
  type EffectiveLook,
  type GradePreset,
} from "../motion/look";
import type { Overlay } from "../types/timeline";

// A scene drawn with its look (see motion/look.ts), bottom to top: the scene picture (`children`:
// image or video, layers, scene text) through the color grade, the grade's vignette, then the
// overlays in list order. The brand logo and subtitles are drawn outside the scenes, so they stay
// ungraded and on top. Only used when the look draws something: otherwise the Timeline renders
// the scene exactly as before.
//
// Every moving part is a pure function of the absolute timeline frame: `startFrame` (the scene's
// first frame in the FULL timeline, chunk offset included) plus the scene-local frame.
export const LookedScene: React.FC<{
  look: EffectiveLook;
  autoAs: GradePreset;
  startFrame: number;
  sceneId: string;
  children: React.ReactNode;
}> = ({ look, autoAs, startFrame, sceneId, children }) => {
  const grade = gradeStyle(look.grade, autoAs);
  const filterId = grade ? `vimmer-grade-${hashString(grade.matrix.join(","))}` : null;
  return (
    <AbsoluteFill>
      {grade && filterId ? (
        <>
          {/* Identical matrices get identical ids, so a repeated definition (a held frame under a
              transition, two scenes with the same grade) is harmless. */}
          <svg width={0} height={0} style={{ position: "absolute" }} aria-hidden>
            <filter id={filterId} x="0" y="0" width="100%" height="100%" colorInterpolationFilters="sRGB">
              <feColorMatrix type="matrix" values={grade.matrix.join(" ")} />
            </filter>
          </svg>
          <AbsoluteFill style={{ filter: `url(#${filterId})` }}>{children}</AbsoluteFill>
          {grade.vignette > 0 ? (
            <AbsoluteFill
              style={{
                background: `radial-gradient(ellipse at center, rgba(0,0,0,0) 55%, rgba(0,0,0,${grade.vignette}) 100%)`,
              }}
            />
          ) : null}
        </>
      ) : (
        children
      )}
      {look.overlays.map((o, i) =>
        OVERLAY_KINDS.includes(o.kind) && (o.intensity ?? DEFAULT_INTENSITY) > 0 ? (
          <OverlayLayer key={`${o.kind}-${i}`} overlay={o} seed={overlaySeed(o, i, look.inherited, sceneId)} startFrame={startFrame} />
        ) : null,
      )}
    </AbsoluteFill>
  );
};

const OverlayLayer: React.FC<{ overlay: Overlay; seed: number; startFrame: number }> = ({ overlay, seed, startFrame }) => {
  const frame = startFrame + useCurrentFrame();
  const { width, height } = useVideoConfig();
  const intensity = overlay.intensity ?? DEFAULT_INTENSITY;
  if (overlay.kind === "film_grain") return <Grain seed={seed} frame={frame} intensity={intensity} width={width} height={height} />;
  const color = overlayColor(overlay);
  const particles = particlesAt(overlay.kind, intensity, seed, frame, width, height);
  const gradientId = `vimmer-${overlay.kind}-${hashString(color)}`;
  // Snow: soft white flakes. Dust and bokeh glow (screen blend), so they only ever lighten.
  const soft = overlay.kind === "bokeh" ? [0.9, 0.6] : overlay.kind === "dust" ? [1, 0.5] : [1, 0.75];
  return (
    <AbsoluteFill style={{ mixBlendMode: overlay.kind === "snow" ? "normal" : "screen", pointerEvents: "none" }}>
      <svg width={width} height={height} viewBox={`0 0 ${width} ${height}`} style={{ position: "absolute", inset: 0 }}>
        <defs>
          <radialGradient id={gradientId}>
            <stop offset="0%" stopColor={color} stopOpacity={soft[0]} />
            <stop offset="55%" stopColor={color} stopOpacity={soft[1]} />
            <stop offset="100%" stopColor={color} stopOpacity={0} />
          </radialGradient>
        </defs>
        {particles.map((p, i) => (
          <circle key={i} cx={p.x.toFixed(2)} cy={p.y.toFixed(2)} r={p.r.toFixed(2)} fill={`url(#${gradientId})`} opacity={p.opacity.toFixed(3)} />
        ))}
      </svg>
    </AbsoluteFill>
  );
};

// Film grain: fresh fractal noise every frame (its seed follows the absolute frame), drawn at half
// resolution and scaled up so the grain is soft and clumpy like film, blended with `overlay` so
// mid-gray changes nothing and the noise lightens and darkens around it.
const Grain: React.FC<{ seed: number; frame: number; intensity: number; width: number; height: number }> = ({
  seed,
  frame,
  intensity,
  width,
  height,
}) => {
  const w = Math.ceil(width / 2);
  const h = Math.ceil(height / 2);
  // The frame is part of the id: a held frame and the live scene may show the same grain overlay
  // at two different frames.
  const id = `vimmer-grain-${seed}-${frame}`;
  return (
    <AbsoluteFill style={{ mixBlendMode: "overlay", opacity: grainOpacity(intensity), pointerEvents: "none" }}>
      <svg
        width={w}
        height={h}
        viewBox={`0 0 ${w} ${h}`}
        style={{ position: "absolute", left: 0, top: 0, transform: "scale(2)", transformOrigin: "0 0" }}
      >
        <filter id={id} x="0" y="0" width="100%" height="100%" colorInterpolationFilters="sRGB">
          <feTurbulence type="fractalNoise" baseFrequency="0.85" numOctaves={2} seed={grainSeed(seed, frame)} stitchTiles="stitch" />
          <feColorMatrix type="saturate" values="0" />
          <feComponentTransfer>
            <feFuncA type="linear" slope="0" intercept="1" />
          </feComponentTransfer>
        </filter>
        <rect width={w} height={h} filter={`url(#${id})`} />
      </svg>
    </AbsoluteFill>
  );
};
