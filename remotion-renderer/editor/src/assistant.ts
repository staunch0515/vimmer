import { invoke } from "@tauri-apps/api/core";
import type { VideoTimeline } from "../../src/types/timeline";
import { inTauri } from "./project";

// The editing assistant: Claude proposes a JSON Patch, the Rust core checks it
// (`vimmer edit --dry-run --json`), and the editor shows the result. The API key stays with the
// backend process (desktop shell or the local Vite dev server); it never reaches this page.

export const ASSISTANT_MODELS = [
  { id: "claude-opus-5-5", label: "Claude Opus 5.5" },
  { id: "claude-sonnet-5-5", label: "Claude Sonnet 5.5" },
  { id: "claude-haiku-5-5", label: "Claude Haiku 5.5" },
] as const;

export type AssistantModel = (typeof ASSISTANT_MODELS)[number]["id"];

export type EditUsage = {
  input_tokens: number;
  output_tokens: number;
  cache_creation_input_tokens: number;
  cache_read_input_tokens: number;
  cost_usd: number;
};

/** Mirrors `vimmer_storyboard::edit_claude::Proposal` as printed by `vimmer edit --json`. */
export type EditProposal = {
  ok: true;
  status: "edited" | "clarify" | "refused";
  explanation: string;
  patch: unknown[];
  outcome: { timeline: VideoTimeline; summary: string[]; warnings: string[]; changed: boolean };
  usage: EditUsage;
  retries: number;
};

export type EditFailure = {
  ok: false;
  error: { kind: string; message: string };
  usage?: EditUsage | null;
};

export type EditResult = EditProposal | EditFailure;

export type ProposeArgs = {
  timelineJson: string;
  instruction: string;
  sceneId: string | null;
  model: AssistantModel;
};

const DEV_ENDPOINT = "/__vimmer/edit";

const failure = (kind: string, message: string): EditFailure => ({ ok: false, error: { kind, message } });

/** Asks the backend for a proposal. Never throws: problems come back as an `EditFailure`. */
export const proposeEdit = async (args: ProposeArgs): Promise<EditResult> => {
  try {
    if (inTauri()) return await invoke<EditResult>("propose_edit", { args });
    const res = await fetch(DEV_ENDPOINT, {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify(args),
    });
    if (res.status === 404) {
      return failure(
        "unavailable",
        "The assistant needs the desktop app or the editor dev server (npm run editor). A static build has no backend.",
      );
    }
    const body = (await res.json()) as EditResult;
    return body;
  } catch (e) {
    return failure("unavailable", `Could not reach the assistant backend: ${String(e)}`);
  }
};

/** A clear next step for the common setup problems. */
export const failureHint = (f: EditFailure): string | null => {
  switch (f.error.kind) {
    case "no_api_key":
      return "Set ANTHROPIC_API_KEY in ~/.zshrc, then restart the editor. The rest of the editor works without it.";
    case "helper":
      return "Install the Claude helper: sh rust-core/crates/vimmer-storyboard/scripts/install_llm.sh";
    case "no_binary":
      return "Build the CLI first: cd rust-core && cargo build -p vimmer-cli (or set VIMMER_BIN).";
    case "invalid":
      return "Claude's patch kept failing the checks. Rephrase the request or make it smaller.";
    default:
      return null;
  }
};

export const formatCost = (u: EditUsage | null | undefined): string =>
  u ? `$${u.cost_usd.toFixed(4)} · ${u.input_tokens + u.cache_read_input_tokens + u.cache_creation_input_tokens} in / ${u.output_tokens} out tokens` : "";
