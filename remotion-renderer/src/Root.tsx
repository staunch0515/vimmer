import type { CalculateMetadataFunction } from "remotion";
import { Composition } from "remotion";
import { Timeline } from "./compositions/Timeline";
import { totalFrames } from "./layout";
import type { VideoTimeline } from "./types/timeline";
// Shared with the Rust tests: this is the 15-second baseline scene.
import sampleTimeline from "../../fixtures/sample-timeline.json";

// Duration, fps and size come from the JSON props, so the Rust core
// controls the output shape without a code change here.
const calculateMetadata: CalculateMetadataFunction<VideoTimeline> = ({ props }) => ({
  durationInFrames: Math.max(1, totalFrames(props)),
  fps: props.fps,
  width: props.resolution.width,
  height: props.resolution.height,
  props,
});

export const RemotionRoot: React.FC = () => {
  return (
    <Composition
      id="Timeline"
      component={Timeline}
      durationInFrames={450}
      fps={30}
      width={1920}
      height={1080}
      defaultProps={sampleTimeline as VideoTimeline}
      calculateMetadata={calculateMetadata}
    />
  );
};
