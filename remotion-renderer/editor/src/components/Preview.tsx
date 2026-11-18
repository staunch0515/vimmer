import { useEffect, useMemo, useRef, useState } from "react";
import { Player, type PlayerRef } from "@remotion/player";
import { Timeline } from "../../../src/compositions/Timeline";
import type { Layer, SubtitleBox, VideoTimeline } from "../../../src/types/timeline";
import { boxOf, snapBox, type BoxSize, type Guides } from "../subtitleBox";
import { frameCount } from "../model";
import { outlineHeight } from "../layers";
import { AssetUrlContext, resolveAsset, type AssetUrl } from "../../../src/assets";
import { ClipLengthContext, type ClipLength } from "../../../src/components/ClipVideo";
import { assetUrlFor } from "../media";
import * as gate from "../mediaGate";
import { usePreviewTimeline } from "../templates";
import { withPreviewAvatars, type AvatarStatus } from "../avatarUi";
import { AudioLengthsContext } from "../../../src/voiceover";
import { PreviewAudioContext, audioPlan } from "../../../src/audioPlan";
import { previewMixer } from "../previewMixer";

type Props = {
  timeline: VideoTimeline;
  assetRoot: string | null;
  playerRef: React.RefObject<PlayerRef | null>;
  onFrame: (frame: number) => void;
  // The selected layer: outlined on the canvas and draggable. Moving it calls onMoveLayer once,
  // when the drag ends, so a drag is one undo step.
  selectedLayer?: Layer | null;
  onMoveLayer?: (x: number, y: number) => void;
  // A subtitle is selected: the subtitle box is outlined and draggable. Moving it calls
  // onMoveSubtitles once, when the drag ends (one undo step).
  subtitlesSelected?: boolean;
  onMoveSubtitles?: (box: SubtitleBox) => void;
  // Measured audio file lengths: voice-over clips play exactly their trimmed length.
  audioLengths?: Record<string, number>;
  // 数字人: which clips exist. The inset shows the clip, else the dimmed photo (avatarUi.ts).
  avatarStatus?: AvatarStatus | null;
};

// The shown subtitle box, in fractions of the frame: left, top, width, height.
type Rect = { l: number; t: number; w: number; h: number };

// Follows the rendered subtitle box (`data-subtitle-box` in Subtitles.tsx) inside the frame, so
// the handle sits exactly on what the Player draws, whatever the cue and the frame.
const useSubtitleRect = (frameRef: React.RefObject<HTMLDivElement | null>, active: boolean): Rect | null => {
  const [rect, setRect] = useState<Rect | null>(null);
  useEffect(() => {
    if (!active) {
      setRect(null);
      return;
    }
    const frame = frameRef.current;
    if (!frame) return;
    const measure = () => {
      const el = frame.querySelector<HTMLElement>("[data-subtitle-box]");
      let next: Rect | null = null;
      if (el) {
        const f = frame.getBoundingClientRect();
        const b = el.getBoundingClientRect();
        const fw = frame.clientWidth;
        const fh = frame.clientHeight;
        next = { l: (b.left - f.left - frame.clientLeft) / fw, t: (b.top - f.top - frame.clientTop) / fh, w: b.width / fw, h: b.height / fh };
      }
      setRect((r) =>
        r === next || (r && next && Math.abs(r.l - next.l) + Math.abs(r.t - next.t) + Math.abs(r.w - next.w) + Math.abs(r.h - next.h) < 1e-5)
          ? r
          : next,
      );
    };
    // The Player rewrites its DOM on every frame and on every prop change (a cue appears, the
    // box moves); the frame resizes with the window. Measure after each.
    const mutations = new MutationObserver(measure);
    mutations.observe(frame, { subtree: true, childList: true, characterData: true, attributes: true, attributeFilter: ["style"] });
    const resize = new ResizeObserver(measure);
    resize.observe(frame);
    measure();
    return () => {
      mutations.disconnect();
      resize.disconnect();
    };
  }, [frameRef, active]);
  return rect;
};

const SubtitleHandle = ({
  rect, box, timeline, frameRef, onDrag, onMove,
}: {
  rect: Rect; box: SubtitleBox; timeline: VideoTimeline; frameRef: React.RefObject<HTMLDivElement | null>;
  onDrag: (box: SubtitleBox | null, guides: Guides | null) => void; onMove: (box: SubtitleBox) => void;
}) => {
  const start = useRef<{ px: number; py: number; fw: number; fh: number; box: SubtitleBox; size: BoxSize; last: SubtitleBox | null } | null>(null);
  return (
    <div
      className={start.current ? "layer-handle subtitle-handle dragging" : "layer-handle subtitle-handle"}
      data-subtitle-handle
      style={{ left: `${rect.l * 100}%`, top: `${rect.t * 100}%`, width: `${rect.w * 100}%`, height: `${rect.h * 100}%` }}
      title="Drag to move the subtitles (all cues). Arrow keys nudge, Shift for 10 px."
      onPointerDown={(e) => {
        const frame = frameRef.current;
        if (!frame) return;
        e.currentTarget.setPointerCapture(e.pointerId);
        start.current = { px: e.clientX, py: e.clientY, fw: frame.clientWidth, fh: frame.clientHeight, box, size: { w: rect.w, h: rect.h }, last: null };
      }}
      onPointerMove={(e) => {
        const s = start.current;
        if (!s) return;
        const wanted = { x: s.box.x + (e.clientX - s.px) / s.fw, y: s.box.y + (e.clientY - s.py) / s.fh };
        const snapped = snapBox(wanted, s.size, timeline.resolution);
        s.last = snapped.box;
        onDrag(snapped.box, snapped.guides);
      }}
      onPointerUp={() => {
        const s = start.current;
        start.current = null;
        onDrag(null, null);
        if (s?.last && (s.last.x !== s.box.x || s.last.y !== s.box.y)) onMove(s.last);
      }}
    >
      <span className="layer-handle-label">subtitles</span>
    </div>
  );
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
export const Preview = ({ timeline, assetRoot, playerRef, onFrame, selectedLayer, onMoveLayer, subtitlesSelected, onMoveSubtitles, audioLengths, avatarStatus }: Props) => {
  const frameRef = useRef<HTMLDivElement>(null);
  // While the subtitles are dragged, the Player draws them at the dragged place.
  const [dragBox, setDragBox] = useState<SubtitleBox | null>(null);
  const [guides, setGuides] = useState<Guides | null>(null);
  const subtitleRect = useSubtitleRect(frameRef, !!subtitlesSelected && !!onMoveSubtitles);
  useEffect(() => {
    const player = playerRef.current;
    if (!player) return;
    const handler = (e: { detail: { frame: number } }) => onFrame(e.detail.frame);
    player.addEventListener("frameupdate", handler);
    return () => player.removeEventListener("frameupdate", handler);
  }, [playerRef, onFrame]);

  // The play gate (mediaGate.ts): a play right after a take is kept waits until the new files have
  // loaded. The Player's own play button is taken over only while the gate holds.
  useEffect(() => {
    const el = frameRef.current;
    const p = () => playerRef.current;
    gate.registerPreview(
      {
        play: () => p()?.play(),
        pause: () => p()?.pause(),
        toggle: () => p()?.toggle(),
        isPlaying: () => p()?.isPlaying() ?? false,
      },
      el,
    );
    const onClick = (e: MouseEvent) => {
      const button = (e.target as Element | null)?.closest?.('button[aria-label="Play video"]');
      if (!button || !gate.holding()) return;
      e.preventDefault();
      e.stopPropagation();
      gate.playWhenSettled();
    };
    el?.addEventListener("click", onClick, true);
    return () => {
      el?.removeEventListener("click", onClick, true);
      gate.registerPreview(null, null);
    };
  }, [playerRef]);

  // The preview's sound plays through Web Audio, not media elements (previewMixer.ts says why).
  // The mixer follows the Player: play, pause, seek, each drawn frame (drift), rate, volume.
  useEffect(() => {
    const player = playerRef.current;
    if (!player) return;
    const m = previewMixer();
    const frame = () => player.getCurrentFrame();
    const onPlay = () => m.play(frame());
    const onPause = () => m.pause(frame());
    const onSeek = (e: { detail: { frame: number } }) => m.seek(e.detail.frame);
    const onFrame = (e: { detail: { frame: number } }) => m.frame(e.detail.frame);
    const onRate = (e: { detail: { playbackRate: number } }) => m.setRate(e.detail.playbackRate);
    const onVolume = () => m.setVolume(player.getVolume(), player.isMuted());
    // The context starts at the first click or key press anywhere (a user gesture), and stays running.
    const unlock = () => m.unlock();
    player.addEventListener("play", onPlay);
    player.addEventListener("pause", onPause);
    player.addEventListener("ended", onPause);
    player.addEventListener("seeked", onSeek);
    player.addEventListener("frameupdate", onFrame);
    player.addEventListener("ratechange", onRate);
    player.addEventListener("volumechange", onVolume);
    player.addEventListener("mutechange", onVolume);
    window.addEventListener("pointerdown", unlock, true);
    window.addEventListener("keydown", unlock, true);
    onVolume();
    if (player.isPlaying()) m.play(frame());
    return () => {
      player.removeEventListener("play", onPlay);
      player.removeEventListener("pause", onPause);
      player.removeEventListener("ended", onPause);
      player.removeEventListener("seeked", onSeek);
      player.removeEventListener("frameupdate", onFrame);
      player.removeEventListener("ratechange", onRate);
      player.removeEventListener("volumechange", onVolume);
      player.removeEventListener("mutechange", onVolume);
      window.removeEventListener("pointerdown", unlock, true);
      window.removeEventListener("keydown", unlock, true);
      m.pause();
    };
  }, [playerRef]);

  // Template scenes are shown expanded by the Rust code the renderer uses (see templates.ts).
  const { timeline: templated, errors: templateErrors } = usePreviewTimeline(timeline);
  const expanded = useMemo(() => withPreviewAvatars(templated, avatarStatus ?? null), [templated, avatarStatus]);
  const shown = dragBox ? { ...expanded, subtitle_box: dragBox } : expanded;
  const { width, height } = timeline.resolution;
  const assetUrl = assetRoot ? assetUrlFor(assetRoot) : null;
  const clipLength = useClipLengths(timeline, assetUrl);
  const durationInFrames = frameCount(timeline);
  // What the composition would play, handed to the mixer (it ignores a plan that sounds the same).
  useEffect(() => {
    previewMixer().setPlan(audioPlan(shown, assetUrl, audioLengths ?? null), timeline.fps, durationInFrames);
  });
  return (
    <div className="canvas-slot">
      <div
        ref={frameRef}
        className="canvas-frame"
        // Largest box with the composition's aspect ratio that fits the slot.
        style={{ width: `min(100cqw, calc(100cqh * ${width} / ${height}))`, aspectRatio: `${width} / ${height}` }}
      >
      <AssetUrlContext.Provider value={assetUrl}>
      <ClipLengthContext.Provider value={clipLength}>
      <AudioLengthsContext.Provider value={audioLengths ?? null}>
      <PreviewAudioContext.Provider value={true}>
      <Player
        ref={playerRef}
        component={Timeline}
        inputProps={shown}
        durationInFrames={durationInFrames}
        fps={timeline.fps}
        compositionWidth={width}
        compositionHeight={height}
        controls
        clickToPlay={false}
        style={{ width: "100%", height: "100%" }}
        // No pooled <audio> tags: the composition draws none in the preview, and the Player would
        // otherwise play() these empty tags on every click.
        numberOfSharedAudioTags={0}
      />
      </PreviewAudioContext.Provider>
      </AudioLengthsContext.Provider>
      </ClipLengthContext.Provider>
      </AssetUrlContext.Provider>
      {templateErrors.length > 0 && (
        <div className="template-errors" role="status">{templateErrors.join("  ·  ")}</div>
      )}
      {selectedLayer && onMoveLayer && (
        <div className="layer-overlay">
          <LayerHandle layer={selectedLayer} onMove={onMoveLayer} />
        </div>
      )}
      {subtitlesSelected && onMoveSubtitles && (
        <div className="layer-overlay" data-subtitle-overlay>
          {guides && (
            <>
              {/* The title-safe area, and the lines the box snapped to. */}
              <div className="safe-area-guide" />
              {guides.x.map((x) => <div key={`gx${x}`} className="snap-line v" style={{ left: `${x * 100}%` }} />)}
              {guides.y.map((y) => <div key={`gy${y}`} className="snap-line h" style={{ top: `${y * 100}%` }} />)}
            </>
          )}
          {subtitleRect && (
            <SubtitleHandle
              rect={subtitleRect}
              box={boxOf(timeline.subtitle_box, timeline.resolution)}
              timeline={timeline}
              frameRef={frameRef}
              onDrag={(b, g) => {
                setDragBox(b);
                setGuides(g);
              }}
              onMove={onMoveSubtitles}
            />
          )}
        </div>
      )}
      </div>
    </div>
  );
};
