// "Listen" (试听) before a render: asks the render's own narration code for clips. In the desktop
// app that is the `listen_narration` command; in the browser dev editor the dev server runs
// `vimmer tts` (see editor/assistant-server.ts). Either way the clips land in the render's TTS cache.
import { invoke } from "@tauri-apps/api/core";
import { inTauri } from "./project";
import type { ListenResult, TtsChoice } from "./narration";

const failed = (error: string): ListenResult => ({ ok: false, error });

/** Synthesizes `sceneIds` (all narrated scenes when empty) of the timeline with `tts`. */
export const listenNarration = async (timelineJson: string, sceneIds: string[], tts: TtsChoice): Promise<ListenResult> => {
  try {
    if (inTauri()) {
      return await invoke<ListenResult>("listen_narration", { args: { timelineJson, sceneIds, tts } });
    }
    const res = await fetch("/__vimmer/tts", {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ timelineJson, sceneIds, tts }),
    });
    if (res.status === 404) return failed("Listening needs the editor dev server (npm run editor) or the desktop app.");
    return (await res.json()) as ListenResult;
  } catch (e) {
    return failed(`Narration could not be made: ${String(e)}`);
  }
};
