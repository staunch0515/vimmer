// When a sound effect plays, in frames from its scene's start. Mirrors `SoundEffect::frame_in`
// in rust-core/crates/vimmer-dsl/src/model.rs, which places it in the rendered soundtrack: an
// enter on the layer's first frame, an exit on the first frame of its exit animation (or its
// end frame without one), the same frames motion/layers.ts animates.
import type { Scene, SoundEffect } from "../types/timeline";

// DEFAULT_ANIMATION_MS of motion/layers.ts (repeated: this module only imports types, so the
// Node test runner can load it without a bundler).
const DEFAULT_ANIMATION_MS = 400;

const msToFrames = (ms: number, fps: number): number => Math.round((ms * fps) / 1000);

export const DEFAULT_SFX_VOLUME = 0.8;

// Never later than the scene's last frame (a time that rounds onto the cut plays just before it).
export const sfxFrame = (fx: SoundEffect, scene: Scene, fps: number): number | null => {
  const sceneFrames = msToFrames(scene.duration_ms, fps);
  const last = (f: number) => Math.min(f, Math.max(0, sceneFrames - 1));
  if (fx.at_ms != null) return last(msToFrames(fx.at_ms, fps));
  if (!fx.on) return null;
  const layer = (scene.layers ?? []).find((l) => l.id === fx.on?.layer);
  if (!layer) return null;
  const start = msToFrames(layer.start_ms ?? 0, fps);
  if (fx.on.event === "enter") return last(start);
  const end = layer.end_ms == null ? sceneFrames : Math.min(sceneFrames, msToFrames(layer.end_ms, fps));
  if (!layer.exit) return last(end);
  const n = Math.max(1, msToFrames(layer.exit.duration_ms ?? DEFAULT_ANIMATION_MS, fps));
  return last(Math.max(start, end - n));
};
