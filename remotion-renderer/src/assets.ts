import { createContext } from "react";
import { staticFile } from "remotion";

// Maps a timeline asset path (relative to `public/`) to a URL. The editor's desktop shell
// provides a function that reads files from disk, so new files show without a rebuild.
// Without a provider (the renderer, the browser), files come from the bundled public folder.
export type AssetUrl = (path: string) => string;
export const AssetUrlContext = createContext<AssetUrl | null>(null);

export const resolveAsset = (path: string, provider: AssetUrl | null): string => {
  if (/^https?:\/\//.test(path)) return path;
  return provider ? provider(path) : staticFile(path);
};
