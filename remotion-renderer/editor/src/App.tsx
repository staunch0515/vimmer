import { useEffect, useMemo, useReducer, useRef, useState } from "react";
import type { PlayerRef } from "@remotion/player";
import sample from "../../../fixtures/sample-timeline.json";
import type { VideoTimeline } from "../../src/types/timeline";
import { checkTimeline, frameCount, frameToMs, timecode, totalMs, type Selection } from "./model";
import { reducer, type EditorState } from "./state";
import { Toolbar } from "./components/Toolbar";
import { Preview } from "./components/Preview";
import { Inspector } from "./components/Inspector";
import { EditorTimeline } from "./components/EditorTimeline";

const STORAGE_KEY = "vimmer-studio-timeline";

// Restores the last session's draft if present. Storage can be unavailable (private mode), so every access is guarded.
const loadInitial = (): VideoTimeline => {
  try {
    const saved = window.localStorage.getItem(STORAGE_KEY);
    if (saved) return JSON.parse(saved) as VideoTimeline;
  } catch {
    /* fall through to the sample */
  }
  return { ...(sample as VideoTimeline), subtitles: (sample as VideoTimeline).subtitles ?? [] };
};

const initial: EditorState = {
  timeline: loadInitial(),
  selection: { kind: "none" },
  history: [],
  future: [],
};

export const App = () => {
  const [state, dispatch] = useReducer(reducer, initial);
  const [frame, setFrame] = useState(0);
  const [tab, setTab] = useState<"scene" | "subtitles">("scene");
  const playerRef = useRef<PlayerRef>(null);
  const { timeline } = state;
  const problems = useMemo(() => checkTimeline(timeline), [timeline]);
  const fps = timeline.fps;

  // Autosave draft. Failure is non-fatal: the editor keeps working without persistence.
  useEffect(() => {
    try {
      window.localStorage.setItem(STORAGE_KEY, JSON.stringify(timeline));
    } catch {
      /* quota or disabled storage */
    }
  }, [timeline]);

  // Keep the playhead inside the timeline when edits shorten it.
  useEffect(() => {
    const max = frameCount(timeline) - 1;
    if (frame > max) playerRef.current?.seekTo(max);
  }, [timeline, frame]);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      const target = e.target as HTMLElement;
      const typing = target.matches("input, textarea, select, [contenteditable]");
      const mod = e.metaKey || e.ctrlKey;
      if (mod && e.key.toLowerCase() === "z" && !typing) {
        e.preventDefault();
        dispatch({ type: e.shiftKey ? "redo" : "undo" });
      } else if (mod && e.key.toLowerCase() === "y" && !typing) {
        e.preventDefault();
        dispatch({ type: "redo" });
      } else if (e.code === "Space" && !typing) {
        e.preventDefault();
        playerRef.current?.toggle();
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, []);

  const select = (selection: Selection) => {
    dispatch({ type: "select", selection });
    if (selection.kind === "subtitle") setTab("subtitles");
    if (selection.kind === "scene") setTab("scene");
  };

  const seek = (f: number) => {
    const clamped = Math.max(0, Math.min(f, frameCount(timeline) - 1));
    playerRef.current?.seekTo(clamped);
    setFrame(clamped);
  };

  const addSubtitleAtPlayhead = () => {
    dispatch({ type: "addSubtitle", startMs: frameToMs(frame, fps) });
    setTab("subtitles");
  };

  return (
    <div className="studio">
      <Toolbar
        timeline={timeline}
        problems={problems}
        canUndo={state.history.length > 0}
        canRedo={state.future.length > 0}
        onUndo={() => dispatch({ type: "undo" })}
        onRedo={() => dispatch({ type: "redo" })}
        onLoad={(t) => dispatch({ type: "load", timeline: t })}
        onResolution={(w, h) => dispatch({ type: "setResolution", width: w, height: h })}
      />
      {problems.length > 0 && (
        <div className="problems" role="alert">
          <strong>{problems.length} issue{problems.length > 1 ? "s" : ""}</strong>
          <span>{problems.slice(0, 2).join("  ·  ")}{problems.length > 2 ? "  …" : ""}</span>
        </div>
      )}
      <main className="workspace">
        <section className="stage">
          <Preview timeline={timeline} playerRef={playerRef} onFrame={setFrame} />
          <div className="stage-meta">
            <span className="mono">{timecode(frameToMs(frame, fps))}</span>
            <span className="dim mono">/ {timecode(totalMs(timeline))}</span>
            <span className="dim">frame {frame} of {frameCount(timeline)}</span>
            <span className="spacer" />
            <span className="dim">Space play · ⌘Z undo</span>
          </div>
        </section>
        <Inspector
          timeline={timeline}
          selection={state.selection}
          tab={tab}
          setTab={setTab}
          playheadMs={frameToMs(frame, fps)}
          dispatch={dispatch}
          onSelect={select}
          onAddSubtitle={addSubtitleAtPlayhead}
        />
      </main>
      <EditorTimeline
        timeline={timeline}
        selection={state.selection}
        frame={frame}
        onSeek={seek}
        onSelect={select}
        dispatch={dispatch}
      />
    </div>
  );
};
