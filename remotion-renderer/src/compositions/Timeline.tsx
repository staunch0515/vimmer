import { Fragment, useContext } from "react";
import { AbsoluteFill, Audio, Freeze, Sequence, interpolate, useCurrentFrame, useVideoConfig } from "remotion";
import { AnimatedText } from "../components/AnimatedText";
import { Subtitles } from "../components/Subtitles";
import { KenBurnsImage, KenBurnsVideo } from "../components/KenBurnsImage";
import { LayerStack } from "../components/LayerStack";
import { AssetUrlContext, resolveAsset } from "../assets";
import { AudioLengthsContext, clipFrames, clipGain } from "../voiceover";
import { BUILTIN, brandColor } from "../brand";
import { BrandContext, BrandOverlay, useBrand } from "../components/Brand";
import { scenePages } from "../textPages";
import { layoutScenes, msToFrames, totalFrames, type SceneLayout } from "../layout";
import { transitionPose } from "../motion/transitions";
import { holdWindow, inHoldWindow, type HoldWindow } from "../motion/hold";
import { DEFAULT_SFX_VOLUME, sfxFrame } from "../motion/sfx";
import { LookedScene } from "../components/Atmosphere";
import { autoPreset, drawsAnything, effectiveLook, type GradePreset } from "../motion/look";
import type { Look, Music, Scene, SubtitleCue, Transition, VideoTimeline } from "../types/timeline";

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

// The visual content of one scene, bottom to top: the scene image (or video), layers with z < 0, the scene
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
    ) : scene.video ? (
      <KenBurnsVideo
        clip={scene.video}
        motion={scene.motion}
        keyframes={scene.keyframes}
        durationInFrames={durationInFrames}
      />
    ) : null}
    <LayerStack layers={scene.layers} side="below" sceneFrames={durationInFrames} />
    {scene.text ? (
      <SceneText scene={scene} />
    ) : null}
    <LayerStack layers={scene.layers} side="above" sceneFrames={durationInFrames} />
  </AbsoluteFill>
);

// The scene text, paged when its layout asks for it (src/textPages.ts): the render's timed
// `text_pages`, else pages timed by characters over the scene (the editor preview).
const SceneText: React.FC<{ scene: Scene }> = ({ scene }) => {
  const { width, height } = useVideoConfig();
  const brand = useBrand();
  return (
    <AnimatedText
      text={scene.text ?? ""}
      highlights={scene.highlights}
      onImage={Boolean(scene.image || scene.video)}
      textStyle={scene.text_style}
      layout={scene.text_layout}
      pages={scenePages(scene, brand, { width, height })}
      sceneId={scene.id}
    />
  );
};

// How a scene's look is drawn: the timeline default, what `auto` stands for, and the scene's first
// frame in the full timeline (chunk offset included).
type LookContext = { defaultLook: Look | null | undefined; autoAs: GradePreset; startFrame: number };

// A scene with its look (components/Atmosphere.tsx). Without a look that draws something, exactly
// the SceneBody of before: no extra element, no filter.
const SceneVisual: React.FC<{ scene: Scene; durationInFrames: number; look: LookContext }> = ({ scene, durationInFrames, look }) => {
  const body = <SceneBody scene={scene} durationInFrames={durationInFrames} />;
  if (!look.defaultLook && !scene.look) return body;
  const effective = effectiveLook(look.defaultLook, scene.look);
  if (!drawsAnything(effective, look.autoAs)) return body;
  return (
    <LookedScene look={effective} autoAs={look.autoAs} startFrame={look.startFrame} sceneId={scene.id}>
      {body}
    </LookedScene>
  );
};

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

// The previous scene's last frame, still, under the start of a transition (see motion/hold.ts).
// Deliberately not inside a <Sequence>: there, Remotion would clamp the frozen frame to the
// composition length and hold an earlier frame whenever the composition ends soon after the
// transition (a render chunk, or a timeline whose last scenes are short).
const HeldFrame: React.FC<{ hold: HoldWindow; scene: Scene; durationInFrames: number; look: LookContext }> = ({
  hold,
  scene,
  durationInFrames,
  look,
}) => {
  const frame = useCurrentFrame();
  if (!inHoldWindow(hold, frame)) return null;
  return (
    <AbsoluteFill>
      <Freeze frame={hold.freezeFrame}>
        {/* The held scene keeps its own look (its overlays stand still with the frame). */}
        <SceneVisual scene={scene} durationInFrames={durationInFrames} look={look} />
      </Freeze>
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
  const lengths = useContext(AudioLengthsContext);
  const layouts = layoutScenes(timeline);
  const brand = timeline.brand ?? null;
  const chunkStart = timeline.chunk?.start_frame ?? 0;
  const autoAs = autoPreset(timeline);
  const lookAt = (from: number): LookContext => ({ defaultLook: timeline.look, autoAs, startFrame: chunkStart + from });
  return (
    <BrandContext.Provider value={brand}>
    <AbsoluteFill style={{ backgroundColor: brandColor(brand, "background", BUILTIN.background) }}>
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
              <HeldFrame
                hold={holdWindow(hold, layout, transitionFrames)}
                scene={hold.scene}
                durationInFrames={hold.durationInFrames}
                look={lookAt(hold.from)}
              />
            ) : null}
            <Sequence from={from} durationInFrames={durationInFrames} name={scene.id}>
              {transition ? (
                <TransitionIn transition={transition}>
                  <SceneVisual scene={scene} durationInFrames={durationInFrames} look={lookAt(from)} />
                </TransitionIn>
              ) : (
                <SceneVisual scene={scene} durationInFrames={durationInFrames} look={lookAt(from)} />
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
      {/* The brand logo sits above every scene's layers and below the subtitles. */}
      <BrandOverlay timeline={timeline} />
      {timeline.chunk ? (
        <Subtitles cues={timeline.chunk.subtitles} frameOffset={timeline.chunk.start_frame} box={timeline.subtitle_box} />
      ) : (
        <Subtitles cues={timeline.subtitles} box={timeline.subtitle_box} />
      )}
      {/* Sound effects for the preview (renders are muted; the Rust core mixes them). Outside the
          scene Sequences so they ring out past the cut, as in the render. */}
      {layouts.flatMap(({ scene, from }) =>
        (scene.sfx ?? []).map((fx, k) => {
          const at = sfxFrame(fx, scene, timeline.fps);
          return at == null ? null : (
            <Sequence key={`${scene.id}-sfx-${k}`} from={from + at} name={`${scene.id} sfx ${k}`} layout="none">
              <Audio src={resolveAsset(fx.src, assetUrl)} volume={fx.volume ?? DEFAULT_SFX_VOLUME} />
            </Sequence>
          );
        }),
      )}
      {/* The voice-over track (旁白音轨), for the preview; the render mixes it after joining. */}
      {(timeline.voiceover ?? []).map((clip) => {
        const f = clipFrames(clip, timeline.fps, lengths?.[clip.source]);
        if (f.durationInFrames === 0) return null;
        return (
          <Sequence
            key={`voiceover-${clip.id}`}
            from={f.from}
            durationInFrames={f.durationInFrames ?? undefined}
            name={`voice-over ${clip.id}`}
            layout="none"
          >
            <Audio src={resolveAsset(clip.source, assetUrl)} trimBefore={f.trimBefore || undefined} volume={clipGain(clip)} />
          </Sequence>
        );
      })}
      {timeline.music?.source ? (
        // Looped and faded in the preview. The render mixes music in after joining, so chunks omit it.
        <Audio src={resolveAsset(timeline.music.source, assetUrl)} loop volume={musicVolume(timeline.music, timeline)} />
      ) : null}
    </AbsoluteFill>
    </BrandContext.Provider>
  );
};
