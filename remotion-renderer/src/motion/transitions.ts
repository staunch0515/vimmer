// Overlap rule for scene transitions (documented here and in README.md):
//
//   A scene with `transition_in` of duration D starts at its nominal start time, as always.
//   Its first D milliseconds blend in over the previous scene's LAST FRAME, which is held
//   still for those D milliseconds. The previous scene keeps its nominal end time, so the
//   total duration and every scene's start do not change. The overlap lies inside the
//   incoming scene, which means it never needs frames from another scene's span.
//
// This module is pure: it maps the transition kind and progress to CSS for the incoming layer.

import type { TransitionKind } from "../types/timeline";

export type TransitionPose = {
  opacity: number;
  transform: string;
  clipPath: string;
};

// `progress` runs from 0 (only the held frame is visible) to 1 (the incoming scene is complete).
export const transitionPose = (kind: TransitionKind, progress: number): TransitionPose => {
  const p = Math.min(1, Math.max(0, progress));
  const rest = 1 - p;
  const none = "none";
  switch (kind) {
    case "fade":
      return { opacity: p, transform: none, clipPath: none };
    case "slide_left": // incoming enters from the right and moves left
      return { opacity: 1, transform: `translate3d(${rest * 100}%, 0, 0)`, clipPath: none };
    case "slide_right": // incoming enters from the left and moves right
      return { opacity: 1, transform: `translate3d(${-rest * 100}%, 0, 0)`, clipPath: none };
    case "slide_up": // incoming enters from the bottom
      return { opacity: 1, transform: `translate3d(0, ${rest * 100}%, 0)`, clipPath: none };
    case "slide_down": // incoming enters from the top
      return { opacity: 1, transform: `translate3d(0, ${-rest * 100}%, 0)`, clipPath: none };
    case "zoom_in":
      return { opacity: p, transform: `scale(${0.6 + 0.4 * p})`, clipPath: none };
    case "wipe_left": // the reveal edge sweeps from the right toward the left
      return { opacity: 1, transform: none, clipPath: `inset(0 0 0 ${rest * 100}%)` };
  }
};
