// 按旁白对齐场景: asks the render's code (local whisper) for scene lengths that put the cuts at the
// voice-over's sentence boundaries. Desktop app: the `align_scenes` command; browser editor: the
// dev server's /__vimmer/align (which runs `vimmer align-scenes`). The pure part (applying) is in
// voiceover.ts.
import { invoke } from "@tauri-apps/api/core";
import { inTauri } from "./project";
import { openProjectDir } from "./studio/session";

export type SceneChange = {
  index: number;
  id: string;
  old_ms: number;
  new_ms: number;
  said_from_ms: number | null;
  said_to_ms: number | null;
};

export type AlignResult =
  | { ok: true; clip: string; changes: SceneChange[]; durations: number[]; notes: string[] }
  | { ok: false; error: string };

export const alignScenes = async (timelineJson: string, clip: string | null): Promise<AlignResult> => {
  try {
    if (inTauri()) {
      return await invoke<AlignResult>("align_scenes", { args: { timelineJson, clip, dir: openProjectDir() } });
    }
    const res = await fetch("/__vimmer/align", {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ timelineJson, clip }),
    });
    if (res.status === 404) return { ok: false, error: "对齐需要编辑器开发服务器（npm run editor）或桌面应用。" };
    return (await res.json()) as AlignResult;
  } catch (e) {
    return { ok: false, error: `对齐没有完成：${String(e)}` };
  }
};
