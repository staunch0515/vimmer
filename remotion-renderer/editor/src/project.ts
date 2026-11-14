import { invoke } from "@tauri-apps/api/core";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { ask, open, save } from "@tauri-apps/plugin-dialog";
import type { VideoTimeline } from "../../src/types/timeline";
import { templateInfo } from "./templates";

// Desktop (Tauri) and browser differ only in file access. Both paths live here,
// so the rest of the editor deals with one API.
export const inTauri = (): boolean => typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;

export const serialize = (t: VideoTimeline): string => JSON.stringify(t, null, 2) + "\n";

const FILTERS = [{ name: "Timeline JSON", extensions: ["json"] }];

export const baseName = (path: string | null): string => (path ? path.split(/[\\/]/).pop() || path : "Untitled");

export const isTimeline = (v: unknown): v is VideoTimeline => {
  const t = v as VideoTimeline;
  return !!t && typeof t.fps === "number" && Array.isArray(t.scenes) && !!t.resolution;
};

/** Parses a timeline document, filling optional fields the same way the renderer does. */
export const parseTimeline = (text: string): VideoTimeline => {
  const parsed: unknown = JSON.parse(text);
  if (!isTimeline(parsed)) throw new Error("This file is not a timeline (missing fps, resolution or scenes).");
  // Template scenes may leave out their length (the template's default applies, as in the render).
  const scenes = parsed.scenes.map((s) =>
    s.template && !s.duration_ms ? { ...s, duration_ms: templateInfo(s.template.name)?.default_ms ?? 4000 } : s,
  );
  return { ...parsed, scenes, subtitles: parsed.subtitles ?? [] };
};

/** Asks the user for a timeline to open. Resolves to null when the dialog is cancelled. */
export const pickToOpen = async (): Promise<{ path: string | null; text: string } | null> => {
  if (inTauri()) {
    const picked = await open({ multiple: false, directory: false, filters: FILTERS });
    if (!picked) return null;
    const path: string = picked;
    const text = await invoke<string>("read_project_file", { path });
    return { path, text };
  }
  return new Promise((resolve) => {
    const input = document.createElement("input");
    input.type = "file";
    input.accept = "application/json,.json";
    input.onchange = async () => {
      const file = input.files?.[0];
      resolve(file ? { path: null, text: await file.text() } : null);
    };
    input.addEventListener("cancel", () => resolve(null));
    input.click();
  });
};

/**
 * Saves the timeline. In the desktop app this writes to `path`, or asks for one when there is
 * none or when `saveAs` is set. In the browser it downloads a file. Resolves to the saved
 * path (null in the browser), or undefined when the user cancelled.
 */
export const saveTimeline = async (
  text: string,
  current: { path: string | null },
  opts: { saveAs: boolean },
): Promise<{ path: string | null } | undefined> => {
  if (!inTauri()) {
    const url = URL.createObjectURL(new Blob([text], { type: "application/json" }));
    const a = document.createElement("a");
    a.href = url;
    a.download = baseName(current.path) === "Untitled" ? "timeline.json" : baseName(current.path);
    a.click();
    URL.revokeObjectURL(url);
    return { path: null };
  }
  let target = current.path;
  if (!target || opts.saveAs) {
    const chosen = await save({ defaultPath: target ?? "timeline.json", filters: FILTERS });
    if (!chosen) return undefined;
    target = chosen;
  }
  await invoke("write_project_file", { path: target, contents: text });
  return { path: target };
};

/** Confirms before discarding unsaved changes. Uses the native dialog in the desktop app. */
export const confirmDiscard = async (message: string): Promise<boolean> => {
  if (inTauri()) return ask(message, { title: "Vimmer Studio", kind: "warning" });
  return window.confirm(message);
};

export const setWindowTitle = (title: string): void => {
  document.title = title;
  if (inTauri()) void getCurrentWindow().setTitle(title).catch(() => undefined);
};

/** Asks before the window closes while `shouldBlock()` is true. Desktop only; a no-op in the browser. */
export const guardClose = async (shouldBlock: () => boolean, message: string): Promise<() => void> => {
  if (!inTauri()) return () => undefined;
  return getCurrentWindow().onCloseRequested(async (event) => {
    if (shouldBlock() && !(await confirmDiscard(message))) event.preventDefault();
  });
};
