// Rendering and checking a project's timeline. The same two commands the editor already uses
// (`start_render`, `check_before_render`), with the project folder added: its files are found
// first, the render goes to `<project>/renders/` and lands in the project's history.
import { invoke } from "@tauri-apps/api/core";
import { toPanelArgs, type CheckReport, type RenderDefaults, type RenderOptions, type TtsName } from "../renderOptions";
import type { RenderFinished } from "../render";
import type { ProjectSummary } from "./types";

/** Starts a render into the project's `renders/` folder (a date-time file name). */
export const startProjectRender = (projectDir: string, timelineJson: string, options: RenderOptions, defaults: RenderDefaults, avatarOff = false): Promise<void> =>
  invoke("start_render", { args: { timelineJson, projectDir, options: toPanelArgs(options, defaults, avatarOff) } });

export const checkProjectBeforeRender = (projectDir: string, timelineJson: string, tts: TtsName): Promise<CheckReport> =>
  invoke<CheckReport>("check_before_render", { args: { timelineJson, tts, projectDir } });

/** What `render-finished` carries for a project render, besides the fields the editor knew. */
export type ProjectRenderFinished = RenderFinished & { project?: ProjectSummary; notes?: string[] };
