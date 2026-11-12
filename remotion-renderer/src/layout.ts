import type { Scene, VideoTimeline } from "./types/timeline";

// Must match the Rust rounding in `Scene::duration_frames` (round half up).
export const msToFrames = (ms: number, fps: number): number =>
  Math.round((ms * fps) / 1000);

export type SceneLayout = {
  scene: Scene;
  from: number;
  durationInFrames: number;
};

// Turns the scene list into absolute frame ranges so each scene can be a <Sequence>.
export const layoutScenes = (timeline: VideoTimeline): SceneLayout[] => {
  let cursor = 0;
  return timeline.scenes.map((scene) => {
    const durationInFrames = msToFrames(scene.duration_ms, timeline.fps);
    const layout = { scene, from: cursor, durationInFrames };
    cursor += durationInFrames;
    return layout;
  });
};

export const totalFrames = (timeline: VideoTimeline): number =>
  layoutScenes(timeline).reduce((sum, l) => sum + l.durationInFrames, 0);
