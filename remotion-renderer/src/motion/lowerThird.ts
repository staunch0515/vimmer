// Pure timing of a lower third's built-in entrance and exit. No runtime imports, so it can be
// unit-tested with Node (test/motion.test.ts). Mirrors `LowerThirdLayer::builtin_ms` in
// rust-core/crates/vimmer-dsl/src/layer.rs, in frames: the entrance takes LOWER_THIRD_IN_MS (at
// most 40 % of the layer's frames), the exit LOWER_THIRD_OUT_MS (at most 30 %). A layer with its
// own `enter` / `exit` skips the built-in one on that side.

export const LOWER_THIRD_IN_MS = 700;
export const LOWER_THIRD_OUT_MS = 450;

const clamp01 = (v: number): number => Math.min(1, Math.max(0, v));
const easeOutCubic = (t: number): number => 1 - (1 - t) ** 3;
const easeInCubic = (t: number): number => t ** 3;

export type LowerThirdPose = {
  bar: number; // accent bar scaleY, 0..1 (grows from the bottom)
  reveal: number; // visible fraction of the box from the left, 0..1 (clip-path wipe)
  textShift: number; // text offset in em (negative: to the left)
  textOpacity: number;
};

// Pose on layer frame `frame` of a layer `span` frames long.
export const lowerThirdPose = (
  frame: number,
  span: number,
  fps: number,
  builtinIn: boolean,
  builtinOut: boolean,
): LowerThirdPose => {
  const inF = builtinIn ? Math.min(Math.round((LOWER_THIRD_IN_MS * fps) / 1000), Math.floor((span * 2) / 5)) : 0;
  const outF = builtinOut ? Math.min(Math.round((LOWER_THIRD_OUT_MS * fps) / 1000), Math.floor((span * 3) / 10)) : 0;
  const pIn = inF > 0 ? clamp01(frame / inF) : 1;
  const pOut = outF > 0 ? clamp01((frame - (span - outF) + 1) / outF) : 0;
  const text = easeOutCubic(clamp01((pIn - 0.35) / 0.65));
  const out = easeInCubic(pOut);
  return {
    bar: easeOutCubic(clamp01(pIn / 0.45)) * (1 - easeInCubic(clamp01((pOut - 0.5) / 0.5))),
    reveal: easeOutCubic(clamp01((pIn - 0.15) / 0.85)) * (1 - out),
    textShift: 0 - 0.6 * (1 - text + out), // "0 -": never -0 at rest
    textOpacity: text * (1 - clamp01((pOut - 0.4) / 0.6)),
  };
};
