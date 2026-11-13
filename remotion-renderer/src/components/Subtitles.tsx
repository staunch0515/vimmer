import { AbsoluteFill, useCurrentFrame, useVideoConfig } from "remotion";
import { cjkLang, sceneCutFrames, subtitleAt, type WordState } from "../subtitles/words";
import type { SubtitleCue, VideoTimeline, WordTiming } from "../types/timeline";

// Word-by-word colors: the spoken word stands out, words still to come are dimmed.
const WORD_STYLE: Record<WordState, React.CSSProperties> = {
  spoken: { color: "#fafafa" },
  current: { color: "#facc15", textShadow: "0 0 18px rgba(250, 204, 21, 0.45)" },
  upcoming: { color: "rgba(250, 250, 250, 0.55)" },
};

// Plain inline spans, so spaces between English words and line breaking between CJK
// characters work exactly as they do for unsplit text.
const Words: React.FC<{ words: WordTiming[]; states: WordState[] }> = ({ words, states }) => (
  <>
    {words.map((word, i) => (
      <span key={i} style={WORD_STYLE[states[i]]}>
        {word.text}
      </span>
    ))}
  </>
);

type Props = {
  cues: SubtitleCue[];
  // Frames where a word's highlight stops (sceneCutFrames), in composition frames. Defaults to
  // the scenes of the composition's timeline props.
  sceneStarts?: number[];
  // Added to the composition frame (and to sceneStarts) before matching `cues`. A chunked render
  // passes the chunk's start frame with full-timeline cues, so every frame is drawn exactly like
  // the same frame of a full render: no fade-out/fade-in blink at a chunk seam.
  frameOffset?: number;
};

// Shows the cue whose frame range contains the current frame. Frame ranges come from the
// same rounding as the Rust side, so a cue starts on the exact frame the DSL specifies.
// A cue with `words` is drawn word by word with the spoken word highlighted; without words, or
// when the words no longer spell the cue text (edited after timing), it is drawn as one line.
export const Subtitles: React.FC<Props> = ({ cues, sceneStarts, frameOffset = 0 }) => {
  const { fps, width, height, props } = useVideoConfig();
  const frame = useCurrentFrame() + frameOffset;
  const scenes = (props as Partial<VideoTimeline>).scenes;
  const cuts = (sceneStarts ?? (Array.isArray(scenes) ? sceneCutFrames(scenes, fps) : [])).map((c) => c + frameOffset);

  const shown = subtitleAt(cues, frame, fps, cuts);
  if (!shown) {
    return null;
  }
  const { cue: active, opacity, states } = shown;

  // Scale with the shorter side so the size reads the same in landscape and portrait.
  const fontSize = Math.round(Math.min(width, height) * 0.05);

  return (
    <AbsoluteFill
      style={{ justifyContent: "flex-end", alignItems: "center", paddingBottom: "4%", pointerEvents: "none" }}
    >
      <div
        lang={states ? cjkLang(active.text) : undefined}
        style={{
          opacity,
          maxWidth: "86%",
          padding: `${Math.round(fontSize * 0.3)}px ${Math.round(fontSize * 0.6)}px`,
          borderRadius: fontSize * 0.4,
          backgroundColor: "rgba(9, 9, 11, 0.72)",
          border: "1px solid rgba(255, 255, 255, 0.08)",
          color: "#fafafa",
          fontFamily: "Inter, 'Geist', system-ui, sans-serif",
          fontSize,
          fontWeight: 500,
          lineHeight: 1.3,
          textAlign: "center",
          textWrap: "balance",
          overflowWrap: "break-word",
        }}
      >
        {states ? <Words words={active.words ?? []} states={states} /> : active.text}
      </div>
    </AbsoluteFill>
  );
};
