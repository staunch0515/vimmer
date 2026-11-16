// Shapes shared by the Studio screens. They mirror the Rust structs in
// desktop/src-tauri/src/{studio,project_assets,renders,generate}.rs (serde camelCase).

export type Lang = "zh" | "ja" | "en";
export type Orientation = "landscape" | "portrait";
export type Origin = "blank" | "generated" | "imported";

export type ProjectSummary = {
  /** Absolute path of the project folder; the project's identity in every call. */
  dir: string;
  name: string;
  createdMs: number;
  updatedMs: number;
  lang: Lang | null;
  orientation: Orientation;
  durationMs: number;
  origin: Origin;
  renders: number;
  /** Absolute path of thumb.jpg, when there is one. */
  thumb: string | null;
  thumbVersion: number;
};

export type RenderRecord = {
  id: string;
  /** Project-relative (`renders/x.mp4`), or absolute when saved elsewhere. */
  file: string;
  finishedMs: number;
  bytes: number;
  durationMs: number;
  options: Record<string, unknown> | null;
  imported: boolean;
  /** Absolute path. */
  path: string;
  missing: boolean;
};

export type OpenedProject = {
  project: ProjectSummary;
  timelineJson: string;
};

export type ImportedProject = {
  project: ProjectSummary;
  copied: string[];
  /** Files the timeline names that were found nowhere. */
  missing: string[];
  /** Files not next to the timeline but found in another project (also in `copied`). */
  borrowed: { path: string; project: string }[];
};

/** "Save as" from a project: the timeline, and the project files it names in `filesDir`. */
export type ExportedTimeline = {
  path: string;
  filesDir: string | null;
  copied: string[];
  missing: string[];
};

export type StudioSettings = {
  root: string;
  defaultRoot: string;
  isDefault: boolean;
  recent: ProjectSummary[];
};

export type AssetKind = "image" | "icon" | "video" | "audio" | "font";

export type AssetInfo = {
  /** Project-relative path, as the timeline names the file (`assets/pic.png`). */
  path: string;
  name: string;
  kind: AssetKind;
  bytes: number;
  modifiedMs: number;
  width: number | null;
  height: number | null;
  durationMs: number | null;
  /** Scene ids (and "Music", "Brand") that use the file. */
  usedBy: string[];
  /** Absolute path, for the preview. */
  file: string;
};

export type ImportedAsset = { source: string; path: string; reused: boolean };

export type RendersChanged = { project: ProjectSummary; renders: RenderRecord[] };

export type GenerateModel = "claude-opus-5-5" | "claude-sonnet-5-5";

export type GenerateArgs = {
  text: string;
  lang: Lang;
  orientation: Orientation;
  targetSeconds: number | null;
  model: GenerateModel;
  illustrate: boolean;
  /** Illustration detail; missing means `rich`. */
  detail?: IllustrationDetail;
  name: string | null;
};

/** `rich`: layered, lit, textured pictures; `simple`: the original flat ones (cheaper). */
export type IllustrationDetail = "rich" | "simple";

export type Cost = { typical: number; worst: number };

export type Estimate = {
  model: string;
  inputTokens: number;
  outputTokens: number;
  scenes: number;
  storyboard: Cost;
  illustrate: Cost | null;
  total: Cost;
  /** Something that stops a generation before any call (the Python helper is missing). */
  problem: string | null;
};

export type GenerateProgress = { stage: "storyboard" | "illustrations" | "project"; message: string };

export type GenerateFinished = {
  ok: boolean;
  cancelled: boolean;
  project: ProjectSummary | null;
  costUsd: number;
  inputTokens: number;
  outputTokens: number;
  scenes: number;
  illustrated: number;
  warnings: string[];
  errorKind: string | null;
  error: string | null;
};
