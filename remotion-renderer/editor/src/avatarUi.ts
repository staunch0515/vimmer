// 数字人讲解人 (narrator avatar) in the editor, as plain data and pure functions (no Tauri, no
// React), so test/editor-avatar.test.ts can check them with node. Only type imports here.
//
// - Which avatar a scene shows mirrors `resolve` in rust-core/crates/vimmer-dsl/src/avatar.rs.
// - The status comes from the desktop `avatar_status` command (desktop/src-tauri/src/avatar.rs),
//   which reads what `vimmer avatar generate` recorded; progress from the scheduler's stage lines.
// - The preview draws the still photo, dimmed, until a clip exists, then the real clip.
import type { Avatar, AvatarClip, LogoCorner, Scene, SceneAvatar, VideoTimeline } from "../../src/types/timeline";
import type { CheckFinding } from "./renderOptions";

export const AVATAR_DEFAULTS = { corner: "bottom_right" as LogoCorner, size: 0.3, margin: 0.04, shape: "circle" as const, borderWidth: 0.02, borderColor: "#FFFFFF" };
export const CORNER_LABELS: Record<LogoCorner, string> = { top_left: "左上", top_right: "右上", bottom_left: "左下", bottom_right: "右下" };
export const CORNERS: LogoCorner[] = ["top_left", "top_right", "bottom_left", "bottom_right"];

/** The timeline's own avatar, else the brand's (the timeline's replaces the brand's). */
export const defaultAvatar = (t: VideoTimeline): Avatar | null => t.avatar ?? t.brand?.avatar ?? null;

export type ResolvedAvatar = { photo: string; corner: LogoCorner; size: number; margin: number; shape: "circle" | "rounded"; borderWidth: number; borderColor: string };

const resolved = (d: Avatar, photo: string, corner: LogoCorner | undefined): ResolvedAvatar => ({
  photo,
  corner: corner ?? d.corner ?? AVATAR_DEFAULTS.corner,
  size: d.size ?? AVATAR_DEFAULTS.size,
  margin: d.margin ?? AVATAR_DEFAULTS.margin,
  shape: d.shape ?? AVATAR_DEFAULTS.shape,
  borderWidth: d.border ? (d.border.width ?? AVATAR_DEFAULTS.borderWidth) : 0,
  borderColor: d.border?.color ?? AVATAR_DEFAULTS.borderColor,
});

/** The avatar scene `i` shows, or null (off). */
export const sceneAvatar = (t: VideoTimeline, i: number): ResolvedAvatar | null => {
  const own = t.scenes[i]?.avatar ?? undefined;
  const d = defaultAvatar(t);
  if (d) {
    const on = own?.enabled ?? d.enabled ?? true;
    return on ? resolved(d, own?.photo ?? d.photo, own?.corner) : null;
  }
  if (own?.enabled === false || !own?.photo) return null;
  return resolved({ photo: own.photo }, own.photo, own.corner);
};

export const anySceneAvatar = (t: VideoTimeline): boolean => t.scenes.some((_, i) => sceneAvatar(t, i) !== null);

// ---- Edits (each is one reducer action, so one undo step) ----

/** Drops undefined keys, so the saved JSON has only what was set. */
const compact = <T extends object>(o: T): T => Object.fromEntries(Object.entries(o).filter(([, v]) => v !== undefined)) as T;

/**
 * The timeline with its avatar changed by `patch`. Starts from the brand's avatar when the
 * timeline has none, so changing a look control keeps the brand's photo. `null` removes it.
 */
export const withAvatar = (t: VideoTimeline, patch: Partial<Avatar> | null): VideoTimeline => {
  const { avatar: _old, ...rest } = t;
  if (patch === null) return rest;
  const next = compact({ ...(defaultAvatar(t) ?? { photo: "" }), ...patch });
  return next.photo ? { ...rest, avatar: next } : rest;
};

/** Scene `i` with its own avatar settings changed by `patch` (null or an empty result: none). */
export const withSceneAvatar = (t: VideoTimeline, i: number, patch: Partial<SceneAvatar> | null): VideoTimeline => ({
  ...t,
  scenes: t.scenes.map((s, j): Scene => {
    if (j !== i) return s;
    const { avatar: _old, ...rest } = s;
    if (patch === null) return rest;
    const next = compact({ ...(s.avatar ?? {}), ...patch });
    if (next.photo === "") delete next.photo;
    return Object.keys(next).length ? { ...rest, avatar: next } : rest;
  }),
});

/**
 * The one-click fix of an `avatar.over_text` finding: its suggestion sets `/scenes/i/avatar`
 * (to `{corner}`) or `/scenes/i/avatar/corner`. Returns the scene and the corner, or null.
 */
export const overTextFix = (f: CheckFinding): { index: number; corner: LogoCorner } | null => {
  if (f.code !== "avatar.over_text" || !f.suggestion.path) return null;
  const m = /^\/scenes\/(\d+)\/avatar(\/corner)?$/.exec(f.suggestion.path);
  if (!m) return null;
  const v = f.suggestion.value as unknown;
  const corner = m[2] ? v : (v as { corner?: unknown } | null)?.corner;
  return CORNERS.includes(corner as LogoCorner) ? { index: Number(m[1]), corner: corner as LogoCorner } : null;
};

// ---- Status (desktop `avatar_status`) ----

export type AvatarRun = { scenes: string[]; photo: string; clip: string; ready: boolean; duration_ms: number; estimate_s: number };
export type AvatarStatus =
  | { ok: true; known: boolean; runs: AvatarRun[]; missing_estimate_s: number; speaking_estimate_s: number }
  | { ok: false; error: string };
export type AvatarEnvState = "ready" | "not_installed" | "unsupported" | "browser";
export type AvatarEnv = { state: AvatarEnvState; message: string; zh: string };

export const BROWSER_ENV: AvatarEnv = {
  state: "browser",
  message: "the browser editor cannot generate avatar clips",
  zh: "浏览器编辑器不能生成数字人：请在 iMac 的桌面应用里点「生成数字人」。",
};

/** "约 6 分钟" / "约 40 秒", like `describe_estimate` in vimmer-media. */
export const aboutTime = (secs: number): string => (secs < 90 ? `约 ${Math.round(secs)} 秒` : `约 ${Math.round(secs / 60)} 分钟`);

export type StatusLine = { ready: boolean; label: string; scenes: string[] };

/** One line per run: 「✓ 已生成」 or 「○ 未生成 · 约 N 分钟」. Unknown: one line for the timeline. */
export const statusLines = (s: AvatarStatus | null): StatusLine[] => {
  if (!s || !s.ok) return [];
  if (!s.known) return s.missing_estimate_s > 0 ? [{ ready: false, label: `○ 未生成 · ${aboutTime(s.missing_estimate_s)}`, scenes: [] }] : [];
  return s.runs.map((r) => ({ ready: r.ready, scenes: r.scenes, label: r.ready ? "✓ 已生成" : `○ 未生成 · ${aboutTime(r.estimate_s)}` }));
};

/** Whether some clip still needs generating. */
export const needsGenerating = (s: AvatarStatus | null): boolean => !!s && s.ok && s.missing_estimate_s > 0;

// ---- Progress (scheduler stage lines while generating) ----

export type AvatarProgress = { run: number; runs: number; done: number; total: number; text: string };

/**
 * Reads a stage line such as "Narrator avatar 1/2: 100/775 frames, about 5 min (约 5 分钟) left".
 * Lines before the avatar step (narration) keep run 0.
 */
export const parseProgress = (prev: AvatarProgress | null, name: string): AvatarProgress => {
  const base = prev ?? { run: 0, runs: 0, done: 0, total: 0, text: "" };
  const m = /^Narrator avatar (\d+)\/(\d+): (.*)$/.exec(name);
  if (!m) return { ...base, text: name };
  const [run, runs, rest] = [Number(m[1]), Number(m[2]), m[3]];
  const f = /^(\d+)\/(\d+) frames/.exec(rest);
  if (f) return { run, runs, done: Number(f[1]), total: Number(f[2]), text: rest };
  if (rest.startsWith("head pose")) return { ...base, run, runs }; // a log line, not a stage
  return { run, runs, done: run === base.run ? base.done : 0, total: run === base.run ? base.total : 0, text: rest };
};

/** Overall progress in [0, 1] across runs. */
export const progressFraction = (p: AvatarProgress | null): number => {
  if (!p || p.runs === 0) return 0;
  const inRun = p.total > 0 ? p.done / p.total : 0;
  return Math.min(1, (p.run - 1 + inRun) / p.runs);
};

// ---- Preview (the inset before and after generation) ----

/** The editor-only preview fields of an inset (never saved, never sent to the render). */
export type PreviewClip = AvatarClip & { preview_photo?: string; preview_badge?: string };

/**
 * The timeline the preview Player draws: every scene showing an avatar gets an inset. A scene
 * whose clip is ready plays it (muted, at the scene's offset into the run, so it follows the
 * preview time); otherwise the still photo is drawn dimmed with 「未生成 · 约 N 分钟」. A scene
 * that already has a render-time `avatar_clip` keeps it.
 */
export const withPreviewAvatars = (t: VideoTimeline, status: AvatarStatus | null): VideoTimeline => {
  if (!anySceneAvatar(t)) return t;
  const runs = status?.ok && status.known ? status.runs : [];
  const fps = t.fps;
  const starts: number[] = [];
  let cursor = 0;
  for (const s of t.scenes) {
    starts.push(cursor);
    cursor += Math.round((s.duration_ms * fps) / 1000);
  }
  const index = new Map(t.scenes.map((s, i) => [s.id, i]));
  // Not recorded yet: the estimate is for every speaking scene together.
  const badge = status?.ok && !status.known && status.missing_estimate_s > 0 ? `未生成 · ${aboutTime(status.missing_estimate_s)}` : "未生成";
  return {
    ...t,
    scenes: t.scenes.map((scene, i): Scene => {
      const a = sceneAvatar(t, i);
      if (!a || scene.avatar_clip) return scene;
      const look = { corner: a.corner, size: a.size, margin: a.margin, shape: a.shape, border_width: a.borderWidth, border_color: a.borderColor };
      const run = runs.find((r) => r.scenes.includes(scene.id));
      if (run?.ready) {
        const first = index.get(run.scenes[0]) ?? i;
        return { ...scene, avatar_clip: { src: run.clip, offset_frames: starts[i] - starts[first], ...look } };
      }
      const label = run ? `未生成 · ${aboutTime(run.estimate_s)}` : badge;
      const clip: PreviewClip = { src: "", offset_frames: 0, ...look, preview_photo: a.photo, preview_badge: label };
      return { ...scene, avatar_clip: clip };
    }),
  };
};

// ---- Render panel ----

export type AvatarRenderWarning = { level: "warn" | "block"; text: string } | null;

/**
 * What the render panel says about the avatar: nothing without one or when every clip is ready;
 * with clips missing, 「这次渲染不会显示讲解人」 (show) or that the render will stop (require).
 */
export const avatarRenderWarning = (t: VideoTimeline, status: AvatarStatus | null, requireAvatar: boolean): AvatarRenderWarning => {
  if (!anySceneAvatar(t) || !needsGenerating(status)) return null;
  return requireAvatar
    ? { level: "block", text: "数字人还没生成：选了「必须带数字人」，渲染会停下。请先生成。" }
    : { level: "warn", text: "数字人还没生成，这次渲染不会显示讲解人" };
};
