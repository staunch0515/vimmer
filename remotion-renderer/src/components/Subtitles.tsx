import { AbsoluteFill, interpolate, useCurrentFrame, useVideoConfig } from "remotion";
import { msToFrames } from "../layout";
import { cjkLang, wordStates, type WordState } from "../subtitles/words";
import type { SubtitleCue, WordTiming } from "../types/timeline";

const FADE_FRAMES = 6;

// Word-by-word colors: the spoken word stands out, words still to come are dimmed.
const WORD_STYLE: Record<WordState, React.CSSProperties> = {
  spoken: { color: "#fafafa" },
  current: { color: "#facc15", textShadow: "0 0 18px rgba(250, 204, 21, 0.45)" },
  upcoming: { color: "rgba(250, 250, 250, 0.55)" },
};

// Plain inline spans, so spaces between English words and line breaking between CJK
// characters work exactly as they do for unsplit text.
const Words: React.FC<{ words: WordTiming[]; frame: number; fps: number }> = ({ words, frame, fps }) => {
  const states = wordStates(words, frame, fps);
  return (
    <>
      {words.map((word, i) => (
        <span key={i} style={WORD_STYLE[states[i]]}>
          {word.text}
        </span>
      ))}
    </>
  );
};

type Props = {
  cues: SubtitleCue[];
};

// Shows the cue whose frame range contains the current frame. Frame ranges come from the
// same rounding as the Rust side, so a cue starts on the exact frame the DSL specifies.
// A cue with `words` is drawn word by word with the spoken word highlighted; without words it
// is drawn as one line, as before.
export const Subtitles: React.FC<Props> = ({ cues }) => {
  const frame = useCurrentFrame();
  const { fps, width, height } = useVideoConfig();

  const active = cues.find(
    (c) => frame >= msToFrames(c.start_ms, fps) && frame < msToFrames(c.end_ms, fps),
  );
  if (!active) {
    return null;
  }

  const start = msToFrames(active.start_ms, fps);
  const end = msToFrames(active.end_ms, fps);
  const fadeIn = interpolate(frame, [start, start + FADE_FRAMES], [0, 1], {
    extrapolateLeft: "clamp",
    extrapolateRight: "clamp",
  });
  const fadeOut = interpolate(frame, [end - FADE_FRAMES, end], [1, 0], {
    extrapolateLeft: "clamp",
    extrapolateRight: "clamp",
  });

  const hasWords = (active.words?.length ?? 0) > 0;

  // Scale with the shorter side so the size reads the same in landscape and portrait.
  const fontSize = Math.round(Math.min(width, height) * 0.05);

  return (
    <AbsoluteFill
      style={{ justifyContent: "flex-end", alignItems: "center", paddingBottom: "4%", pointerEvents: "none" }}
    >
      <div
        lang={hasWords ? cjkLang(active.text) : undefined}
        style={{
          opacity: Math.min(fadeIn, fadeOut),
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
        {hasWords ? <Words words={active.words ?? []} frame={frame} fps={fps} /> : active.text}
      </div>
    </AbsoluteFill>
  );
};
