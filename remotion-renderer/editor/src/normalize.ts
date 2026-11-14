import type { Scene } from "../../src/types/timeline";

/**
 * Fills the fields the Rust DSL defaults with serde (`#[serde(default)]`) so a scene that omits
 * them is still a complete `Scene`. Optional fields (layers, keyframes, video, ...) stay absent.
 */
export const normalizeScene = (s: Partial<Scene> & Pick<Scene, "id">): Scene => ({
  ...s,
  duration_ms: s.duration_ms ?? 0,
  text: s.text ?? null,
  highlights: s.highlights ?? [],
  image: s.image ?? null,
  motion: s.motion ?? "static",
  audio: s.audio ?? null,
});
