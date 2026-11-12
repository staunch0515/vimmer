import { useEffect, useMemo, useReducer, useRef, useState } from "react";
import type { PlayerRef } from "@remotion/player";
import sample from "../../../fixtures/sample-timeline.json";
import type { VideoTimeline } from "../../src/types/timeline";
import { checkTimeline, emptyScene, frameCount, frameToMs, timecode, totalMs, ALLOWED_RESOLUTIONS, type Selection } from "./model";
import { reducer, type EditorState } from "./state";
import { baseName, confirmDiscard, guardClose, parseTimeline, pickToOpen, saveTimeline, serialize, setWindowTitle } from "./project";
import { Toolbar } from "./components/Toolbar";
import { Preview } from "./components/Preview";
import { Inspector } from "./components/Inspector";
import { EditorTimeline } from "./components/EditorTimeline";

// Unsaved work is mirrored here so a crash or reload does not lose it. Cleared on save.
const DRAFT_KEY = "vimmer-studio-draft";

const readDraft = (): VideoTimeline | null => {
  try {
    const raw = window.localStorage.getItem(DRAFT_KEY);
    return raw ? parseTimeline(raw) : null;
  } catch {
    return null;
  }
};

const newTimeline = (): VideoTimeline => ({
  fps: 30,
  resolution: { width: 1920, height: 1080 },
  scenes: [emptyScene(0)],
  subtitles: [],
});

const draft = readDraft();
const startTimeline: VideoTimeline = draft ?? parseTimeline(JSON.stringify(sample));

const initial: EditorState = {
  timeline: startTimeline,
  selection: { kind: "none" },
  history: [],
  future: [],
};

export const App = () => {
  const [state, dispatch] = useReducer(reducer, initial);
  const [frame, setFrame] = useState(0);
  const [tab, setTab] = useState<"scene" | "subtitles">("scene");
  const [path, setPath] = useState<string | null>(null);
  // The last saved (or opened) document. Unsaved changes = the current document differs from it.
  // A recovered draft starts as "different", so it reads as unsaved until the user saves it.
  const [saved, setSaved] = useState<string>(draft ? "" : serialize(startTimeline));
  const [error, setError] = useState<string | null>(null);
  const playerRef = useRef<PlayerRef>(null);
  const { timeline } = state;
  const problems = useMemo(() => checkTimeline(timeline), [timeline]);
  const fps = timeline.fps;
  const text = serialize(timeline);
  const dirty = text !== saved;
  const dirtyRef = useRef(dirty);
  dirtyRef.current = dirty;

  useEffect(() => {
    try {
      if (dirty) window.localStorage.setItem(DRAFT_KEY, text);
      else window.localStorage.removeItem(DRAFT_KEY);
    } catch {
      /* storage unavailable: the document is still saved to disk when the user saves */
    }
  }, [text, dirty]);

  useEffect(() => {
    setWindowTitle(`${baseName(path)}${dirty ? " •" : ""} — Vimmer Studio`);
  }, [path, dirty]);

  useEffect(() => {
    let off: (() => void) | undefined;
    void guardClose(() => dirtyRef.current, "You have unsaved changes. Close anyway?").then((f) => (off = f));
    return () => off?.();
  }, []);

  // Keep the playhead inside the timeline when edits shorten it.
  useEffect(() => {
    const max = frameCount(timeline) - 1;
    if (frame > max) playerRef.current?.seekTo(max);
  }, [timeline, frame]);

  const discardOk = async () => !dirtyRef.current || (await confirmDiscard("Discard unsaved changes?"));

  const doNew = async () => {
    if (!(await discardOk())) return;
    const t = newTimeline();
    dispatch({ type: "load", timeline: t });
    setPath(null);
    setSaved(serialize(t));
    setError(null);
  };

  const doOpen = async () => {
    if (!(await discardOk())) return;
    try {
      const picked = await pickToOpen();
      if (!picked) return;
      const t = parseTimeline(picked.text);
      dispatch({ type: "load", timeline: t });
      setPath(picked.path);
      setSaved(serialize(t));
      setError(null);
    } catch (e) {
      setError(`Could not open the file: ${(e as Error).message}`);
    }
  };

  const doSave = async (saveAs: boolean) => {
    try {
      const result = await saveTimeline(text, { path }, { saveAs });
      if (!result) return;
      if (result.path) setPath(result.path);
      setSaved(text);
      setError(null);
    } catch (e) {
      setError(`Could not save: ${(e as Error).message}`);
    }
  };

  // Shortcuts call the latest handlers through a ref, so the listener never sees stale state.
  const actions = useRef({ doNew, doOpen, doSave });
  actions.current = { doNew, doOpen, doSave };

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      const target = e.target as HTMLElement;
      const typing = target.matches("input, textarea, select, [contenteditable]");
      const mod = e.metaKey || e.ctrlKey;
      const key = e.key.toLowerCase();
      if (mod && key === "s") {
        e.preventDefault();
        void actions.current.doSave(e.shiftKey);
      } else if (mod && key === "o") {
        e.preventDefault();
        void actions.current.doOpen();
      } else if (mod && key === "n") {
        e.preventDefault();
        void actions.current.doNew();
      } else if (mod && key === "z" && !typing) {
        e.preventDefault();
        dispatch({ type: e.shiftKey ? "redo" : "undo" });
      } else if (mod && key === "y" && !typing) {
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
        fileName={baseName(path)}
        dirty={dirty}
        problemCount={problems.length}
        format={timeline.resolution}
        formats={[...ALLOWED_RESOLUTIONS]}
        canUndo={state.history.length > 0}
        canRedo={state.future.length > 0}
        onNew={() => void doNew()}
        onOpen={() => void doOpen()}
        onSave={() => void doSave(false)}
        onSaveAs={() => void doSave(true)}
        onUndo={() => dispatch({ type: "undo" })}
        onRedo={() => dispatch({ type: "redo" })}
        onResolution={(w, h) => dispatch({ type: "setResolution", width: w, height: h })}
      />
      {error && (
        <div className="problems error" role="alert">
          <strong>{error}</strong>
          <button className="btn ghost small" onClick={() => setError(null)}>Dismiss</button>
        </div>
      )}
      {problems.length > 0 && (
        <div className="problems" role="status">
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
            <span className="dim">Space play · ⌘S save · ⌘Z undo</span>
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
