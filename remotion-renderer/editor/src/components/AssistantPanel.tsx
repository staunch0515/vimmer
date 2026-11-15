import { useEffect, useReducer, useRef, useState, type Dispatch } from "react";
import type { VideoTimeline } from "../../../src/types/timeline";
import type { Selection } from "../model";
import type { Action } from "../state";
import { serialize } from "../project";
import {
  ASSISTANT_MODELS, failureHint, formatCost, proposeEdit,
  type AssistantModel, type EditProposal,
} from "../assistant";
import {
  buildConversation, busy as isBusy, chatReducer, failureOf, initialChat, nextExchangeId, openProposal,
  type Exchange,
} from "../chat";

type Props = {
  timeline: VideoTimeline;
  selection: Selection;
  dispatch: Dispatch<Action>;
  /** Changes when another document is opened: the chat starts over. */
  docKey: number;
};

const STATUS_LABEL = { edited: "Proposed edit 修改建议", clarify: "Question 需要确认", refused: "Not possible 无法完成" } as const;

const DECISION_LABEL = {
  accepted: "✓ Accepted 已接受",
  rejected: "Rejected 已拒绝",
  superseded: "Not applied 未采用 (you moved on)",
} as const;

const selectedSceneId = (t: VideoTimeline, s: Selection): string | null => {
  const index = s.kind === "scene" ? s.index : s.kind === "keyframe" || s.kind === "layer" ? s.sceneIndex : -1;
  return t.scenes[index]?.id ?? null;
};

/**
 * Natural-language editing as a chat. Claude sees the current timeline and the recent
 * exchanges, so follow-ups work. Nothing changes until Accept; Accept is one undo step.
 */
export const AssistantPanel = ({ timeline, selection, dispatch, docKey }: Props) => {
  const [instruction, setInstruction] = useState("");
  const [useScene, setUseScene] = useState(true);
  const [model, setModel] = useState<AssistantModel>("claude-opus-5-5");
  const [chat, chatDispatch] = useReducer(chatReducer, docKey, initialChat);
  const logEnd = useRef<HTMLDivElement>(null);
  const sceneId = selectedSceneId(timeline, selection);
  const current = serialize(timeline);
  const busy = isBusy(chat);
  const conversation = buildConversation(chat.exchanges);

  useEffect(() => {
    chatDispatch({ type: "openDocument", doc: docKey });
  }, [docKey]);
  // Braces matter: newer browsers return a Promise from scrollIntoView, and React must get no value back.
  useEffect(() => {
    logEnd.current?.scrollIntoView?.({ block: "nearest" });
  }, [chat.exchanges]);

  const send = async () => {
    const text = instruction.trim();
    if (!text || busy) return;
    const id = nextExchangeId(chat);
    const generation = chat.generation;
    const scene = useScene ? sceneId : null;
    chatDispatch({ type: "send", instruction: text, sceneId: scene, base: current });
    setInstruction("");
    const result = await proposeEdit({
      timelineJson: current,
      instruction: text,
      sceneId: scene,
      model,
      ...(conversation.turns.length ? { conversation } : {}),
    });
    chatDispatch({ type: "answer", id, generation, result });
  };

  const accept = (e: Exchange, p: EditProposal) => {
    if (e.base !== current) return;
    dispatch({ type: "applyAssistant", timeline: p.outcome.timeline });
    chatDispatch({ type: "accept", id: e.id });
  };

  const last = chat.exchanges[chat.exchanges.length - 1];
  const asked = last?.result?.ok && last.result.status === "clarify";

  return (
    <div className="form assistant">
      <div className="assistant-row">
        <div className="section-title assistant-title">Chat with Claude 助手</div>
        <button
          className="btn"
          onClick={() => chatDispatch({ type: "clear" })}
          disabled={busy || chat.exchanges.length === 0}
          title="Forget this conversation and start over"
        >
          新对话 New chat
        </button>
      </div>

      <div className="chat-log" aria-live="polite">
        {chat.exchanges.length === 0 && (
          <div className="dim small">
            Ask for a change, then follow up: “再快一点”, “第 3 幕也这样”, “刚才那个不要了”. Claude remembers this chat
            (not saved; cleared when you open another file).
          </div>
        )}
        {conversation.dropped > 0 && (
          <div className="dim small chat-note">
            {conversation.dropped} older message{conversation.dropped > 1 ? "s are" : " is"} no longer sent to Claude ·
            较早的 {conversation.dropped} 条对话不再发送
          </div>
        )}
        {chat.exchanges.map((e) => (
          <ExchangeView key={e.id} exchange={e} current={current} onAccept={accept} onReject={(id) => chatDispatch({ type: "reject", id })} />
        ))}
        <div ref={logEnd} />
      </div>

      <textarea
        rows={3}
        value={instruction}
        placeholder={asked ? "Answer the question above · 回答上面的问题" : "e.g. 第二段节奏快一点，加一个淡入转场 · make scene 3 slower · switch to vertical"}
        onChange={(e) => setInstruction(e.target.value)}
        onKeyDown={(e) => {
          if (e.key === "Enter" && (e.metaKey || e.ctrlKey)) {
            e.preventDefault();
            void send();
          }
        }}
        aria-label="Message to the assistant"
      />
      <label className="check">
        <input type="checkbox" checked={useScene && !!sceneId} disabled={!sceneId} onChange={(e) => setUseScene(e.target.checked)} />
        {sceneId ? <>About the selected scene <span className="mono">{sceneId}</span></> : "No scene selected"}
      </label>
      <div className="assistant-row">
        <select value={model} onChange={(e) => setModel(e.target.value as AssistantModel)} aria-label="Model">
          {ASSISTANT_MODELS.map((m) => <option key={m.id} value={m.id}>{m.label}</option>)}
        </select>
        <button className="btn primary" onClick={() => void send()} disabled={!instruction.trim() || busy}>
          {busy ? "Thinking… 思考中" : "Send 发送"}
        </button>
      </div>
      <div className="dim small">Each message is a paid API call. Nothing changes until you accept. ⌘↵ sends.</div>
    </div>
  );
};

type ExchangeProps = {
  exchange: Exchange;
  current: string;
  onAccept: (e: Exchange, p: EditProposal) => void;
  onReject: (id: number) => void;
};

const ExchangeView = ({ exchange: e, current, onAccept, onReject }: ExchangeProps) => {
  const failure = failureOf(e);
  const open = openProposal(e);
  const stale = open !== null && e.base !== current;
  return (
    <>
      <div className="chat-user">
        <div>{e.instruction}</div>
        {e.sceneId && <div className="dim small mono">{e.sceneId}</div>}
      </div>
      {e.result === null && <div className="assistant-card chat-wait dim">Thinking… 思考中</div>}
      {failure && (
        <div className="assistant-card bad" role="alert">
          <div className="assistant-status">Assistant unavailable 助手不可用</div>
          <div>{failure.error.message}</div>
          {failureHint(failure) && <div className="dim">{failureHint(failure)}</div>}
          {failure.usage && failure.usage.cost_usd > 0 && <div className="dim mono small">{formatCost(failure.usage)}</div>}
        </div>
      )}
      {e.result?.ok && (
        <div className={`assistant-card ${e.result.status} ${e.decision}`} data-decision={e.result.status === "edited" ? e.decision : undefined}>
          <div className="assistant-status">{STATUS_LABEL[e.result.status]}</div>
          <div>{e.result.explanation}</div>
          {e.result.outcome.summary.length > 0 && (
            <ul className="assistant-diff">
              {e.result.outcome.summary.map((line) => <li key={line} className="mono">{line}</li>)}
            </ul>
          )}
          {e.result.outcome.warnings.map((w) => <div key={w} className="assistant-warning">⚠ {w}</div>)}
          <div className="dim mono small">
            {formatCost(e.result.usage)}
            {e.result.retries > 0 ? ` · ${e.result.retries} correction${e.result.retries > 1 ? "s" : ""}` : ""}
          </div>
          {stale && <div className="assistant-warning">The timeline changed since this was proposed. Ask again. 时间线已变，请重新提问。</div>}
          {open && (
            <div className="assistant-row">
              <button className="btn primary" onClick={() => onAccept(e, open)} disabled={stale}>Accept 接受</button>
              <button className="btn" onClick={() => onReject(e.id)}>Reject 拒绝</button>
            </div>
          )}
          {e.result.status === "edited" && e.decision !== "pending" && (
            <div className={`chat-decision ${e.decision}`}>{DECISION_LABEL[e.decision]}</div>
          )}
        </div>
      )}
    </>
  );
};
