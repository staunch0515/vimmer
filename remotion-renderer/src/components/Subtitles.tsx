import { AbsoluteFill, useCurrentFrame, useVideoConfig } from "remotion";
import { cjkLang, sceneCutFrames, subtitleAt, type WordState } from "../subtitles/words";
import { BUILTIN, brandColor, fontStack, withAlpha } from "../brand";
import type { Brand, SubtitleCue, VideoTimeline, WordTiming } from "../types/timeline";
import { useBrand } from "./Brand";

// Word-by-word colors: the spoken word stands out, words still to come are dimmed.
const WORD_STYLE: Record<WordState, React.CSSProperties> = {
  spoken: { color: "#fafafa" },
  current: { color: "#facc15", textShadow: "0 0 18px rgba(250, 204, 21, 0.45)" },
  upcoming: { color: "rgba(250, 250, 250, 0.55)" },
};

// The same with brand colors: spoken words in the text color, the current word in the accent,
// upcoming ones in the text color at 55 %. Without brand colors, WORD_STYLE as it always was.
const wordStyles = (brand: Brand | null): Record<WordState, React.CSSProperties> => {
  const text = brand?.colors?.text;
  const accent = brand?.colors?.accent;
  if (!text && !accent) return WORD_STYLE;
  return {
    spoken: text ? { color: text } : WORD_STYLE.spoken,
    current: accent ? { color: accent, textShadow: `0 0 18px ${withAlpha(accent, 0.45)}` } : WORD_STYLE.current,
    upcoming: text ? { color: withAlpha(text, 0.55) } : WORD_STYLE.upcoming,
  };
};

// Plain inline spans, so spaces between English words and line breaking between CJK
// characters work exactly as they do for unsplit text.
const Words: React.FC<{ words: WordTiming[]; states: WordState[]; styles: Record<WordState, React.CSSProperties> }> = ({
  words,
  states,
  styles,
}) => (
  <>
    {words.map((word, i) => (
      <span key={i} style={styles[states[i]]}>
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
  const brand = useBrand();
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
          backgroundColor: brandColor(brand, "subtitle_background", BUILTIN.subtitleBackground),
          border: "1px solid rgba(255, 255, 255, 0.08)",
          color: brandColor(brand, "text", BUILTIN.text),
          fontFamily: fontStack(brand, "body"),
          fontSize,
          fontWeight: 500,
          lineHeight: 1.3,
          textAlign: "center",
          textWrap: "balance",
          overflowWrap: "break-word",
        }}
      >
        {states ? <Words words={active.words ?? []} states={states} styles={wordStyles(brand)} /> : active.text}
      </div>
    </AbsoluteFill>
  );
};
