import { useEffect, useRef, useState } from "react";
import { Player, type PlayerRef } from "@remotion/player";
import { Timeline } from "../../../src/compositions/Timeline";
import type { Layer, VideoTimeline } from "../../../src/types/timeline";
import { frameCount } from "../model";
import { outlineHeight } from "../layers";
import { AssetUrlContext, resolveAsset, type AssetUrl } from "../../../src/assets";
import { ClipLengthContext, type ClipLength } from "../../../src/components/ClipVideo";
import { assetUrlFor } from "../media";

type Props = {
  timeline: VideoTimeline;
  assetRoot: string | null;
  playerRef: React.RefObject<PlayerRef | null>;
  onFrame: (frame: number) => void;
  // The selected layer: outlined on the canvas and draggable. Moving it calls onMoveLayer once,
  // when the drag ends, so a drag is one undo step.
  selectedLayer?: Layer | null;
  onMoveLayer?: (x: number, y: number) => void;
};

const round4 = (v: number) => Math.round(v * 10_000) / 10_000;

const LayerHandle = ({ layer, onMove }: { layer: Layer; onMove: (x: number, y: number) => void }) => {
  const [drag, setDrag] = useState<{ dx: number; dy: number } | null>(null);
  const start = useRef<{ px: number; py: number; w: number; h: number } | null>(null);
  const x = layer.x + (drag?.dx ?? 0);
  const y = layer.y + (drag?.dy ?? 0);
  // Auto-height text: outline about one line.
  const height = outlineHeight(layer);
  const transforms = [layer.anchor === "center" ? "translate(-50%, -50%)" : "", layer.rotation ? `rotate(${layer.rotation}deg)` : ""];
  return (
    <div
      className={drag ? "layer-handle dragging" : "layer-handle"}
      data-layer-handle={layer.id}
      style={{
        left: `${x * 100}%`,
        top: `${y * 100}%`,
        width: `${layer.width * 100}%`,
        height: `${Math.max(height, 0.004) * 100}%`,
        transform: transforms.filter(Boolean).join(" ") || undefined,
      }}
      onPointerDown={(e) => {
        const frame = (e.currentTarget.parentElement as HTMLElement).getBoundingClientRect();
        start.current = { px: e.clientX, py: e.clientY, w: frame.width, h: frame.height };
        e.currentTarget.setPointerCapture(e.pointerId);
        setDrag({ dx: 0, dy: 0 });
      }}
      onPointerMove={(e) => {
        const s = start.current;
        if (!s) return;
        setDrag({ dx: (e.clientX - s.px) / s.w, dy: (e.clientY - s.py) / s.h });
      }}
      onPointerUp={() => {
        const moved = drag && (drag.dx !== 0 || drag.dy !== 0);
        start.current = null;
        setDrag(null);
        if (moved) onMove(round4(Math.min(2, Math.max(-1, x))), round4(Math.min(2, Math.max(-1, y))));
      }}
    >
      <span className="layer-handle-label">{layer.id}</span>
    </div>
  );
};

// Every clip src in the timeline (scene backgrounds and video layers).
const clipSources = (timeline: VideoTimeline): string[] => {
  const out = new Set<string>();
  for (const s of timeline.scenes) {
    if (s.video?.src) out.add(s.video.src);
    for (const l of s.layers ?? []) if (l.kind === "video" && l.src) out.add(l.src);
  }
  return [...out];
};

// File lengths of the timeline's clips, read from the browser's metadata, so the preview can loop
// and freeze a clip without trim_end_ms the way the render does (the renderer gets every length
// from the scheduler instead).
const useClipLengths = (timeline: VideoTimeline, assetUrl: AssetUrl | null): ClipLength => {
  const [lengths, setLengths] = useState<Record<string, number>>({});
  const sources = clipSources(timeline).join("\n");
  useEffect(() => {
    const videos: HTMLVideoElement[] = [];
    for (const src of sources ? sources.split("\n") : []) {
      if (lengths[src] !== undefined) continue;
      const v = document.createElement("video");
      v.preload = "metadata";
      v.muted = true;
      v.onloadedmetadata = () => {
        if (Number.isFinite(v.duration)) setLengths((m) => ({ ...m, [src]: Math.round(v.duration * 1000) }));
      };
      v.src = resolveAsset(src, assetUrl);
      videos.push(v);
    }
    return () => videos.forEach((v) => v.removeAttribute("src"));
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [sources, assetUrl]);
  return (src) => lengths[src] ?? null;
};

// The preview is the real Remotion composition running in the Player, so what you see is what renders.
export const Preview = ({ timeline, assetRoot, playerRef, onFrame, selectedLayer, onMoveLayer }: Props) => {
  useEffect(() => {
    const player = playerRef.current;
    if (!player) return;
    const handler = (e: { detail: { frame: number } }) => onFrame(e.detail.frame);
    player.addEventListener("frameupdate", handler);
    return () => player.removeEventListener("frameupdate", handler);
  }, [playerRef, onFrame]);

  const { width, height } = timeline.resolution;
  const assetUrl = assetRoot ? assetUrlFor(assetRoot) : null;
  const clipLength = useClipLengths(timeline, assetUrl);
  return (
    <div className="canvas-slot">
      <div
        className="canvas-frame"
        // Largest box with the composition's aspect ratio that fits the slot.
        style={{ width: `min(100cqw, calc(100cqh * ${width} / ${height}))`, aspectRatio: `${width} / ${height}` }}
      >
      <AssetUrlContext.Provider value={assetUrl}>
      <ClipLengthContext.Provider value={clipLength}>
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
      </ClipLengthContext.Provider>
      </AssetUrlContext.Provider>
      {selectedLayer && onMoveLayer && (
        <div className="layer-overlay">
          <LayerHandle layer={selectedLayer} onMove={onMoveLayer} />
        </div>
      )}
      </div>
    </div>
  );
};
