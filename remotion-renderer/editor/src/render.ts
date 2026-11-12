import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { save } from "@tauri-apps/plugin-dialog";
import { inTauri } from "./project";

// Mirrors `vimmer_scheduler::Progress` (serde tag "kind", snake_case).
export type RenderProgress =
  | { kind: "stage"; name: string }
  | { kind: "images"; done: number; total: number }
  | { kind: "chunks"; done: number; total: number }
  | { kind: "done"; output: string };

// Mirrors the `Finished` payload in desktop/src-tauri/src/render.rs.
export type RenderFinished = {
  ok: boolean;
  cancelled: boolean;
  output: string | null;
  error: string | null;
};

export const renderSupported = (): boolean => inTauri();

export const pickRenderOutput = (defaultName: string): Promise<string | null> =>
  save({ defaultPath: defaultName, filters: [{ name: "MP4 video", extensions: ["mp4"] }] });

export const startRender = (timelineJson: string, output: string): Promise<void> =>
  invoke("start_render", { args: { timelineJson, output } });

export const cancelRender = (): Promise<boolean> => invoke<boolean>("cancel_render");

export const revealInFinder = (path: string): Promise<void> => invoke("reveal_in_finder", { path });

/** Subscribes to render progress and completion. Returns a function that unsubscribes both. */
export const listenRender = async (
  onProgress: (p: RenderProgress) => void,
  onFinished: (f: RenderFinished) => void,
): Promise<() => void> => {
  const unlisteners: UnlistenFn[] = await Promise.all([
    listen<RenderProgress>("render-progress", (e) => onProgress(e.payload)),
    listen<RenderFinished>("render-finished", (e) => onFinished(e.payload)),
  ]);
  return () => unlisteners.forEach((u) => u());
};

/** What the progress card shows. */
export type RenderView =
  | { status: "idle" }
  | { status: "running"; stage: string; images?: { done: number; total: number }; chunks?: { done: number; total: number } }
  | { status: "done"; output: string }
  | { status: "cancelled" }
  | { status: "failed"; error: string };

/** Overall progress in [0, 1]: images 0–20 %, chunks 20–90 %, joining 90–100 %. */
export const fraction = (v: RenderView): number => {
  if (v.status === "done") return 1;
  if (v.status !== "running") return 0;
  if (v.chunks && v.chunks.total > 0) return 0.2 + 0.7 * (v.chunks.done / v.chunks.total);
  if (v.images && v.images.total > 0) return 0.2 * (v.images.done / v.images.total);
  return v.stage === "Joining segments" ? 0.92 : 0.02;
};

/** Applies one progress event to the current view. */
export const applyProgress = (view: RenderView, p: RenderProgress): RenderView => {
  const running: RenderView = view.status === "running" ? view : { status: "running", stage: "Starting" };
  switch (p.kind) {
    case "stage":
      return { ...running, stage: p.name };
    case "images":
      return { ...running, images: { done: p.done, total: p.total } };
    case "chunks":
      return { ...running, chunks: { done: p.done, total: p.total } };
    case "done":
      return running;
  }
};
