import { invoke } from "@tauri-apps/api/core";
import { open } from "@tauri-apps/plugin-dialog";
import { inTauri } from "./project";
import { assetFileUrl, importIntoOpenProject } from "./studio/session";
import type { AssetUrl } from "../../src/assets";

export type MediaKind = "image" | "audio" | "icon" | "video";

const FILTERS: Record<MediaKind, { name: string; extensions: string[] }> = {
  image: { name: "Images", extensions: ["png", "jpg", "jpeg", "webp"] },
  audio: { name: "Audio", extensions: ["wav", "mp3", "m4a", "aac", "aif", "aiff"] },
  icon: { name: "SVG icons", extensions: ["svg"] },
  video: { name: "Video clips", extensions: ["mp4", "mov", "m4v", "webm"] },
};

export const canChooseMedia = (): boolean => inTauri();

/**
 * Asks for a file, copies it into the open project (or, with no project open, into the public
 * folder) and returns the path the timeline names it by. Resolves to null when the dialog is
 * cancelled.
 */
export const chooseAndImport = async (kind: MediaKind): Promise<string | null> => {
  const picked = await open({ multiple: false, directory: false, filters: [FILTERS[kind]] });
  if (!picked) return null;
  const source: string = picked;
  return (await importIntoOpenProject(source)) ?? invoke<string>("import_asset", { source });
};

/**
 * Builds asset URLs from the files on disk, so preview shows files added after the build: a file
 * of the open project from its folder, anything else from the public folder (`root`).
 */
export const assetUrlFor = (root: string): AssetUrl => (path) => assetFileUrl(path, root);

export const loadAssetRoot = (): Promise<string> => invoke<string>("asset_root");
