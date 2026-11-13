import { useState, type Dispatch } from "react";
import type { VideoTimeline } from "../../../src/types/timeline";
import type { Selection } from "../model";
import type { Action } from "../state";
import { serialize } from "../project";
import {
  ASSISTANT_MODELS, failureHint, formatCost, proposeEdit,
  type AssistantModel, type EditFailure, type EditProposal,
} from "../assistant";

type Props = {
  timeline: VideoTimeline;
  selection: Selection;
  dispatch: Dispatch<Action>;
};

type View =
  | { kind: "idle" }
  | { kind: "busy" }
  | { kind: "proposal"; proposal: EditProposal; base: string; instruction: string }
  | { kind: "failed"; failure: EditFailure };

const STATUS_LABEL = { edited: "Proposed edit", clarify: "Needs more detail", refused: "Not possible" } as const;

const selectedSceneId = (t: VideoTimeline, s: Selection): string | null => {
  const index = s.kind === "scene" ? s.index : s.kind === "keyframe" ? s.sceneIndex : -1;
  return t.scenes[index]?.id ?? null;
};

/** Natural-language editing. Nothing changes until Accept; Accept is one undo step. */
export const AssistantPanel = ({ timeline, selection, dispatch }: Props) => {
  const [instruction, setInstruction] = useState("");
  const [useScene, setUseScene] = useState(true);
  const [model, setModel] = useState<AssistantModel>("claude-opus-5-5");
  const [view, setView] = useState<View>({ kind: "idle" });
  const sceneId = selectedSceneId(timeline, selection);
  const current = serialize(timeline);
  const stale = view.kind === "proposal" && view.base !== current;

  const propose = async () => {
    const text = instruction.trim();
    if (!text || view.kind === "busy") return;
    setView({ kind: "busy" });
    const result = await proposeEdit({
      timelineJson: current,
      instruction: text,
      sceneId: useScene ? sceneId : null,
      model,
    });
    setView(result.ok ? { kind: "proposal", proposal: result, base: current, instruction: text } : { kind: "failed", failure: result });
  };

  const accept = () => {
    if (view.kind !== "proposal" || stale) return;
    dispatch({ type: "applyAssistant", timeline: view.proposal.outcome.timeline });
    setInstruction("");
    setView({ kind: "idle" });
  };

  return (
    <div className="form assistant">
      <div className="section-title">Edit with Claude</div>
      <textarea
        rows={3}
        value={instruction}
        placeholder="e.g. 第二段节奏快一点，加一个淡入转场 · make scene 3 slower · switch to vertical"
        onChange={(e) => setInstruction(e.target.value)}
        onKeyDown={(e) => {
          if (e.key === "Enter" && (e.metaKey || e.ctrlKey)) {
            e.preventDefault();
            void propose();
          }
        }}
        aria-label="Editing instruction"
      />
      <label className="check">
        <input type="checkbox" checked={useScene && !!sceneId} disabled={!sceneId} onChange={(e) => setUseScene(e.target.checked)} />
        {sceneId ? <>About the selected scene <span className="mono">{sceneId}</span></> : "No scene selected"}
      </label>
      <div className="assistant-row">
        <select value={model} onChange={(e) => setModel(e.target.value as AssistantModel)} aria-label="Model">
          {ASSISTANT_MODELS.map((m) => <option key={m.id} value={m.id}>{m.label}</option>)}
        </select>
        <button className="btn primary" onClick={() => void propose()} disabled={!instruction.trim() || view.kind === "busy"}>
          {view.kind === "busy" ? "Thinking…" : "Propose"}
        </button>
      </div>
      <div className="dim small">Each proposal is a paid API call. Nothing changes until you accept. ⌘↵ proposes.</div>

      {view.kind === "failed" && (
        <div className="assistant-card bad" role="alert">
          <div className="assistant-status">Assistant unavailable</div>
          <div>{view.failure.error.message}</div>
          {failureHint(view.failure) && <div className="dim">{failureHint(view.failure)}</div>}
          {view.failure.usage && view.failure.usage.cost_usd > 0 && <div className="dim mono small">{formatCost(view.failure.usage)}</div>}
        </div>
      )}

      {view.kind === "proposal" && (
        <div className={`assistant-card ${view.proposal.status}`}>
          <div className="assistant-status">{STATUS_LABEL[view.proposal.status]}</div>
          <div className="dim small">“{view.instruction}”</div>
          <div>{view.proposal.explanation}</div>
          {view.proposal.outcome.summary.length > 0 && (
            <ul className="assistant-diff">
              {view.proposal.outcome.summary.map((line) => <li key={line} className="mono">{line}</li>)}
            </ul>
          )}
          {view.proposal.outcome.warnings.map((w) => <div key={w} className="assistant-warning">⚠ {w}</div>)}
          <div className="dim mono small">
            {formatCost(view.proposal.usage)}
            {view.proposal.retries > 0 ? ` · ${view.proposal.retries} correction${view.proposal.retries > 1 ? "s" : ""}` : ""}
          </div>
          {stale && <div className="assistant-warning">The timeline changed since this was proposed. Propose again.</div>}
          <div className="assistant-row">
            {view.proposal.status === "edited" && (
              <button className="btn primary" onClick={accept} disabled={stale}>Accept</button>
            )}
            <button className="btn" onClick={() => setView({ kind: "idle" })}>
              {view.proposal.status === "edited" ? "Reject" : "Dismiss"}
            </button>
          </div>
        </div>
      )}
    </div>
  );
};
