import { convertFileSrc, invoke } from "@tauri-apps/api/core";
import { open } from "@tauri-apps/plugin-dialog";
import { inTauri } from "./project";
import type { AssetUrl } from "../../src/assets";

export type MediaKind = "image" | "audio";

const FILTERS: Record<MediaKind, { name: string; extensions: string[] }> = {
  image: { name: "Images", extensions: ["png", "jpg", "jpeg", "webp"] },
  audio: { name: "Audio", extensions: ["wav", "mp3", "m4a", "aac", "aif", "aiff"] },
};

export const canChooseMedia = (): boolean => inTauri();

/**
 * Asks for a file, copies it into the public folder, and returns its public-relative path.
 * Resolves to null when the dialog is cancelled.
 */
export const chooseAndImport = async (kind: MediaKind): Promise<string | null> => {
  const picked = await open({ multiple: false, directory: false, filters: [FILTERS[kind]] });
  if (!picked) return null;
  const source: string = picked;
  return invoke<string>("import_asset", { source });
};

/** Builds asset URLs from the public folder on disk, so preview shows files added after the build. */
export const assetUrlFor = (root: string): AssetUrl => (path) => convertFileSrc(`${root}/${path}`);

export const loadAssetRoot = (): Promise<string> => invoke<string>("asset_root");
