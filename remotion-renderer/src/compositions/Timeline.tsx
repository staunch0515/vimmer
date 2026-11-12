import { useContext } from "react";
import { AbsoluteFill, Audio, Sequence, interpolate } from "remotion";
import { AnimatedText } from "../components/AnimatedText";
import { Subtitles } from "../components/Subtitles";
import { KenBurnsImage } from "../components/KenBurnsImage";
import { AssetUrlContext, resolveAsset } from "../assets";
import { layoutScenes, msToFrames, totalFrames } from "../layout";
import type { Music, VideoTimeline } from "../types/timeline";

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

// Renders every scene of the Rust-generated timeline at its frame offset.
export const Timeline: React.FC<VideoTimeline> = (timeline) => {
  const assetUrl = useContext(AssetUrlContext);
  return (
    <AbsoluteFill style={{ backgroundColor: "#09090b" }}>
      {layoutScenes(timeline).map(({ scene, from, durationInFrames }) => (
        <Sequence key={scene.id} from={from} durationInFrames={durationInFrames} name={scene.id}>
          <AbsoluteFill>
            {scene.image ? (
              <KenBurnsImage
                src={scene.image}
                motion={scene.motion}
                durationInFrames={durationInFrames}
              />
            ) : null}
            {scene.text ? <AnimatedText text={scene.text} highlights={scene.highlights} /> : null}
          </AbsoluteFill>
          {scene.audio ? (
            <Sequence from={msToFrames(scene.audio.offset_ms, timeline.fps)}>
              <Audio src={resolveAsset(scene.audio.source, assetUrl)} />
            </Sequence>
          ) : null}
        </Sequence>
      ))}
      <Subtitles cues={timeline.subtitles} />
      {timeline.music?.source ? (
        // Looped and faded in the preview. The render mixes music in after joining, so chunks omit it.
        <Audio src={resolveAsset(timeline.music.source, assetUrl)} loop volume={musicVolume(timeline.music, timeline)} />
      ) : null}
    </AbsoluteFill>
  );
};
