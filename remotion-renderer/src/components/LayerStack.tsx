import { AbsoluteFill, useCurrentFrame, useVideoConfig } from "remotion";
import { applyEasing, sampleKeyframes } from "../motion/keyframes";
import { layerFrames, layerStateAt, splitLayers } from "../motion/layers";
import type { Layer } from "../types/timeline";
import { ClipVideo } from "./ClipVideo";
import {
  BarChartView,
  CounterView,
  IconLayerView,
  ImageLayerView,
  LowerThirdView,
  ProgressRingView,
  ShapeLayerView,
  TextLayerView,
} from "./LayerKinds";

// One layer in its box. Rendered inside the scene's Sequence, so `useCurrentFrame` is the
// scene-relative frame (frozen on the last frame while a transition holds the scene).
const LayerBox: React.FC<{ layer: Layer; sceneFrames: number }> = ({ layer, sceneFrames }) => {
  const frame = useCurrentFrame();
  const { fps, width, height } = useVideoConfig();
  const kf = layer.keyframes && layer.keyframes.length > 0 ? sampleKeyframes(layer.keyframes, (frame * 1000) / fps) : undefined;
  const state = layerStateAt(layer, frame, fps, sceneFrames, width, height, kf, applyEasing);
  if (!state.visible) return null;
  const span = layerFrames(layer, fps, sceneFrames);
  const local = frame - span.start;
  const ctx = { frameHeight: height, frame: local, span: span.end - span.start, fps };
  const boxHeight = state.height;
  let content: React.ReactNode;
  switch (layer.kind) {
    case "image":
      content = <ImageLayerView layer={layer} {...ctx} />;
      break;
    case "video":
      content = <ClipVideo clip={layer} frame={local} span={ctx.span} />;
      break;
    case "text":
      content = <TextLayerView layer={layer} boxHeight={boxHeight} {...ctx} />;
      break;
    case "shape":
      content = <ShapeLayerView layer={layer} width={state.width} height={boxHeight ?? 0} {...ctx} />;
      break;
    case "icon":
      content = <IconLayerView layer={layer} {...ctx} />;
      break;
    case "lower_third":
      content = <LowerThirdView layer={layer} boxHeight={boxHeight} {...ctx} />;
      break;
    case "counter":
      content = <CounterView layer={layer} boxHeight={boxHeight} {...ctx} />;
      break;
    case "bar_chart":
      content = <BarChartView layer={layer} width={state.width} height={boxHeight ?? 0} {...ctx} />;
      break;
    case "progress_ring":
      content = <ProgressRingView layer={layer} width={state.width} height={boxHeight ?? 0} {...ctx} />;
      break;
    default:
      // A kind this renderer does not know (a newer timeline): draw nothing rather than fail.
      return null;
  }
  return (
    <div
      data-layer-id={layer.id}
      style={{
        position: "absolute",
        left: state.left,
        top: state.top,
        width: state.width,
        height: boxHeight ?? undefined,
        transform: state.transform,
        transformOrigin: "center center",
        opacity: state.opacity,
      }}
    >
      {content}
    </div>
  );
};

// The scene's layers on one side of the scene text: `below` (z < 0) or `above` (z >= 0).
// Renders nothing at all when there are none, so a scene without layers is unchanged.
export const LayerStack: React.FC<{ layers: Layer[] | undefined; side: "below" | "above"; sceneFrames: number }> = ({
  layers,
  side,
  sceneFrames,
}) => {
  if (!layers || layers.length === 0) return null;
  const list = splitLayers(layers)[side];
  if (list.length === 0) return null;
  return (
    <AbsoluteFill style={{ pointerEvents: "none" }}>
      {list.map((layer) => (
        <LayerBox key={layer.id} layer={layer} sceneFrames={sceneFrames} />
      ))}
    </AbsoluteFill>
  );
};
