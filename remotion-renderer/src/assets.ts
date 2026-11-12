import { staticFile } from "remotion";

// Asset paths in the timeline are relative to `public/`. Absolute URLs pass through untouched.
export const resolveAsset = (path: string): string =>
  /^https?:\/\//.test(path) ? path : staticFile(path);
