import { useEffect, useMemo, useReducer, useRef, useState } from "react";
import type { PlayerRef } from "@remotion/player";
import sample from "../../../fixtures/sample-timeline.json";
import type { VideoTimeline } from "../../src/types/timeline";
import { checkTimeline, emptyScene, frameCount, frameToMs, timecode, totalMs, ALLOWED_RESOLUTIONS, type Selection } from "./model";
import { reducer, type EditorState } from "./state";
import { baseName, confirmDiscard, guardClose, parseTimeline, pickToOpen, saveTimeline, serialize, setWindowTitle } from "./project";
import { Toolbar } from "./components/Toolbar";
import { Preview } from "./components/Preview";
import { Inspector, type InspectorTab } from "./components/Inspector";
import { EditorTimeline } from "./components/EditorTimeline";
import { RenderPanel } from "./components/RenderPanel";
import {
  applyProgress,
  cancelRender,
  listenRender,
  pickRenderOutput,
  renderSupported,
  revealInFinder,
  startRender,
  type RenderView,
} from "./render";
import { inTauri } from "./project";
import { assetUrlFor, loadAssetRoot } from "./media";
import { resolveAsset } from "../../src/assets";
import { beatMarks, detectBeats, type DetectedBeats } from "./beats";
import { loadMusicLibrary, musicForBeats, pickKey, pickMusic, resolvedMusic, type MusicLibrary, type PickedTrack } from "./music";
import type { BeatStatus } from "./components/SoundPanel";

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
  const [tab, setTab] = useState<InspectorTab>("scene");
  const [path, setPath] = useState<string | null>(null);
  // The last saved (or opened) document. Unsaved changes = the current document differs from it.
  // A recovered draft starts as "different", so it reads as unsaved until the user saves it.
  const [saved, setSaved] = useState<string>(draft ? "" : serialize(startTimeline));
  const [error, setError] = useState<string | null>(null);
  const [render, setRender] = useState<RenderView>({ status: "idle" });
  const [assetRoot, setAssetRoot] = useState<string | null>(null);
  const [beats, setBeats] = useState<DetectedBeats | null>(null);
  const [beatStatus, setBeatStatus] = useState<BeatStatus>({ busy: false, error: null });
  const playerRef = useRef<PlayerRef>(null);
  const { timeline } = state;
  // Music by mood: the library (loaded once) and the track the pick gets, asked again whenever
  // what the choice depends on changes.
  const [library, setLibrary] = useState<MusicLibrary | { error: string } | null>(null);
  const [picked, setPicked] = useState<{ key: string; result: PickedTrack | { error: string } } | null>(null);
  const currentPickKey = pickKey(timeline);
  useEffect(() => {
    if (!currentPickKey || library) return;
    void loadMusicLibrary().then(setLibrary);
  }, [currentPickKey, library]);
  useEffect(() => {
    if (!currentPickKey || picked?.key === currentPickKey) return;
    const key = currentPickKey;
    const handle = window.setTimeout(() => {
      void pickMusic(timeline).then((result) => setPicked({ key, result }));
    }, 150);
    return () => window.clearTimeout(handle);
  }, [currentPickKey, picked, timeline]);
  const pickedTrack = picked && picked.key === currentPickKey && !("error" in picked.result) ? picked.result : null;
  const pickError = picked && picked.key === currentPickKey && "error" in picked.result ? picked.result.error : null;
  // The preview plays what the render will play: a pick becomes the chosen file.
  const previewTimeline = useMemo(
    () => (timeline.music?.pick ? { ...timeline, music: resolvedMusic(timeline.music, pickedTrack) } : timeline),
    [timeline, pickedTrack],
  );
  const marks = useMemo(
    () => beatMarks(musicForBeats(timeline.music, pickedTrack), beats, totalMs(timeline)),
    [timeline, beats, pickedTrack],
  );
  const onDetectBeats = () => {
    const source = previewTimeline.music?.source;
    if (!source) return;
    setBeatStatus({ busy: true, error: null });
    void detectBeats(source).then((r) => {
      if ("error" in r) setBeatStatus({ busy: false, error: r.error });
      else {
        setBeats(r);
        setBeatStatus({ busy: false, error: null });
      }
    });
  };
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

  useEffect(() => {
    if (!inTauri()) return;
    loadAssetRoot().then(setAssetRoot).catch(() => setAssetRoot(null));
  }, []);

  // Render progress and completion come from the desktop shell as events.
  useEffect(() => {
    if (!inTauri()) return;
    let unlisten: (() => void) | undefined;
    void listenRender(
      (p) => setRender((v) => applyProgress(v, p)),
      (f) => {
        if (f.ok && f.output) setRender({ status: "done", output: f.output });
        else if (f.cancelled) setRender({ status: "cancelled" });
        else setRender({ status: "failed", error: f.error ?? "Unknown error" });
      },
    ).then((u) => (unlisten = u));
    return () => unlisten?.();
  }, []);

  // Keep the playhead inside the timeline when edits shorten it.
  useEffect(() => {
    const max = frameCount(timeline) - 1;
    if (frame > max) playerRef.current?.seekTo(max);
  }, [timeline, frame]);

  const selectedLayer =
    state.selection.kind === "layer" && tab === "scene"
      ? timeline.scenes[state.selection.sceneIndex]?.layers?.[state.selection.index] ?? null
      : null;

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
    // In the assistant tab, picking a scene only sets the assistant's context.
    if (tab === "assistant") return;
    if (selection.kind === "subtitle") setTab("subtitles");
    if (selection.kind === "scene" || selection.kind === "keyframe" || selection.kind === "layer") setTab("scene");
    if (selection.kind === "music") setTab("music");
  };

  const seek = (f: number) => {
    const clamped = Math.max(0, Math.min(f, frameCount(timeline) - 1));
    playerRef.current?.seekTo(clamped);
    setFrame(clamped);
  };

  const doRender = async () => {
    if (!renderSupported()) {
      setError("Rendering needs the desktop app.");
      return;
    }
    if (problems.length > 0) {
      setError(`Fix ${problems.length} issue${problems.length > 1 ? "s" : ""} before rendering.`);
      return;
    }
    const output = await pickRenderOutput(`${baseName(path).replace(/\.json$/i, "") || "video"}.mp4`);
    if (!output) return;
    setError(null);
    setRender({ status: "running", stage: "Starting" });
    try {
      await startRender(text, output);
    } catch (e) {
      setRender({ status: "failed", error: String(e) });
    }
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
        onRender={() => void doRender()}
        rendering={render.status === "running"}
      />
      <RenderPanel
        view={render}
        onCancel={() => void cancelRender()}
        onReveal={(p) => void revealInFinder(p).catch((e) => setError(String(e)))}
        onDismiss={() => setRender({ status: "idle" })}
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
          <Preview
            timeline={previewTimeline}
            assetRoot={assetRoot}
            playerRef={playerRef}
            onFrame={setFrame}
            selectedLayer={selectedLayer}
            onMoveLayer={(x, y) => {
              if (state.selection.kind !== "layer") return;
              const { sceneIndex, index } = state.selection;
              dispatch({ type: "patchLayer", sceneIndex, index, patch: { x, y } });
            }}
          />
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
          beats={beats}
          beatStatus={beatStatus}
          onDetectBeats={onDetectBeats}
          musicLibrary={library}
          onLoadMusicLibrary={() => void loadMusicLibrary().then(setLibrary)}
          pickedTrack={pickedTrack}
          pickError={pickError}
          audioUrl={(path) => resolveAsset(path, assetRoot ? assetUrlFor(assetRoot) : null)}
        />
      </main>
      <EditorTimeline
        timeline={timeline}
        selection={state.selection}
        frame={frame}
        onSeek={seek}
        onSelect={select}
        dispatch={dispatch}
        beatMarks={marks}
      />
    </div>
  );
};
