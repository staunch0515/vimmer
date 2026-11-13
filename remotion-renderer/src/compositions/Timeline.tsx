import { Fragment, useContext } from "react";
import { AbsoluteFill, Audio, Freeze, Sequence, interpolate, useCurrentFrame, useVideoConfig } from "remotion";
import { AnimatedText } from "../components/AnimatedText";
import { Subtitles } from "../components/Subtitles";
import { KenBurnsImage } from "../components/KenBurnsImage";
import { LayerStack } from "../components/LayerStack";
import { AssetUrlContext, resolveAsset } from "../assets";
import { layoutScenes, msToFrames, totalFrames, type SceneLayout } from "../layout";
import { transitionPose } from "../motion/transitions";
import type { Music, Scene, SubtitleCue, Transition, VideoTimeline } from "../types/timeline";

// Gain at a frame: the music volume, faded in and out at the timeline's ends.
const musicVolume = (music: Music, timeline: VideoTimeline) => {
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

// The visual content of one scene, bottom to top: the scene image, layers with z < 0, the scene
// text, layers with z >= 0 (subtitles are drawn over every scene). No audio, which stays with the
// scene. Without layers the DOM is exactly what it was before layers existed.
const SceneBody: React.FC<{ scene: Scene; durationInFrames: number }> = ({ scene, durationInFrames }) => (
  <AbsoluteFill>
    {scene.image ? (
      <KenBurnsImage
        src={scene.image}
        motion={scene.motion}
        keyframes={scene.keyframes}
        durationInFrames={durationInFrames}
      />
    ) : null}
    <LayerStack layers={scene.layers} side="below" sceneFrames={durationInFrames} />
    {scene.text ? <AnimatedText text={scene.text} highlights={scene.highlights} onImage={Boolean(scene.image)} /> : null}
    <LayerStack layers={scene.layers} side="above" sceneFrames={durationInFrames} />
  </AbsoluteFill>
);

// Plays the incoming scene over the held last frame of the previous one (see motion/transitions.ts).
// Rendered inside the incoming scene's own Sequence, so its frame counts from the scene start.
const TransitionIn: React.FC<{ transition: Transition; children: React.ReactNode }> = ({ transition, children }) => {
  const frame = useCurrentFrame();
  const { fps } = useVideoConfig();
  const frames = Math.max(1, msToFrames(transition.duration_ms, fps));
  const progress = interpolate(frame, [0, frames], [0, 1], { extrapolateLeft: "clamp", extrapolateRight: "clamp" });
  const pose = transitionPose(transition.kind, progress);
  return (
    <AbsoluteFill style={{ opacity: pose.opacity, transform: pose.transform, clipPath: pose.clipPath }}>
      {children}
    </AbsoluteFill>
  );
};

// Extra props the scheduler adds to a chunk render (`chunk_props` in vimmer-scheduler/src/plan.rs):
// the chunk's start frame in the full timeline and the full-timeline cues that overlap it. Never
// part of a user timeline; without it, `subtitles` is drawn as is.
type ChunkSubtitles = { start_frame: number; subtitles: SubtitleCue[] };

// Renders every scene of the Rust-generated timeline at its frame offset.
export const Timeline: React.FC<VideoTimeline & { chunk?: ChunkSubtitles | null }> = (timeline) => {
  const assetUrl = useContext(AssetUrlContext);
  const layouts = layoutScenes(timeline);
  return (
    <AbsoluteFill style={{ backgroundColor: "#09090b" }}>
      {layouts.map((layout: SceneLayout, index) => {
        const { scene, from, durationInFrames } = layout;
        const transition = index > 0 ? scene.transition_in : null;
        // The transition overlaps the start of this scene. The previous scene's last frame is
        // held underneath it. The hold sits before this scene's Sequence, so this scene paints on top.
        const hold = transition ? layouts[index - 1] : null;
        const transitionFrames = transition ? Math.max(1, msToFrames(transition.duration_ms, timeline.fps)) : 0;
        return (
          <Fragment key={scene.id}>
            {hold && transition ? (
              <Sequence from={from} durationInFrames={transitionFrames} name={`${hold.scene.id} hold`}>
                <Freeze frame={hold.durationInFrames - 1}>
                  <SceneBody scene={hold.scene} durationInFrames={hold.durationInFrames} />
                </Freeze>
              </Sequence>
            ) : null}
            <Sequence from={from} durationInFrames={durationInFrames} name={scene.id}>
              {transition ? (
                <TransitionIn transition={transition}>
                  <SceneBody scene={scene} durationInFrames={durationInFrames} />
                </TransitionIn>
              ) : (
                <SceneBody scene={scene} durationInFrames={durationInFrames} />
              )}
              {scene.audio ? (
                <Sequence from={msToFrames(scene.audio.offset_ms, timeline.fps)}>
                  <Audio src={resolveAsset(scene.audio.source, assetUrl)} />
                </Sequence>
              ) : null}
            </Sequence>
          </Fragment>
        );
      })}
      {timeline.chunk ? (
        <Subtitles cues={timeline.chunk.subtitles} frameOffset={timeline.chunk.start_frame} />
      ) : (
        <Subtitles cues={timeline.subtitles} />
      )}
      {timeline.music?.source ? (
        // Looped and faded in the preview. The render mixes music in after joining, so chunks omit it.
        <Audio src={resolveAsset(timeline.music.source, assetUrl)} loop volume={musicVolume(timeline.music, timeline)} />
      ) : null}
    </AbsoluteFill>
  );
};
