// The one module the Studio screens call. `tauriApi` talks to the desktop shell
// (desktop/src-tauri/src/{studio,project_assets,renders,generate}.rs); `createFakeApi` in
// ./fake.ts serves the same interface from memory for the browser.
import { convertFileSrc, invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { getCurrentWebview } from "@tauri-apps/api/webview";
import { ask, open } from "@tauri-apps/plugin-dialog";
import { createContext, useContext } from "react";
import { MEDIA_EXTENSIONS } from "./format";
import type {
  AssetInfo,
  Estimate,
  GenerateArgs,
  GenerateFinished,
  GenerateProgress,
  ImportedAsset,
  ImportedProject,
  ExportedTimeline,
  Lang,
  OpenedProject,
  Orientation,
  ProjectSummary,
  RenderRecord,
  RendersChanged,
  StudioSettings,
} from "./types";

export interface StudioApi {
  // Settings and projects
  settings(): Promise<StudioSettings>;
  setProjectsRoot(root: string): Promise<StudioSettings>;
  listProjects(): Promise<ProjectSummary[]>;
  createProject(name: string, lang: Lang | null, orientation: Orientation): Promise<ProjectSummary>;
  openProject(dir: string): Promise<OpenedProject>;
  /** Saves the timeline atomically and updates the project's facts and thumbnail. */
  saveProject(dir: string, timelineJson: string): Promise<ProjectSummary>;
  renameProject(dir: string, name: string): Promise<ProjectSummary>;
  duplicateProject(dir: string): Promise<ProjectSummary>;
  /** Moves the project folder to the Trash. */
  trashProject(dir: string): Promise<void>;
  /** Makes a project from a timeline file, copying the files it names. */
  importTimeline(path: string): Promise<ImportedProject>;
  /** Writes the project's timeline to `path` and copies the files it names into `<name>-files/` next to it. */
  exportTimeline(dir: string, path: string, contents: string): Promise<ExportedTimeline>;
  /** Shows a project folder or one of its files in Finder. */
  reveal(path: string): Promise<void>;

  // Assets
  /** The project's media, with who uses each file in `timelineJson` (the editor's document). */
  listAssets(dir: string, timelineJson: string): Promise<AssetInfo[]>;
  importAssets(dir: string, sources: string[]): Promise<ImportedAsset[]>;
  /** Moves one unused file to the Trash. */
  trashAsset(dir: string, path: string, timelineJson: string): Promise<void>;
  /** Moves every unused file to the Trash; resolves to their paths. */
  trashUnusedAssets(dir: string, timelineJson: string): Promise<string[]>;

  // Renders
  listRenders(dir: string): Promise<RenderRecord[]>;
  /** Removes the entry; a file inside the project goes to the Trash. */
  trashRender(dir: string, id: string): Promise<RendersChanged>;
  addExistingRender(dir: string, source: string): Promise<RendersChanged>;

  // Generate from text
  /** The cost estimate. Free: no API call is made. */
  estimate(args: GenerateArgs): Promise<Estimate>;
  /** Starts the paid generation. Call only after the user confirmed the estimate. */
  startGenerate(args: GenerateArgs): Promise<void>;
  cancelGenerate(): Promise<boolean>;
  onGenerate(onProgress: (p: GenerateProgress) => void, onFinished: (f: GenerateFinished) => void): Promise<() => void>;

  // The shell: files, dialogs, drops
  /** A URL the webview can load for a file on disk (thumbnail, asset, render). */
  fileUrl(path: string, version?: number): string;
  pickTimelineFile(): Promise<string | null>;
  pickMediaFiles(): Promise<string[]>;
  pickVideoFile(): Promise<string | null>;
  pickTextFile(): Promise<{ name: string; text: string } | null>;
  pickFolder(): Promise<string | null>;
  confirm(message: string, okLabel: string): Promise<boolean>;
  /** Files dragged from Finder onto the window. Resolves to a function that stops listening. */
  onFilesDropped(handler: (paths: string[]) => void): Promise<() => void>;
}

const pickOne = async (name: string, extensions: string[]): Promise<string | null> => {
  const picked = await open({ multiple: false, directory: false, filters: [{ name, extensions }] });
  return typeof picked === "string" ? picked : null;
};

export const tauriApi: StudioApi = {
  settings: () => invoke("studio_settings"),
  setProjectsRoot: (root) => invoke("set_projects_root", { root }),
  listProjects: () => invoke("list_projects"),
  createProject: (name, lang, orientation) => invoke("create_project", { name, lang, orientation }),
  openProject: (dir) => invoke("open_project", { dir }),
  saveProject: (dir, timelineJson) => invoke("save_project", { dir, timelineJson }),
  renameProject: (dir, name) => invoke("rename_project", { dir, name }),
  duplicateProject: (dir) => invoke("duplicate_project", { dir }),
  trashProject: (dir) => invoke("trash_project", { dir }),
  importTimeline: (path) => invoke("import_timeline", { path }),
  exportTimeline: (dir, path, contents) => invoke("export_timeline", { dir, path, contents }),
  reveal: (path) => invoke("reveal_path", { path }),

  listAssets: (dir, timelineJson) => invoke("list_assets", { dir, timelineJson }),
  importAssets: (dir, sources) => invoke("import_assets", { dir, sources }),
  trashAsset: (dir, path, timelineJson) => invoke("trash_asset", { dir, path, timelineJson }),
  trashUnusedAssets: (dir, timelineJson) => invoke("trash_unused_assets", { dir, timelineJson }),

  listRenders: (dir) => invoke("list_renders", { dir }),
  trashRender: (dir, id) => invoke("trash_render", { dir, id }),
  addExistingRender: (dir, source) => invoke("add_existing_render", { dir, source }),

  estimate: (args) => invoke("generate_estimate", { args: { ...args, confirmed: false } }),
  // `confirmed` is set here and nowhere else: this is the call behind the confirm button.
  startGenerate: (args) => invoke("start_generate", { args: { ...args, confirmed: true } }),
  cancelGenerate: () => invoke("cancel_generate"),
  onGenerate: async (onProgress, onFinished) => {
    const off: UnlistenFn[] = await Promise.all([
      listen<GenerateProgress>("generate-progress", (e) => onProgress(e.payload)),
      listen<GenerateFinished>("generate-finished", (e) => onFinished(e.payload)),
    ]);
    return () => off.forEach((u) => u());
  },

  fileUrl: (path, version) => convertFileSrc(path) + (version ? `?v=${version}` : ""),
  pickTimelineFile: () => pickOne("Timeline JSON", ["json"]),
  pickMediaFiles: async () => {
    const picked = await open({ multiple: true, directory: false, filters: [{ name: "Media", extensions: MEDIA_EXTENSIONS }] });
    return Array.isArray(picked) ? picked : picked ? [picked] : [];
  },
  pickVideoFile: () => pickOne("Video", ["mp4", "mov", "m4v", "webm"]),
  pickTextFile: async () => {
    const path = await pickOne("Text", ["txt", "md"]);
    if (!path) return null;
    const text = await invoke<string>("read_project_file", { path });
    return { name: path.split(/[\\/]/).pop() ?? path, text };
  },
  pickFolder: async () => {
    const picked = await open({ multiple: false, directory: true });
    return typeof picked === "string" ? picked : null;
  },
  confirm: (message, okLabel) => ask(message, { title: "Vimmer Studio", kind: "warning", okLabel, cancelLabel: "Cancel" }),
  onFilesDropped: (handler) =>
    getCurrentWebview().onDragDropEvent((event) => {
      if (event.payload.type === "drop" && event.payload.paths.length > 0) handler(event.payload.paths);
    }),
};

/** The API the screens use. The desktop shell by default; a test or the browser harness swaps it. */
export const StudioApiContext = createContext<StudioApi>(tauriApi);
export const useStudioApi = (): StudioApi => useContext(StudioApiContext);

/** An error from a command, as text for the screen. */
export const errorText = (e: unknown): string => (e instanceof Error ? e.message : String(e));
