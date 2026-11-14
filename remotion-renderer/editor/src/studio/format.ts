// Pure helpers for the Studio screens (no React, no Tauri): tested in test/studio-format.test.ts.
import type { AssetInfo, AssetKind, Cost, ProjectSummary } from "./types";

/** `0:07`, `1:05`, `1:02:03`. */
export const formatDuration = (ms: number): string => {
  const total = Math.max(0, Math.round(ms / 1000));
  const h = Math.floor(total / 3600);
  const m = Math.floor((total % 3600) / 60);
  const s = total % 60;
  const ss = String(s).padStart(2, "0");
  return h > 0 ? `${h}:${String(m).padStart(2, "0")}:${ss}` : `${m}:${ss}`;
};

/** `512 B`, `1.2 KB`, `34 MB`, `1.5 GB`. */
export const formatBytes = (bytes: number): string => {
  if (bytes < 1000) return `${bytes} B`;
  const units = ["KB", "MB", "GB", "TB"];
  let value = bytes;
  let unit = -1;
  do {
    value /= 1000;
    unit += 1;
  } while (value >= 999.5 && unit < units.length - 1);
  return `${value >= 10 ? Math.round(value) : Math.round(value * 10) / 10} ${units[unit]}`;
};

/** `just now`, `5 min ago`, `3 h ago`, `yesterday`, `4 days ago`, then the date. */
export const formatWhen = (ms: number, now: number): string => {
  const diff = now - ms;
  const minute = 60_000;
  if (diff < minute) return "just now";
  if (diff < 60 * minute) return `${Math.floor(diff / minute)} min ago`;
  if (diff < 24 * 60 * minute) return `${Math.floor(diff / (60 * minute))} h ago`;
  const days = Math.floor(diff / (24 * 60 * minute));
  if (days === 1) return "yesterday";
  if (days < 7) return `${days} days ago`;
  const d = new Date(ms);
  const pad = (n: number) => String(n).padStart(2, "0");
  return `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())}`;
};

/** A cost for people: `$0.03`, `$0.012`, `$1.20`; under a tenth of a cent, `< $0.001`. */
export const formatUsd = (usd: number): string => {
  if (usd <= 0) return "$0";
  if (usd < 0.001) return "< $0.001";
  if (usd < 0.1) return `$${usd.toFixed(3).replace(/0$/, "")}`;
  return `$${usd.toFixed(2)}`;
};

/** `about $0.03 (at most about $0.11)`. */
export const formatCost = (c: Cost): string => `about ${formatUsd(c.typical)} (at most about ${formatUsd(c.worst)})`;

/** Projects whose name contains every word of the query, in any letter case. */
export const filterProjects = (projects: ProjectSummary[], query: string): ProjectSummary[] => {
  const words = query.toLowerCase().split(/\s+/).filter(Boolean);
  if (words.length === 0) return projects;
  return projects.filter((p) => {
    const name = p.name.toLowerCase();
    return words.every((w) => name.includes(w));
  });
};

export type AssetFilter = "all" | AssetKind | "unused";

export const ASSET_FILTERS: { value: AssetFilter; label: string }[] = [
  { value: "all", label: "All" },
  { value: "image", label: "Images" },
  { value: "video", label: "Video" },
  { value: "audio", label: "Audio" },
  { value: "icon", label: "Icons" },
  { value: "font", label: "Fonts" },
  { value: "unused", label: "Unused" },
];

export const filterAssets = (assets: AssetInfo[], filter: AssetFilter): AssetInfo[] => {
  if (filter === "all") return assets;
  if (filter === "unused") return assets.filter((a) => a.usedBy.length === 0);
  return assets.filter((a) => a.kind === filter);
};

/** How many assets each filter shows; filters with nothing (other than All) can be hidden. */
export const countAssets = (assets: AssetInfo[]): Record<AssetFilter, number> => {
  const counts = { all: assets.length, image: 0, icon: 0, video: 0, audio: 0, font: 0, unused: 0 };
  for (const a of assets) {
    counts[a.kind] += 1;
    if (a.usedBy.length === 0) counts.unused += 1;
  }
  return counts;
};

/** `1920×1080 · 0:04 · 2.1 MB`, leaving out what the file does not have. */
export const assetFacts = (a: AssetInfo): string =>
  [
    a.width && a.height ? `${a.width}×${a.height}` : null,
    a.durationMs != null ? formatDuration(a.durationMs) : null,
    formatBytes(a.bytes),
  ]
    .filter(Boolean)
    .join(" · ");

/** `Used by intro`, `Used by intro, outro and 2 more`. */
export const usedByLabel = (users: string[]): string => {
  if (users.length === 0) return "Not used";
  if (users.length <= 2) return `Used by ${users.join(", ")}`;
  return `Used by ${users.slice(0, 2).join(", ")} and ${users.length - 2} more`;
};

const EXTENSIONS: Record<AssetKind, string[]> = {
  image: ["png", "jpg", "jpeg", "webp", "gif"],
  icon: ["svg"],
  video: ["mp4", "mov", "m4v", "webm"],
  audio: ["wav", "mp3", "m4a", "aac", "aif", "aiff"],
  font: ["ttf", "otf", "woff", "woff2"],
};

/** Every extension a project can hold, for the import dialog. */
export const MEDIA_EXTENSIONS: string[] = Object.values(EXTENSIONS).flat();

/** The kind of a file by its name, or null for something a timeline cannot use. */
export const kindOfFile = (name: string): AssetKind | null => {
  const ext = name.includes(".") ? name.split(".").pop()!.toLowerCase() : "";
  const hit = (Object.keys(EXTENSIONS) as AssetKind[]).find((k) => EXTENSIONS[k].includes(ext));
  return hit ?? null;
};

/** A colour for a project's title card, stable for its name. */
export const cardHue = (name: string): number => {
  let hash = 0;
  for (const ch of name) hash = (hash * 31 + ch.codePointAt(0)!) % 360;
  return hash;
};
