import { useEffect } from "react";
import { Player, type PlayerRef } from "@remotion/player";
import { Timeline } from "../../../src/compositions/Timeline";
import type { VideoTimeline } from "../../../src/types/timeline";
import { frameCount } from "../model";
import { AssetUrlContext } from "../../../src/assets";
import { assetUrlFor } from "../media";

type Props = {
  timeline: VideoTimeline;
  assetRoot: string | null;
  playerRef: React.RefObject<PlayerRef | null>;
  onFrame: (frame: number) => void;
};

// The preview is the real Remotion composition running in the Player, so what you see is what renders.
export const Preview = ({ timeline, assetRoot, playerRef, onFrame }: Props) => {
  useEffect(() => {
    const player = playerRef.current;
    if (!player) return;
    const handler = (e: { detail: { frame: number } }) => onFrame(e.detail.frame);
    player.addEventListener("frameupdate", handler);
    return () => player.removeEventListener("frameupdate", handler);
  }, [playerRef, onFrame]);

  const { width, height } = timeline.resolution;
  return (
    <div className="canvas-slot">
      <div
        className="canvas-frame"
        // Largest box with the composition's aspect ratio that fits the slot.
        style={{ width: `min(100cqw, calc(100cqh * ${width} / ${height}))`, aspectRatio: `${width} / ${height}` }}
      >
      <AssetUrlContext.Provider value={assetRoot ? assetUrlFor(assetRoot) : null}>
      <Player
        ref={playerRef}
        component={Timeline}
        inputProps={timeline}
        durationInFrames={frameCount(timeline)}
        fps={timeline.fps}
        compositionWidth={width}
        compositionHeight={height}
        controls
        clickToPlay={false}
        style={{ width: "100%", height: "100%" }}
      />
      </AssetUrlContext.Provider>
      </div>
    </div>
  );
};
