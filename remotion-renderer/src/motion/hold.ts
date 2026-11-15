// The held frame under a transition (see transitions.ts): while the incoming scene blends in, the
// previous scene's LAST frame is shown still underneath it. This module is pure: it says which
// composition frames show the hold and which scene frame is held.
//
// Why the renderer does not wrap the hold in a <Sequence from={start}>: Remotion's <Freeze> stores
// the frozen clock as `frame + the from of its nearest <Sequence>` and clamps that value to the
// composition length. Under a Sequence starting at the incoming scene, the value is
// `start + previous scene length - 1`, which runs past the end of a short composition (a render
// chunk that ends soon after the transition, or a full timeline whose last scenes are shorter
// than the held one). Remotion then clamps it and the hold shows an earlier frame of the previous
// scene: a visible jump at the start of the transition. Outside any Sequence the stored value is
// just `freezeFrame`, which is always inside the composition, because the held scene lies before
// the incoming one.

export type Span = { from: number; durationInFrames: number };

export type HoldWindow = {
  // First composition frame that shows the hold (the incoming scene's first frame).
  start: number;
  // First composition frame after it.
  end: number;
  // Frame of the previous scene that is held (its last).
  freezeFrame: number;
};

// The hold for the scene at `incoming` (index > 0) with a transition of `transitionFrames`.
export const holdWindow = (previous: Span, incoming: Span, transitionFrames: number): HoldWindow => ({
  start: incoming.from,
  end: incoming.from + Math.max(1, transitionFrames),
  freezeFrame: Math.max(0, previous.durationInFrames - 1),
});

export const inHoldWindow = (w: HoldWindow, frame: number): boolean => frame >= w.start && frame < w.end;

// The clock value Remotion's <Freeze> keeps for `freezeFrame` under a Sequence starting at
// `sequenceFrom` (0 when there is none), clamped like Remotion clamps it to the composition. The
// frame its children see is this value minus `sequenceFrom`.
export const frozenFrameSeen = (freezeFrame: number, sequenceFrom: number, compositionFrames: number): number =>
  Math.max(0, Math.min(Math.max(0, compositionFrames - 1), freezeFrame + sequenceFrom)) - sequenceFrom;
