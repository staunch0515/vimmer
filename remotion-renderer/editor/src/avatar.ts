// 数字人讲解人: the desktop commands (desktop/src-tauri/src/avatar.rs) behind the editor's avatar
// controls. The pure parts are in avatarUi.ts.
//
// The browser editor cannot generate (the engine runs only in the desktop app on an Apple-silicon
// Mac): it shows the disabled state with BROWSER_ENV, and the preview always draws the still
// photo. For the UI check, `window.__vimmerAvatarMock` stands in for the desktop (env, status,
// probe, and a generate that counts frames on a timer).
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { inTauri } from "./project";
import { openProjectDir } from "./studio/session";
import { BROWSER_ENV, type AvatarEnv, type AvatarRun, type AvatarStatus } from "./avatarUi";
import type { RenderProgress } from "./render";

export type ProbeResult = { ok: true; state: "found" | "no_face" | "unavailable" | "missing"; zh: string } | { ok: false; error: string };
export type AvatarFinished = { ok: boolean; cancelled: boolean; error: string | null; runs: AvatarRun[] };

type Mock = {
  env?: AvatarEnv;
  status?: AvatarStatus;
  probe?: ProbeResult;
  /** Frames the mock generation counts to (every 150 ms). */
  frames?: number;
};
const mock = (): Mock | null => (globalThis as { __vimmerAvatarMock?: Mock }).__vimmerAvatarMock ?? null;

export const avatarEnv = async (): Promise<AvatarEnv> => {
  const m = mock();
  if (m) return m.env ?? BROWSER_ENV;
  if (!inTauri()) return BROWSER_ENV;
  try {
    return await invoke<AvatarEnv>("avatar_env");
  } catch (e) {
    return { state: "unsupported", message: String(e), zh: `不能检查数字人引擎：${String(e)}` };
  }
};

export const avatarStatus = async (timelineJson: string): Promise<AvatarStatus | null> => {
  const m = mock();
  if (m) return m.status ?? null;
  if (!inTauri()) return null;
  try {
    return await invoke<AvatarStatus>("avatar_status", { args: { timelineJson } });
  } catch (e) {
    return { ok: false, error: String(e) };
  }
};

export const probeFace = async (photo: string): Promise<ProbeResult> => {
  const m = mock();
  if (m) return m.probe ?? { ok: true, state: "found", zh: "✓ 找到清晰的正脸" };
  if (!inTauri()) return { ok: true, state: "unavailable", zh: "浏览器编辑器不能检查人脸；在桌面应用里会检查。" };
  try {
    return await invoke<ProbeResult>("avatar_probe", { args: { photo, projectDir: openProjectDir() } });
  } catch (e) {
    return { ok: false, error: String(e) };
  }
};

let mockTimer: number | null = null;
let mockListeners: { progress: (p: RenderProgress) => void; finished: (f: AvatarFinished) => void } | null = null;

export const startAvatarGenerate = async (timelineJson: string, options: unknown): Promise<void> => {
  const m = mock();
  if (m) {
    const total = m.frames ?? 40;
    let done = 0;
    mockListeners?.progress({ kind: "stage", name: "Narrator avatar 1/1: preparing speech" });
    mockTimer = window.setInterval(() => {
      done += 1;
      mockListeners?.progress({ kind: "stage", name: `Narrator avatar 1/1: ${done}/${total} frames, about 1 min (约 1 分钟) left` });
      if (done >= total) {
        window.clearInterval(mockTimer!);
        mockTimer = null;
        const runs = m.status?.ok ? m.status.runs.map((r) => ({ ...r, ready: true })) : [];
        if (m.status?.ok) m.status = { ...m.status, known: true, runs, missing_estimate_s: 0 };
        mockListeners?.finished({ ok: true, cancelled: false, error: null, runs });
      }
    }, 150);
    return;
  }
  await invoke("start_avatar_generate", { args: { timelineJson, options, projectDir: openProjectDir() } });
};

export const cancelAvatarGenerate = async (): Promise<boolean> => {
  if (mock()) {
    if (mockTimer !== null) window.clearInterval(mockTimer);
    mockTimer = null;
    mockListeners?.finished({ ok: false, cancelled: true, error: null, runs: [] });
    return true;
  }
  return invoke<boolean>("cancel_avatar_generate");
};

/** Subscribes to the generation's progress and result. Returns a function that unsubscribes. */
export const listenAvatar = async (onProgress: (p: RenderProgress) => void, onFinished: (f: AvatarFinished) => void): Promise<() => void> => {
  mockListeners = { progress: onProgress, finished: onFinished };
  if (!inTauri()) return () => void (mockListeners = null);
  const unlisteners: UnlistenFn[] = await Promise.all([
    listen<RenderProgress>("avatar-progress", (e) => onProgress(e.payload)),
    listen<AvatarFinished>("avatar-finished", (e) => onFinished(e.payload)),
  ]);
  return () => {
    mockListeners = null;
    unlisteners.forEach((u) => u());
  };
};
