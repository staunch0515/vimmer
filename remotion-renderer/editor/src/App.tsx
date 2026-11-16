import { useCallback, useEffect, useMemo, useReducer, useRef, useState } from "react";
import type { PlayerRef } from "@remotion/player";
import sample from "../../../fixtures/sample-timeline.json";
import type { VideoTimeline } from "../../src/types/timeline";
import { checkTimeline, emptyScene, frameCount, frameToMs, timecode, totalMs, ALLOWED_RESOLUTIONS, type Selection } from "./model";
import { reducer, type Action, type EditorState } from "./state";
import { mergeKey, mergesInput } from "./history";
import { boxOf, nudgeBox } from "./subtitleBox";
import { baseName, confirmDiscard, guardClose, parseTimeline, pickToOpen, saveTimeline, serialize, setWindowTitle } from "./project";
import { Toolbar } from "./components/Toolbar";
import { Preview } from "./components/Preview";
import { Inspector, type InspectorTab } from "./components/Inspector";
import { EditorTimeline } from "./components/EditorTimeline";
import { RenderPanel } from "./components/RenderPanel";
import { StudioLayout } from "./components/layout/StudioLayout";
import {
  applyProgress,
  cancelRender,
  checkBeforeRender,
  listenRender,
  loadRenderDefaults,
  pickRenderOutput,
  renderSupported,
  revealInFinder,
  startRender,
  type RenderView,
} from "./render";
import {
  FALLBACK_DEFAULTS,
  checkKey,
  findingSelection,
  hasOptions,
  loadOptions,
  saveOptions,
  type CheckFinding,
  type CheckView,
  type RenderDefaults,
  type RenderOptions,
} from "./renderOptions";
import { inTauri } from "./project";
import { assetUrlFor, loadAssetRoot } from "./media";
import { resolveAsset } from "../../src/assets";
import { beatMarks, detectBeats, type DetectedBeats } from "./beats";
import { resolveAutoGrades } from "../../src/motion/look";
import { loadMusicLibrary, musicForBeats, pickKey, pickMusic, resolvedMusic, type MusicLibrary, type PickedTrack } from "./music";
import type { BeatStatus } from "./components/SoundPanel";
import { heardPages } from "../../src/textPages";
import { listenKey, rememberClips, toPreviewMs, toSavedMs, withNarration, type HeardClips, type ListenClip, type TtsChoice } from "./narration";
import { listenNarration } from "./listen";
import { NarrationBar } from "./components/NarrationBar";
import { useAudioLengths } from "./audioLengths";
import * as vo from "./voiceover";
import type { VoicePanelProps } from "./components/VoiceoverPanel";
import { SceneRecorder } from "./components/SceneRecorder";
import { Teleprompter } from "./components/Teleprompter";
import { AlignScenes } from "./components/AlignScenes";
import "./studio/studio.css";
import { ASSET_DRAG_TYPE } from "./studio/AssetPanel";
import { errorText, tauriApi } from "./studio/api";
import { Home } from "./studio/Home";
import { LeaveDialog } from "./studio/LeaveDialog";
import { checkProjectBeforeRender, startProjectRender, type ProjectRenderFinished } from "./studio/projectRender";
import { PlainFileSidebar, ProjectSidebar } from "./studio/ProjectSidebar";
import { clearOpenProject, draftProject, rememberDraftProject, setOpenProject, setProjectFiles } from "./studio/session";
import type { AssetInfo, OpenedProject, ProjectSummary } from "./studio/types";

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

const storage = (): Storage | null => {
  try {
    return window.localStorage;
  } catch {
    return null;
  }
};

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
  // Bumped whenever another document is loaded (new, open, project): the assistant's chat is per document.
  const [docKey, setDocKey] = useState(0);
  // Studio projects (desktop app). The app starts on the projects screen; the browser editor and
  // a recovered draft go straight to the editor, as before. `project` is the open project, or
  // null for a plain timeline file (`path`) outside any project.
  const [screen, setScreen] = useState<"home" | "editor">(inTauri() && !draft ? "home" : "editor");
  const [project, setProject] = useState<ProjectSummary | null>(null);
  const [assetsKey, setAssetsKey] = useState(0);
  const [rendersKey, setRendersKey] = useState(0);
  // Bumped when the open project's file list changes, so previews resolve the new files.
  const [, setFilesVersion] = useState(0);
  // Asked before a document with unsaved changes is replaced by another one.
  const [leave, setLeave] = useState<{ action: string; then: () => void } | null>(null);
  const [leaveSaving, setLeaveSaving] = useState(false);
  const projectRef = useRef(project);
  projectRef.current = project;
  const screenRef = useRef(screen);
  screenRef.current = screen;
  // The last saved (or opened) document. Unsaved changes = the current document differs from it.
  // A recovered draft starts as "different", so it reads as unsaved until the user saves it.
  const [saved, setSaved] = useState<string>(draft ? "" : serialize(startTimeline));
  const [error, setError] = useState<string | null>(null);
  const [render, setRender] = useState<RenderView>({ status: "idle" });
  const [renderOptions, setRenderOptions] = useState<RenderOptions>(() => loadOptions(storage(), null));
  const [renderDefaults, setRenderDefaults] = useState<RenderDefaults>(FALLBACK_DEFAULTS);
  const [check, setCheck] = useState<CheckView>({ status: "idle" });
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
    // `auto` grades follow the pick's mood, so they are settled before the pick becomes a file.
    () => (timeline.music?.pick ? { ...resolveAutoGrades(timeline), music: resolvedMusic(timeline.music, pickedTrack) } : timeline),
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

  const docName = project ? project.name : baseName(path);
  useEffect(() => {
    setWindowTitle(screen === "home" ? "Vimmer Studio" : `${docName}${dirty ? " •" : ""} — Vimmer Studio`);
  }, [docName, dirty, screen]);

  // An unsaved project document is a draft like any other; remember whose it is.
  useEffect(() => {
    rememberDraftProject(dirty && project ? project.dir : null);
  }, [dirty, project]);
  // A draft recovered at launch that belonged to a project goes back into that project, unsaved.
  useEffect(() => {
    const dir = draft && inTauri() ? draftProject() : null;
    if (!dir) return;
    void (async () => {
      try {
        const opened = await tauriApi.openProject(dir);
        const assets = await tauriApi.listAssets(dir, opened.timelineJson).catch(() => []);
        setOpenProject(dir, assets.map((a) => a.path), () => setAssetsKey((k) => k + 1));
        setProject(opened.project);
        setSaved(serialize(parseTimeline(opened.timelineJson)));
      } catch {
        /* the project is gone: the draft stays an untitled file */
      }
    })();
  }, []);

  useEffect(() => {
    let off: (() => void) | undefined;
    void guardClose(() => dirtyRef.current, "You have unsaved changes. Close anyway?").then((f) => (off = f));
    return () => off?.();
  }, []);

  // Render options are remembered per project file. A file without remembered options (a new
  // "Save as" target, a project opened for the first time) keeps the current choices.
  const renderOptionsRef = useRef(renderOptions);
  renderOptionsRef.current = renderOptions;
  const optionsKey = project ? project.dir : path;
  useEffect(() => {
    const store = storage();
    if (hasOptions(store, optionsKey)) setRenderOptions(loadOptions(store, optionsKey));
    else saveOptions(store, optionsKey, renderOptionsRef.current);
  }, [optionsKey]);
  const changeRenderOptions = (o: RenderOptions) => {
    setRenderOptions(o);
    saveOptions(storage(), optionsKey, o);
  };
  useEffect(() => void loadRenderDefaults().then(setRenderDefaults), []);

  // While the render panel is open, the check follows the timeline and the chosen engine.
  const renderEngine = renderOptions.tts ?? renderDefaults.tts;
  const wantedCheck = render.status === "setup" ? checkKey(text, renderEngine) : null;
  const [checkRun, setCheckRun] = useState(0);
  useEffect(() => {
    if (!wantedCheck || !inTauri()) return;
    const key = wantedCheck;
    const handle = window.setTimeout(() => {
      setCheck({ status: "running", key });
      // A project's own files are found first, as in its render.
      (project ? checkProjectBeforeRender(project.dir, text, renderEngine) : checkBeforeRender(text, renderEngine)).then(
        (report) => setCheck((c) => (c.status === "running" && c.key === key ? { status: "done", key, report } : c)),
        (e) => setCheck((c) => (c.status === "running" && c.key === key ? { status: "failed", key, error: String(e) } : c)),
      );
    }, 250);
    return () => window.clearTimeout(handle);
    // `text` and `renderEngine` are part of `wantedCheck`.
  }, [wantedCheck, checkRun, project?.dir]);
  const currentCheck: CheckView =
    check.status !== "idle" && check.key === wantedCheck ? check : { status: "running", key: wantedCheck ?? "" };

  // 试听 Listen: narration clips heard so far (made by the render's code into its cache), and
  // whether the preview plays them ("listen to all"). The preview then shows each heard scene as
  // long as the render will make it; the saved timeline is not changed. The playhead and the
  // timeline below stay in saved time: Player frames are mapped through `narrated`.
  const [heard, setHeard] = useState<HeardClips>({});
  const [narrationOn, setNarrationOn] = useState(false);
  const [listenAll, setListenAll] = useState<{ busy: boolean; error: string | null; warnings: string[] }>({ busy: false, error: null, warnings: [] });
  const onHeard = useCallback((engine: TtsChoice, clips: ListenClip[]) => setHeard((h) => rememberClips(h, engine, clips)), []);
  const narrated = useMemo(
    () =>
      narrationOn
        ? withNarration(previewTimeline, heard, renderEngine, (scene, speechMs) =>
            heardPages(scene, previewTimeline.brand, previewTimeline.resolution, speechMs),
          )
        : null,
    [narrationOn, previewTimeline, heard, renderEngine],
  );
  const narratable = timeline.scenes.some((s) => listenKey(s, renderEngine) !== null);
  const narratedRef = useRef(narrated);
  narratedRef.current = narrated;
  const fpsRef = useRef(timeline.fps);
  fpsRef.current = timeline.fps;
  /** Saved-timeline frame → Player frame. */
  const toPlayerFrame = (f: number): number => {
    const n = narratedRef.current;
    const r = fpsRef.current;
    return n ? Math.round((toPreviewMs(n, frameToMs(f, r)) * r) / 1000) : f;
  };
  const onPlayerFrame = useCallback((f: number) => {
    const n = narratedRef.current;
    const r = fpsRef.current;
    setFrame(n ? Math.floor((toSavedMs(n, frameToMs(f, r)) * r) / 1000) : f);
  }, []);
  useEffect(() => setNarrationOn(false), [docKey]);
  const doListenAll = async () => {
    setListenAll({ busy: true, error: null, warnings: [] });
    const engine = renderEngine;
    const result = await listenNarration(text, [], engine);
    if (!result.ok) {
      setListenAll({ busy: false, error: result.error, warnings: [] });
      return;
    }
    setHeard((h) => rememberClips(h, engine, result.clips));
    setListenAll({ busy: false, error: null, warnings: result.warnings });
    setNarrationOn(true);
    setFrame(0);
    // Once the Player has the narrated timeline: from the start, with sound.
    window.setTimeout(() => {
      playerRef.current?.seekTo(0);
      playerRef.current?.play();
    }, 50);
  };

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
        // A project render is in the project's history now: show it in the sidebar.
        const done = (f as ProjectRenderFinished).project;
        if (done && done.dir === projectRef.current?.dir) {
          setProject(done);
          setRendersKey((k) => k + 1);
        }
      },
    ).then((u) => (unlisten = u));
    return () => unlisten?.();
  }, []);

  // Keep the playhead inside the timeline when edits shorten it.
  useEffect(() => {
    const max = frameCount(timeline) - 1;
    if (frame > max) playerRef.current?.seekTo(toPlayerFrame(max));
  }, [timeline, frame]);

  const selectedLayer =
    state.selection.kind === "layer" && tab === "scene"
      ? timeline.scenes[state.selection.sceneIndex]?.layers?.[state.selection.index] ?? null
      : null;

  const discardOk = async () => !dirtyRef.current || (await confirmDiscard("Discard unsaved changes?"));

  /** The editor now holds a plain file (or nothing saved yet), not a project. */
  const leaveProject = () => {
    clearOpenProject();
    setProject(null);
  };

  const doNew = async () => {
    if (!(await discardOk())) return;
    const t = newTimeline();
    dispatch({ type: "load", timeline: t });
    setDocKey((k) => k + 1);
    leaveProject();
    setPath(null);
    setSaved(serialize(t));
    setError(null);
  };

  /** Opens a plain timeline file. Resolves to whether one was opened. */
  const doOpen = async (): Promise<boolean> => {
    if (!(await discardOk())) return false;
    try {
      const picked = await pickToOpen();
      if (!picked) return false;
      const t = parseTimeline(picked.text);
      dispatch({ type: "load", timeline: t });
      setDocKey((k) => k + 1);
      leaveProject();
      setPath(picked.path);
      setSaved(serialize(t));
      setError(null);
      return true;
    } catch (e) {
      setError(`Could not open the file: ${(e as Error).message}`);
      return false;
    }
  };

  /** Saves the document. Resolves to whether it is saved now. */
  const doSave = async (saveAs: boolean): Promise<boolean> => {
    try {
      if (project && !saveAs) {
        setProject(await tauriApi.saveProject(project.dir, text));
        setSaved(text);
        setError(null);
        return true;
      }
      // In a project, "Save as" writes a copy of the timeline elsewhere; the project stays open.
      const result = await saveTimeline(text, { path: project ? null : path }, { saveAs });
      if (!result) return false;
      if (project) return true;
      if (result.path) setPath(result.path);
      setSaved(text);
      setError(null);
      return true;
    } catch (e) {
      setError(`Could not save: ${errorText(e)}`);
      return false;
    }
  };

  /** Shows a project in the editor. Throws when its timeline cannot be read. */
  const showProject = async (opened: OpenedProject) => {
    const t = parseTimeline(opened.timelineJson);
    const dir = opened.project.dir;
    // Known before the timeline loads, so the preview finds the project's files at once.
    const assets = await tauriApi.listAssets(dir, opened.timelineJson).catch(() => [] as AssetInfo[]);
    setOpenProject(dir, assets.map((a) => a.path), () => setAssetsKey((k) => k + 1));
    dispatch({ type: "load", timeline: t });
    setDocKey((k) => k + 1);
    setProject(opened.project);
    setPath(null);
    setSaved(serialize(t));
    setError(null);
    setRender((r) => (r.status === "running" ? r : { status: "idle" }));
    setScreen("editor");
  };

  /** From Home: opens the project, asking first when the editor holds unsaved changes. */
  const openProject = async (opened: OpenedProject) => {
    if (opened.project.dir === project?.dir) {
      setScreen("editor");
      return;
    }
    if (!dirtyRef.current) return showProject(opened);
    setLeave({ action: `Open “${opened.project.name}”`, then: () => void showProject(opened).catch((e) => setError(errorText(e))) });
  };

  const onProjectAssets = (dir: string, assets: AssetInfo[]) => {
    setProjectFiles(dir, assets.map((a) => a.path));
    setFilesVersion((v) => v + 1);
  };

  /** The scene "use this asset" applies to: the selected one, else the one under the playhead. */
  const currentSceneIndex = (): number => {
    const sel = state.selection;
    if (sel.kind === "scene") return sel.index;
    if (sel.kind === "keyframe" || sel.kind === "layer") return sel.sceneIndex;
    const at = frameToMs(frame, fps);
    let end = 0;
    for (let i = 0; i < timeline.scenes.length; i += 1) {
      end += timeline.scenes[i].duration_ms;
      if (at < end) return i;
    }
    return Math.max(0, timeline.scenes.length - 1);
  };

  /** Puts a project file into the current scene: as its image, clip or audio, or as an icon layer. */
  const applyAsset = (asset: Pick<AssetInfo, "path" | "kind">) => {
    const index = currentSceneIndex();
    const scene = timeline.scenes[index];
    if (!scene) return;
    if (asset.kind === "image") dispatch({ type: "patchScene", index, patch: { image: asset.path } });
    else if (asset.kind === "video") dispatch({ type: "patchScene", index, patch: { video: { ...scene.video, src: asset.path } } });
    else if (asset.kind === "audio") dispatch({ type: "patchScene", index, patch: { audio: { offset_ms: scene.audio?.offset_ms ?? 0, source: asset.path } } });
    else if (asset.kind === "icon") {
      dispatch({ type: "addLayer", sceneIndex: index, kind: "icon" });
      dispatch({ type: "patchLayer", sceneIndex: index, index: scene.layers?.length ?? 0, patch: { src: asset.path } });
    } else return;
    if (asset.kind !== "icon") select({ kind: "scene", index });
  };

  /** A plain file becomes a project: the saved timeline and the files it names are copied. */
  const makeProject = async () => {
    if (!path) return;
    try {
      const got = await tauriApi.importTimeline(path);
      await showProject(await tauriApi.openProject(got.project.dir));
      if (got.missing.length) setError(`Not found, so not copied into the project: ${got.missing.join(", ")}`);
    } catch (e) {
      setError(`Could not make a project: ${errorText(e)}`);
    }
  };

  // Arrow keys nudge the subtitle box while a subtitle is selected: 1 px, 10 px with Shift.
  // Presses in a row are one undo step (like typing in a field).
  const nudgeSubtitles = (key: string, shift: boolean): boolean => {
    if (state.selection.kind !== "subtitle") return false;
    const step = shift ? 10 : 1;
    const d = { ArrowLeft: [-step, 0], ArrowRight: [step, 0], ArrowUp: [0, -step], ArrowDown: [0, step] }[key];
    if (!d) return false;
    const box = nudgeBox(boxOf(timeline.subtitle_box, timeline.resolution), d[0], d[1], timeline.resolution);
    dispatch({ type: "coalesce", key: "nudge-subtitles", at: performance.now(), action: { type: "setSubtitleBox", box } });
    return true;
  };

  // Shortcuts call the latest handlers through a ref, so the listener never sees stale state.
  const actions = useRef({ doNew, doOpen, doSave, nudgeSubtitles });
  actions.current = { doNew, doOpen, doSave, nudgeSubtitles };

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      // The editor's shortcuts are off while the projects screen covers it.
      if (screenRef.current !== "editor") return;
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
      } else if (!typing && !mod && e.key.startsWith("Arrow") && actions.current.nudgeSubtitles(e.key, e.shiftKey)) {
        e.preventDefault();
      } else if (e.code === "Space" && !typing) {
        e.preventDefault();
        playerRef.current?.toggle();
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, []);

  // Edits made while typing in an Inspector field (or dragging one of its sliders) merge into one
  // undo step per field, until it loses focus or a second passes without input (history.ts).
  // A native capture listener sees the input event before React's onChange runs the edit.
  const fieldsRef = useRef<HTMLDivElement>(null);
  const typingIn = useRef<HTMLElement | null>(null);
  const fieldIds = useRef({ next: 1, ids: new WeakMap<Element, number>() });
  useEffect(() => {
    const root = fieldsRef.current;
    if (!root) return;
    const onInput = (e: Event) => {
      const el = e.target as HTMLElement;
      if (!mergesInput(el.tagName, el.getAttribute("type"), el.isContentEditable)) return;
      typingIn.current = el;
      queueMicrotask(() => {
        if (typingIn.current === el) typingIn.current = null;
      });
    };
    const onBlur = () => dispatch({ type: "endMerge" });
    root.addEventListener("input", onInput, true);
    root.addEventListener("focusout", onBlur, true);
    return () => {
      root.removeEventListener("input", onInput, true);
      root.removeEventListener("focusout", onBlur, true);
    };
  }, []);
  const fieldDispatch = (action: Action) => {
    const el = typingIn.current;
    if (!el || action.type === "select" || action.type === "undo" || action.type === "redo" || action.type === "load") {
      dispatch(action);
      return;
    }
    const ids = fieldIds.current;
    const id = ids.ids.get(el) ?? ids.next++;
    ids.ids.set(el, id);
    dispatch({ type: "coalesce", key: mergeKey(id, action), at: performance.now(), action });
  };

  const select = (selection: Selection) => {
    dispatch({ type: "select", selection });
    // In the assistant tab, picking a scene only sets the assistant's context.
    if (tab === "assistant") return;
    if (selection.kind === "subtitle") setTab("subtitles");
    if (selection.kind === "scene" || selection.kind === "keyframe" || selection.kind === "layer") setTab("scene");
    if (selection.kind === "music") setTab("music");
    if (selection.kind === "voiceover") setTab("voice");
  };

  const seek = (f: number) => {
    const clamped = Math.max(0, Math.min(f, frameCount(timeline) - 1));
    playerRef.current?.seekTo(toPlayerFrame(clamped));
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
    setError(null);
    setRender({ status: "setup" });
  };

  // From the render panel, after the options and the check.
  const doStartRender = async () => {
    // A project renders into its own renders/ folder; a plain file asks where to save.
    const output = project ? null : await pickRenderOutput(`${baseName(path).replace(/\.json$/i, "") || "video"}.mp4`);
    if (!project && !output) return;
    setRender({ status: "running", stage: "Starting" });
    try {
      if (project) await startProjectRender(project.dir, text, renderOptions, renderDefaults);
      else await startRender(text, output!, renderOptions, renderDefaults);
    } catch (e) {
      setRender({ status: "failed", error: String(e) });
    }
  };

  const onFinding = (f: CheckFinding) => {
    const selection = findingSelection(f, timeline);
    if (selection) select(selection);
    if (f.time_ms !== undefined) seek(Math.round((f.time_ms / 1000) * fps));
  };

  // The voice-over track: file lengths (measured from the files) and the 旁白 tab's commands.
  const audioAssetUrl = useMemo(() => (assetRoot ? assetUrlFor(assetRoot) : null), [assetRoot]);
  const [audioVersion] = useState(0);
  const [prompter, setPrompter] = useState(false);
  const audioLengths = useAudioLengths(timeline, audioAssetUrl, audioVersion);
  const addVoiceoverClip = (source: string, atMs: number): string | null => {
    const made = vo.addClip(timeline, source, atMs, audioLengths);
    if ("error" in made) return made.error;
    dispatch({ type: "commitTimeline", timeline: made.timeline, selection: { kind: "voiceover", index: made.index } });
    setTab("voice");
    return null;
  };
  const voice: VoicePanelProps = {
    lengths: audioLengths,
    onRecordAll: () => setPrompter(true),
    alignControls: (
      <AlignScenes
        timeline={timeline}
        dispatch={dispatch}
        clipId={state.selection.kind === "voiceover" ? (timeline.voiceover?.[state.selection.index]?.id ?? null) : null}
      />
    ),
    sceneRecorder: (index) => {
      const scene = timeline.scenes[index];
      // Keyed by scene: switching scenes unmounts it, which releases the microphone.
      return scene ? <SceneRecorder key={`${index}:${scene.id}`} scene={scene} index={index} dispatch={dispatch} /> : null;
    },
    onAddClip: (source) => addVoiceoverClip(source, frameToMs(frame, fps)),
    onMoveSceneAudio: (index) => {
      const moved = vo.moveSceneAudioToTrack(timeline, index, audioLengths);
      if ("error" in moved) return moved.error;
      dispatch({ type: "commitTimeline", timeline: moved.timeline, selection: { kind: "voiceover", index: moved.index } });
      setTab("voice");
      return null;
    },
  };

  const addSubtitleAtPlayhead = () => {
    dispatch({ type: "addSubtitle", startMs: frameToMs(frame, fps) });
    setTab("subtitles");
  };

  const onAssetDrag = (e: React.DragEvent) => {
    if (!e.dataTransfer.types.includes(ASSET_DRAG_TYPE)) return;
    e.preventDefault();
    e.dataTransfer.dropEffect = "copy";
  };
  const onAssetDrop = (e: React.DragEvent) => {
    const raw = e.dataTransfer.getData(ASSET_DRAG_TYPE);
    if (!raw) return;
    e.preventDefault();
    try {
      applyAsset(JSON.parse(raw) as Pick<AssetInfo, "path" | "kind">);
    } catch {
      /* not one of ours */
    }
  };

  return (
    <>
    <div className="studio" inert={screen === "home"}>
      <Toolbar
        onHome={inTauri() ? () => setScreen("home") : undefined}
        fileName={docName}
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
        options={renderOptions}
        defaults={renderDefaults}
        onOptions={changeRenderOptions}
        check={currentCheck}
        onRecheck={() => {
          setCheck({ status: "idle" });
          setCheckRun((n) => n + 1);
        }}
        onFinding={onFinding}
        onRender={() => void doStartRender()}
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
      <StudioLayout
        left={
          !inTauri() ? undefined : project ? (
            <ProjectSidebar
              project={project}
              timelineJson={text}
              sceneLabel={timeline.scenes[currentSceneIndex()]?.id ?? null}
              onUseAsset={applyAsset}
              assetsKey={assetsKey}
              rendersKey={rendersKey}
              onProjectChanged={setProject}
              onAssets={onProjectAssets}
              onHome={() => setScreen("home")}
            />
          ) : (
            <PlainFileSidebar fileName={docName} onHome={() => setScreen("home")} onMakeProject={path && !dirty ? () => void makeProject() : undefined} />
          )
        }
        center={
        <section className="stage" onDragOver={onAssetDrag} onDrop={onAssetDrop}>
          <Preview
            timeline={narrated?.timeline ?? previewTimeline}
            assetRoot={assetRoot}
            playerRef={playerRef}
            onFrame={onPlayerFrame}
            audioLengths={audioLengths}
            selectedLayer={selectedLayer}
            subtitlesSelected={state.selection.kind === "subtitle"}
            onMoveSubtitles={(box) => dispatch({ type: "setSubtitleBox", box })}
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
            <span className="dim">Space play · ⌘S save · ⌘Z undo · ⇧⌘Z redo</span>
          </div>
          <NarrationBar
            narrated={narrated}
            savedMs={totalMs(timeline)}
            narratable={narratable}
            status={listenAll}
            onListenAll={() => void doListenAll()}
            onOff={() => {
              setNarrationOn(false);
              // Same place in the plain preview.
              window.setTimeout(() => playerRef.current?.seekTo(frame), 0);
            }}
          />
        </section>
        }
        right={
        <div ref={fieldsRef} style={{ display: "contents" }}>
        <Inspector
          timeline={timeline}
          selection={state.selection}
          tab={tab}
          setTab={setTab}
          playheadMs={frameToMs(frame, fps)}
          dispatch={fieldDispatch}
          onSeekMs={(ms) => seek(Math.round((ms * fps) / 1000))}
          docKey={docKey}
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
          listen={{
            engine: renderEngine,
            heard,
            onHeard,
            timelineJson: () => text,
            audioUrl: (path) => resolveAsset(path, assetRoot ? assetUrlFor(assetRoot) : null),
          }}
          voice={voice}
        />
        </div>
        }
        bottom={
      <EditorTimeline
        timeline={timeline}
        selection={state.selection}
        frame={frame}
        onSeek={seek}
        onSelect={select}
        dispatch={dispatch}
        beatMarks={marks}
        lengths={audioLengths}
        onDropVoiceover={(path, atMs) => {
          const err = addVoiceoverClip(path, atMs);
          if (err) setError(err);
        }}
      />
        }
      />
    </div>
    {prompter && (
      <Teleprompter
        timeline={timeline}
        playheadMs={frameToMs(frame, fps)}
        lengths={audioLengths}
        dispatch={dispatch}
        onSeekMs={(ms) => seek(Math.round((ms * fps) / 1000))}
        onClose={() => setPrompter(false)}
      />
    )}
    {screen === "home" && (
      <div className="st-home-layer">
        <Home
          onOpen={openProject}
          currentDir={project?.dir ?? null}
          onBack={() => setScreen("editor")}
          onOpenFile={() => void doOpen().then((opened) => opened && setScreen("editor"))}
        />
      </div>
    )}
    {leave && (
      <LeaveDialog
        name={docName}
        action={leave.action}
        saving={leaveSaving}
        onCancel={() => setLeave(null)}
        onDiscard={() => {
          const { then } = leave;
          setLeave(null);
          then();
        }}
        onSave={() => {
          const { then } = leave;
          setLeaveSaving(true);
          void doSave(false).then((saved) => {
            setLeaveSaving(false);
            if (!saved) return;
            setLeave(null);
            then();
          });
        }}
      />
    )}
    </>
  );
};
