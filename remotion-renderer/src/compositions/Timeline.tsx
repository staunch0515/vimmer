import { AbsoluteFill, Audio, Sequence } from "remotion";
import { AnimatedText } from "../components/AnimatedText";
import { KenBurnsImage } from "../components/KenBurnsImage";
import { resolveAsset } from "../assets";
import { layoutScenes, msToFrames } from "../layout";
import type { VideoTimeline } from "../types/timeline";

// Renders every scene of the Rust-generated timeline at its frame offset.
export const Timeline: React.FC<VideoTimeline> = (timeline) => {
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
              <Audio src={resolveAsset(scene.audio.source)} />
            </Sequence>
          ) : null}
        </Sequence>
      ))}
    </AbsoluteFill>
  );
};
