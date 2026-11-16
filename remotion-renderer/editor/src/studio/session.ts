// What the editor needs to know about the open project outside React: which asset paths are
// the project's own files (so previews load them from the project folder, not the public one),
// and where an import should go. App.tsx sets this when a project opens or closes; media.ts
// reads it. With no project open, everything behaves as it did before projects existed.
import { convertFileSrc, invoke } from "@tauri-apps/api/core";
import type { ImportedAsset } from "./types";

type OpenProject = { dir: string; files: Set<string>; onImport?: () => void };

let open: OpenProject | null = null;

/** Marks a project as open, with the project-relative paths of its media files. */
export const setOpenProject = (dir: string, files: Iterable<string>, onImport?: () => void): void => {
  open = { dir, files: new Set(files), onImport };
};

export const clearOpenProject = (): void => {
  open = null;
};

/** Replaces the known files of the open project (after its asset list was read again). */
export const setProjectFiles = (dir: string, files: Iterable<string>): void => {
  if (open?.dir === dir) open.files = new Set(files);
};

export const openProjectDir = (): string | null => open?.dir ?? null;

/** True when the path is a file of the open project (which then wins over the public folder). */
export const isProjectFile = (path: string): boolean => open?.files.has(path) ?? false;

/** The absolute path a timeline path stands for: the project's file, else under `publicRoot`. */
export const resolveAssetPath = (path: string, publicRoot: string): string =>
  open && open.files.has(path) ? `${open.dir}/${path}` : `${publicRoot}/${path}`;

/** The URL the webview loads a timeline asset from (desktop app). */
export const assetFileUrl = (path: string, publicRoot: string): string => convertFileSrc(resolveAssetPath(path, publicRoot));

/**
 * Copies a picked file into the open project and returns its project-relative path, or null
 * when no project is open (the caller then imports into the public folder, as before).
 */
export const importIntoOpenProject = async (source: string): Promise<string | null> => {
  const project = open;
  if (!project) return null;
  const [got] = await invoke<ImportedAsset[]>("import_assets", { dir: project.dir, sources: [source] });
  // Known at once, so the preview finds the file before the asset list is read again.
  project.files.add(got.path);
  project.onImport?.();
  return got.path;
};

/** A file the editor itself wrote into the open project (a recording): known at once. */
export const addProjectFile = (path: string): void => {
  if (!open) return;
  open.files.add(path);
  open.onImport?.();
};

// An unsaved project document is mirrored to localStorage like a plain one (App.tsx's draft);
// this remembers which project it belongs to, so the next launch reopens that project.
const DRAFT_PROJECT_KEY = "vimmer-studio-draft:project";

export const rememberDraftProject = (dir: string | null): void => {
  try {
    if (dir) window.localStorage.setItem(DRAFT_PROJECT_KEY, dir);
    else window.localStorage.removeItem(DRAFT_PROJECT_KEY);
  } catch {
    /* storage unavailable */
  }
};

export const draftProject = (): string | null => {
  try {
    return window.localStorage.getItem(DRAFT_PROJECT_KEY);
  } catch {
    return null;
  }
};
