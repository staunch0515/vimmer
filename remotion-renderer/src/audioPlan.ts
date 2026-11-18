import { createContext } from "react";
import { interpolate } from "remotion";
import { resolveAsset, type AssetUrl } from "./assets";
import { layoutScenes, msToFrames, totalFrames } from "./layout";
import { DEFAULT_SFX_VOLUME, sfxFrame } from "./motion/sfx";
import { clipFrames, clipGain } from "./voiceover";
import type { Music, VideoTimeline } from "./types/timeline";

// The preview's sound, as data. The composition places one Remotion <Audio> per sound; in the
// editor's preview those media elements are replaced by a Web Audio mixer (editor/src/previewMixer.ts),
// because WKWebView's media elements can deadlock its WebContent and GPU processes (see
// side-session notes preview-audio.md). The mixer plays exactly this plan.

/**
 * True only inside the editor's Player: the composition then draws no <Audio> and the editor's
 * Web Audio mixer plays `audioPlan(...)` instead. Renders never set it, so render output is unchanged.
 */
export const PreviewAudioContext = createContext<boolean>(false);

/** Gain at a frame: the music volume, faded in and out at the timeline's ends. */
export const musicVolume = (music: Music, timeline: VideoTimeline) => {
  const total = totalFrames(timeline);
  const fadeIn = msToFrames(music.fade_in_ms, timeline.fps);
  const fadeOut = msToFrames(music.fade_out_ms, timeline.fps);
  return (frame: number): number => {
    const inGain = fadeIn > 0 ? interpolate(frame, [0, fadeIn], [0, 1], { extrapolateLeft: "clamp", extrapolateRight: "clamp" }) : 1;
    const outGain =
      fadeOut > 0 ? interpolate(frame, [total - fadeOut, total], [1, 0], { extrapolateLeft: "clamp", extrapolateRight: "clamp" }) : 1;
    return music.volume * Math.min(inGain, outGain);
  };
};

/** One sound of the preview, in timeline frames. */
export type AudioItem = {
  key: string;
  url: string;
  /** First frame it sounds on. */
  from: number;
  /** Frame it stops at (exclusive); null = when the file ends (or the composition does). */
  until: number | null;
  /** File position at `from`, in frames. */
  fileStart: number;
  /**
   * Gain: a number, or a function of the frame passed to Remotion's volume prop. For a looped
   * sound that frame restarts with every loop, as Remotion's `<Audio loop>` does ("repeat").
   */
  gain: number | ((frame: number) => number);
  /** Loops forever (music). Remotion loops every floor(file length × fps) frames. */
  loop: boolean;
};

/** Every sound the composition would play, exactly where its <Audio> sits. */
export const audioPlan = (
  timeline: VideoTimeline,
  assetUrl: AssetUrl | null,
  lengths: Record<string, number> | null,
): AudioItem[] => {
  const out: AudioItem[] = [];
  const layouts = layoutScenes(timeline);
  // Scene audio: <Sequence from durationInFrames> › <Sequence from={offset}> › <Audio>.
  for (const { scene, from, durationInFrames } of layouts) {
    if (!scene.audio) continue;
    const start = from + msToFrames(scene.audio.offset_ms, timeline.fps);
    const first = Math.max(start, from);
    const until = from + durationInFrames;
    if (first >= until) continue;
    out.push({ key: `scene:${scene.id}`, url: resolveAsset(scene.audio.source, assetUrl), from: first, until, fileStart: first - start, gain: 1, loop: false });
  }
  // Sound effects: from the effect's frame until the file ends (they ring out past the cut).
  for (const { scene, from } of layouts) {
    (scene.sfx ?? []).forEach((fx, k) => {
      const at = sfxFrame(fx, scene, timeline.fps);
      if (at == null) return;
      out.push({ key: `sfx:${scene.id}:${k}`, url: resolveAsset(fx.src, assetUrl), from: from + at, until: null, fileStart: 0, gain: fx.volume ?? DEFAULT_SFX_VOLUME, loop: false });
    });
  }
  // Voice-over clips: trimmed at the start, cut after their played length when it is known.
  for (const clip of timeline.voiceover ?? []) {
    const f = clipFrames(clip, timeline.fps, lengths?.[clip.source]);
    if (f.durationInFrames === 0) continue;
    out.push({
      key: `vo:${clip.id}`,
      url: resolveAsset(clip.source, assetUrl),
      from: f.from,
      until: f.durationInFrames == null ? null : f.from + f.durationInFrames,
      fileStart: f.trimBefore,
      gain: clipGain(clip),
      loop: false,
    });
  }
  if (timeline.music?.source) {
    out.push({ key: "music", url: resolveAsset(timeline.music.source, assetUrl), from: 0, until: null, fileStart: 0, gain: musicVolume(timeline.music, timeline), loop: true });
  }
  return out;
};
